// This file is part of Substrate.

// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Data-renewal layer for the Bulletin chain. Sits on top of
//! [`pallet_bulletin_transaction_storage`] via a `Config:
//! pallet_bulletin_transaction_storage::Config` bound (direct calls, no virtual
//! dispatch).
//!
//! ## Surface
//!
//! - **Dispatchables:** `force_renew` (synchronous), `renew` (one-shot scheduler),
//!   `enable_auto_renew` / `disable_auto_renew` (recurring), `process_pending_renewals` (mandatory
//!   drain inherent).
//! - **Storage:** [`Renewals`] (per-content-hash registration) and [`PendingAutoRenewals`]
//!   (per-block scratch queue, drained by the inherent).
//!
//! ## Cross-pallet contract
//!
//! - **Down → storage:** dispatchables and the trait callback read/mutate `Transactions`,
//!   `TransactionByContentHash`, `BlockTransactions`, and `Authorizations` directly through
//!   `pallet_bulletin_transaction_storage`'s public API.
//! - **Up ← storage:** [`OnObsoleteTransactions::handle_obsolete`] is called by the storage
//!   pallet's `on_initialize` when entries age out at the `RetentionPeriod` boundary; entries with
//!   an `Renewals` registration are pushed to [`PendingAutoRenewals`] for the same block's inherent
//!   to drain.
//! - **Per-cycle accounting** (per-account `bytes_permanent` and the chain-wide
//!   `PermanentStorageUsed`) is charged by the storage pallet's `check_authorization` with
//!   `is_renew = true`.
//!
//! ## Prepayment model
//!
//! Both `renew` and `enable_auto_renew` are *feeless* registrations: the
//! transaction-extension's `pre_dispatch` charges one tx slot + `size` bytes
//! up front. The first cycle then fires free (`paid = true` on the inserted
//! [`RenewalData`]), and every subsequent recurring cycle charges per-cycle in
//! [`Pallet::do_process_auto_renewals`].

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod extension;
pub mod migrations;
pub mod types;
pub mod weights;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub use pallet::*;
pub use types::RenewalData;
pub use weights::WeightInfo;

use bulletin_transaction_storage_primitives::ContentHash;
use pallet_bulletin_transaction_storage::{
	AuthorizationScope, AuthorizedCaller, BlockTransactions, OnObsoleteTransactions,
	TransactionByContentHash, TransactionInfo, TransactionKind, TransactionRef,
};
use polkadot_sdk_frame::{deps::*, prelude::*};
use sp_transaction_storage_proof::num_chunks;

#[cfg(feature = "try-runtime")]
const LOG_TARGET: &str = "runtime::data-renewal";

#[polkadot_sdk_frame::pallet]
pub mod pallet {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_bulletin_transaction_storage::Config {
		#[allow(deprecated)]
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// Weight info for renewal dispatchables.
		type WeightInfo: WeightInfo;
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Attempted to call `force_renew` outside of block execution.
		BadContext,
		/// Renewed extrinsic is not found.
		RenewedNotFound,
		/// Block already contains the maximum number of transactions.
		TooManyTransactions,
		/// Auto-renewal is already enabled for this content hash.
		AutoRenewalAlreadyEnabled,
		/// Auto-renewal is not enabled for this content hash.
		AutoRenewalNotEnabled,
		/// Caller is not the owner of the auto-renewal registration.
		NotAutoRenewalOwner,
		/// `disable_auto_renew` rejected: the registration has been prepaid for its next
		/// cycle and cannot be disabled by the owner until the cycle fires and consumes
		/// the prepayment. Root can still disable for governance cleanup.
		CannotDisablePrepaidAutoRenewal,
	}

	const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	/// Maps content hash to the account that registered it for renewal.
	#[pallet::storage]
	pub type Renewals<T: Config> =
		StorageMap<_, Blake2_128Concat, ContentHash, RenewalData<T::AccountId>, OptionQuery>;

	/// Transactions that must be auto-renewed in the current block.
	///
	/// Populated by [`OnObsoleteTransactions::handle_obsolete`] when a block's data is
	/// about to expire. Cleared by the [`Pallet::process_pending_renewals`] mandatory
	/// inherent executed in the same block.
	#[pallet::storage]
	pub type PendingAutoRenewals<T: Config> = StorageValue<
		_,
		BoundedVec<
			(ContentHash, TransactionInfo, RenewalData<T::AccountId>),
			<T as pallet_bulletin_transaction_storage::Config>::MaxBlockTransactions,
		>,
		ValueQuery,
	>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Renewed data under specified index.
		Renewed { index: u32, content_hash: ContentHash },
		/// A renewal was enabled for `content_hash` by `who`.
		RenewalEnabled { content_hash: ContentHash, who: T::AccountId, recurring: bool },
		/// Auto-renewal disabled for `content_hash`. `who` is the registration's owner
		/// (not the caller when Root issued the disable).
		AutoRenewalDisabled { content_hash: ContentHash, who: T::AccountId },
		/// Data was automatically renewed at `index` with `content_hash` for `account`.
		DataAutoRenewed { index: u32, content_hash: ContentHash, account: T::AccountId },
		/// Auto-renewal failed for `content_hash` (insufficient authorization for `account`).
		AutoRenewalFailed { content_hash: ContentHash, account: T::AccountId },
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_finalize(_: BlockNumberFor<T>) {
			// All pending auto-renewals must have been processed by the
			// `process_pending_renewals` inherent.
			#[cfg(feature = "try-runtime")]
			if !PendingAutoRenewals::<T>::get().is_empty() {
				tracing::warn!(
					target: LOG_TARGET,
					"Pending auto-renewals were not processed (expected during try-runtime)"
				);
				PendingAutoRenewals::<T>::kill();
			}

			#[cfg(not(feature = "try-runtime"))]
			assert!(
				PendingAutoRenewals::<T>::get().is_empty(),
				"All pending auto-renewals must be processed by process_pending_renewals"
			);
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Schedule a one-shot auto-renewal. Fires once at the
		/// `RetentionPeriod` boundary, then the registration is removed.
		/// Prepaid at registration; see [`force_renew`](Self::force_renew) for
		/// synchronous renewal or [`enable_auto_renew`](Self::enable_auto_renew)
		/// for recurring.
		#[pallet::call_index(0)]
		#[pallet::weight(<T as Config>::WeightInfo::renew())]
		#[pallet::feeless_if(|_origin: &OriginFor<T>, _entry: &TransactionRef<BlockNumberFor<T>>| -> bool { true })]
		pub fn renew(
			origin: OriginFor<T>,
			entry: TransactionRef<BlockNumberFor<T>>,
		) -> DispatchResult {
			let AuthorizedCaller::Signed { who, scope: _ } =
				pallet_bulletin_transaction_storage::Pallet::<T>::ensure_authorized(origin)?
			else {
				return Err(DispatchError::BadOrigin);
			};
			let info =
				pallet_bulletin_transaction_storage::Pallet::<T>::resolve_transaction_ref(&entry)
					.map_err(|_| Error::<T>::RenewedNotFound)?;
			let content_hash = info.content_hash;

			ensure!(
				!Renewals::<T>::contains_key(content_hash),
				Error::<T>::AutoRenewalAlreadyEnabled
			);

			Renewals::<T>::insert(
				content_hash,
				RenewalData { account: who.clone(), recurring: false, paid: true },
			);
			Self::deposit_event(Event::RenewalEnabled { content_hash, who, recurring: false });
			Ok(())
		}

		/// Renew previously stored data synchronously. Charges `info.size` against
		/// the caller's `bytes_permanent` and the chain-wide `PermanentStorageUsed`.
		#[pallet::call_index(1)]
		#[pallet::weight((<T as Config>::WeightInfo::force_renew(), DispatchClass::Operational))]
		#[pallet::feeless_if(|_origin: &OriginFor<T>, _entry: &TransactionRef<BlockNumberFor<T>>| -> bool { true })]
		pub fn force_renew(
			origin: OriginFor<T>,
			entry: TransactionRef<BlockNumberFor<T>>,
		) -> DispatchResultWithPostInfo {
			let _caller =
				pallet_bulletin_transaction_storage::Pallet::<T>::ensure_authorized(origin)?;
			let info =
				pallet_bulletin_transaction_storage::Pallet::<T>::resolve_transaction_ref(&entry)
					.map_err(|_| Error::<T>::RenewedNotFound)?;

			pallet_bulletin_transaction_storage::Pallet::<T>::ensure_data_size_ok(
				info.size as usize,
			)
			.map_err(|_| Error::<T>::TooManyTransactions)?;

			let content_hash = info.content_hash;
			let new_index = Self::do_renew(info)?;
			Self::deposit_event(Event::Renewed { index: new_index, content_hash });
			Ok(().into())
		}

		/// Register recurring auto-renewal for `content_hash`. First cycle is
		/// prepaid at registration (`paid = true`); subsequent cycles charge
		/// the owner's authorization in [`Self::do_process_auto_renewals`] and
		/// drop the registration on quota exhaustion with
		/// [`Event::AutoRenewalFailed`].
		#[pallet::call_index(2)]
		#[pallet::weight(<T as Config>::WeightInfo::enable_auto_renew())]
		#[pallet::feeless_if(|_origin: &OriginFor<T>, _content_hash: &ContentHash| -> bool { true })]
		pub fn enable_auto_renew(
			origin: OriginFor<T>,
			content_hash: ContentHash,
		) -> DispatchResult {
			let AuthorizedCaller::Signed { who, scope: _ } =
				pallet_bulletin_transaction_storage::Pallet::<T>::ensure_authorized(origin)?
			else {
				return Err(DispatchError::BadOrigin);
			};

			ensure!(
				!Renewals::<T>::contains_key(content_hash),
				Error::<T>::AutoRenewalAlreadyEnabled
			);

			// Defensive content-hash existence check. The hard-cap accounting
			// (`bytes_permanent`, `PermanentStorageUsed`, one tx slot) is performed by
			// the extension's `check_signed` for this call with `is_renew = true`,
			// matching the one-shot `renew`. Registering here must not call
			// `do_renew`, otherwise `bytes_permanent` would be double-charged.
			let (block, index) =
				pallet_bulletin_transaction_storage::Pallet::<T>::lookup_by_content_hash(
					content_hash,
				)
				.ok_or(Error::<T>::RenewedNotFound)?;
			pallet_bulletin_transaction_storage::Pallet::<T>::transaction_info(block, index)
				.ok_or(Error::<T>::RenewedNotFound)?;

			Renewals::<T>::insert(
				content_hash,
				RenewalData { account: who.clone(), recurring: true, paid: true },
			);
			Self::deposit_event(Event::RenewalEnabled { content_hash, who, recurring: true });
			Ok(())
		}

		/// Disable auto-renewal. Signed callers must own the registration AND
		/// wait for the prepaid first cycle to have fired (else
		/// [`Error::CannotDisablePrepaidAutoRenewal`]). Root bypasses both
		/// checks.
		#[pallet::call_index(3)]
		#[pallet::weight(<T as Config>::WeightInfo::disable_auto_renew())]
		#[pallet::feeless_if(|_origin: &OriginFor<T>, _content_hash: &ContentHash| -> bool { true })]
		pub fn disable_auto_renew(
			origin: OriginFor<T>,
			content_hash: ContentHash,
		) -> DispatchResult {
			let caller =
				pallet_bulletin_transaction_storage::Pallet::<T>::ensure_authorized(origin)?;
			let renewal_data =
				Renewals::<T>::get(content_hash).ok_or(Error::<T>::AutoRenewalNotEnabled)?;
			match caller {
				AuthorizedCaller::Signed { who, .. } => {
					ensure!(renewal_data.account == who, Error::<T>::NotAutoRenewalOwner);
					ensure!(!renewal_data.paid, Error::<T>::CannotDisablePrepaidAutoRenewal);
				},
				AuthorizedCaller::Root => {},
				AuthorizedCaller::Unsigned => return Err(DispatchError::BadOrigin),
			}

			Renewals::<T>::remove(content_hash);
			Self::deposit_event(Event::AutoRenewalDisabled {
				content_hash,
				who: renewal_data.account,
			});
			Ok(())
		}

		/// Mandatory inherent: drain [`PendingAutoRenewals`] for the current
		/// block. Refunds to the actually-drained count via `PostDispatchInfo`.
		#[pallet::call_index(4)]
		#[pallet::weight((
			<T as Config>::WeightInfo::process_pending_renewals(
				<T as pallet_bulletin_transaction_storage::Config>::MaxBlockTransactions::get(),
			),
			DispatchClass::Mandatory,
		))]
		pub fn process_pending_renewals(origin: OriginFor<T>) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			let n_actual = Self::do_process_auto_renewals();
			Ok(Some(<T as Config>::WeightInfo::process_pending_renewals(n_actual)).into())
		}
	}

	#[pallet::inherent]
	impl<T: Config> ProvideInherent for Pallet<T> {
		type Call = Call<T>;
		type Error = sp_transaction_storage_proof::InherentError;
		const INHERENT_IDENTIFIER: InherentIdentifier = *b"datarenw";

		fn create_inherent(_data: &InherentData) -> Option<Self::Call> {
			if PendingAutoRenewals::<T>::get().is_empty() {
				return None;
			}
			Some(Call::process_pending_renewals {})
		}

		fn check_inherent(_call: &Self::Call, _data: &InherentData) -> Result<(), Self::Error> {
			Ok(())
		}

		fn is_inherent(call: &Self::Call) -> bool {
			matches!(call, Call::process_pending_renewals { .. })
		}
	}

	#[allow(deprecated)]
	#[pallet::validate_unsigned]
	impl<T: Config> ValidateUnsigned for Pallet<T> {
		type Call = Call<T>;

		fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
			if Self::is_inherent(call) {
				return Ok(ValidTransaction::default());
			}
			// Unsigned `force_renew` is admitted only when backed by a preimage
			// authorization (checked, not consumed, here).
			if let Call::force_renew { entry } = call {
				return pallet_bulletin_transaction_storage::Pallet::<T>::check_renew_unsigned(
					entry,
					pallet_bulletin_transaction_storage::CheckContext::Validate,
				)?
				.ok_or_else(|| InvalidTransaction::Call.into());
			}
			Err(InvalidTransaction::Call.into())
		}

		fn pre_dispatch(call: &Self::Call) -> Result<(), TransactionValidityError> {
			if Self::is_inherent(call) {
				return Ok(());
			}
			// Consume the preimage authorization so the dispatch runs against
			// post-consumption state (mirrors the signed extension's `prepare`).
			if let Call::force_renew { entry } = call {
				pallet_bulletin_transaction_storage::Pallet::<T>::check_renew_unsigned(
					entry,
					pallet_bulletin_transaction_storage::CheckContext::PreDispatch,
				)?;
				return Ok(());
			}
			Err(InvalidTransaction::Call.into())
		}
	}
}

impl<T: Config> Pallet<T> {
	/// Single-renewal wrapper for [`Pallet::force_renew`]. Amortizes one
	/// `BlockTransactions` read/write. Hard-cap accounting runs earlier in the
	/// extension's `pre_dispatch`.
	pub(crate) fn do_renew(info: TransactionInfo) -> Result<u32, Error<T>> {
		let extrinsic_index =
			<frame_system::Pallet<T>>::extrinsic_index().ok_or(Error::<T>::BadContext)?;
		<BlockTransactions<T>>::try_mutate(|transactions| {
			Self::do_renew_in_memory(transactions, &info, extrinsic_index)
				.ok_or(Error::<T>::TooManyTransactions)
		})
	}

	/// Push a `kind = Renew` entry onto the in-memory `BlockTransactions`
	/// accumulator, host-index the renewal, and update
	/// [`TransactionByContentHash`]. Returns `None` at
	/// `MaxBlockTransactions`. Used by both the manual flow ([`Self::do_renew`])
	/// and the batched drain ([`Self::do_process_auto_renewals`]).
	pub(crate) fn do_renew_in_memory(
		transactions: &mut BoundedVec<
			TransactionInfo,
			<T as pallet_bulletin_transaction_storage::Config>::MaxBlockTransactions,
		>,
		info: &TransactionInfo,
		extrinsic_index: u32,
	) -> Option<u32> {
		let block_chunks =
			TransactionInfo::total_chunks(transactions).saturating_add(num_chunks(info.size));
		let new_index = transactions.len() as u32;
		let new_info = TransactionInfo {
			chunk_root: info.chunk_root,
			size: info.size,
			content_hash: info.content_hash,
			hashing: info.hashing,
			cid_codec: info.cid_codec,
			extrinsic_index,
			block_chunks,
			kind: TransactionKind::Renew,
		};
		transactions.try_push(new_info).ok()?;
		sp_io::transaction_index::renew(extrinsic_index, info.content_hash);
		TransactionByContentHash::<T>::insert(
			info.content_hash,
			(pallet_bulletin_transaction_storage::Pallet::<T>::now(), new_index),
		);
		Some(new_index)
	}

	/// Drain [`PendingAutoRenewals`], returning the count drained. Threads one
	/// `BlockTransactions::mutate` across all entries. Per-cycle charges
	/// (recurring cycles past the prepaid one) go through `check_authorization`;
	/// the prepaid bump is refunded when a paid cycle is rejected by the
	/// per-block slot cap.
	///
	/// On any failure (auth, caps, slot cap) the registration is removed and
	/// `AutoRenewalFailed` emitted — the data is gone, since the obsolete
	/// `Transactions` entry was already taken by storage pallet's
	/// `on_initialize`.
	pub(crate) fn do_process_auto_renewals() -> u32 {
		let pending = PendingAutoRenewals::<T>::take();
		let n_actual = pending.len() as u32;
		if n_actual == 0 {
			return 0;
		}

		let extrinsic_index = match <frame_system::Pallet<T>>::extrinsic_index() {
			Some(idx) => idx,
			// Defensive: no extrinsic context means we can't index renewals; fail all
			// rather than silently skip.
			None => {
				for (content_hash, _, renewal_data) in pending.into_iter() {
					Renewals::<T>::remove(content_hash);
					Self::deposit_event(Event::AutoRenewalFailed {
						content_hash,
						account: renewal_data.account,
					});
				}
				return n_actual;
			},
		};
		<BlockTransactions<T>>::mutate(|transactions| {
			for (content_hash, tx_info, renewal_data) in pending.into_iter() {
				// `paid = true` means the cycle was already charged at registration
				// (the one-shot `renew` path and the first cycle after
				// `enable_auto_renew`). All other recurring cycles charge here.
				let was_paid = renewal_data.paid;
				let scope = AuthorizationScope::Account(renewal_data.account.clone());
				let charged = was_paid ||
					pallet_bulletin_transaction_storage::Pallet::<T>::check_authorization(
						&scope,
						tx_info.size,
						true,
						true,
					)
					.is_ok();
				let new_index = if charged {
					Self::do_renew_in_memory(transactions, &tx_info, extrinsic_index)
				} else {
					None
				};

				if let Some(new_index) = new_index {
					if !renewal_data.recurring {
						// One-shot: registration is consumed.
						Renewals::<T>::remove(content_hash);
					} else if was_paid {
						// Recurring: consume the prepayment so subsequent cycles
						// charge per-cycle, and unblock `disable_auto_renew` for the
						// owner now that the prepaid renewal has been delivered.
						Renewals::<T>::mutate(content_hash, |entry| {
							if let Some(data) = entry {
								data.paid = false;
							}
						});
					}
					Self::deposit_event(Event::DataAutoRenewed {
						index: new_index,
						content_hash,
						account: renewal_data.account,
					});
				} else {
					if charged {
						// Reverse the chain-wide `PermanentStorageUsed` bump that
						// `check_authorization` applied for this cycle. The per-account
						// `bytes_permanent` / `transactions` increments are intentionally
						// left burned: slot-cap rejection at inherent time is a chain-level
						// pathological event.
						let size_u64: u64 = tx_info.size.into();
						pallet_bulletin_transaction_storage::Pallet::<T>::update_permanent_storage_used(
							|used| used.saturating_sub(size_u64),
						);
					}
					Renewals::<T>::remove(content_hash);
					Self::deposit_event(Event::AutoRenewalFailed {
						content_hash,
						account: renewal_data.account,
					});
				}
			}
		});
		n_actual
	}
}

/// Upward callback fired by `pallet_bulletin_transaction_storage::on_initialize`
/// with the obsolete-block sweep result. For each `is_latest` entry with a
/// matching [`Renewals`] registration, queue it into [`PendingAutoRenewals`]
/// for the same block's `process_pending_renewals` inherent to drain.
impl<T: Config> OnObsoleteTransactions<BlockNumberFor<T>> for Pallet<T> {
	fn handle_obsolete(_obsolete: BlockNumberFor<T>, items: &[(TransactionInfo, bool)]) {
		// One read, one write — `try_push` cannot overflow under
		// `items.len() <= MaxBlockTransactions` plus the `on_finalize`
		// empty-pending invariant.
		let mut pending = PendingAutoRenewals::<T>::get();
		for (tx_info, is_latest) in items.iter() {
			if !is_latest {
				continue;
			}
			let hash = tx_info.content_hash;
			if let Some(renewal_data) = Renewals::<T>::get(hash) {
				let _ = pending.try_push((hash, tx_info.clone(), renewal_data));
			}
		}
		if !pending.is_empty() {
			PendingAutoRenewals::<T>::put(&pending);
		}
	}
}
