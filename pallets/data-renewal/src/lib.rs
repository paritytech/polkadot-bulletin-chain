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

//! Data renewal pallet for the Bulletin chain.
//!
//! Sits on top of [`pallet_bulletin_transaction_storage`]. Owns the dispatchables
//! `renew`, `force_renew`, `enable_auto_renew`, `disable_auto_renew`, the per-block
//! drain inherent for queued auto-renewals, and the in-memory renew mechanics
//! (`do_renew_in_memory` + `BlockTransactions` mutation).
//!
//! Storage-pallet state is read/mutated via a
//! `Config: pallet_bulletin_transaction_storage::Config` bound (direct calls, no
//! virtual dispatch). The upward call (storage pallet → renewal pallet, when
//! transactions age out at the `RetentionPeriod` boundary) goes through
//! [`pallet_bulletin_transaction_storage::OnObsoleteTransactions`] — the only trait
//! used in this split. Wired by the runtime as
//! `type OnObsoleteTransactions = DataRenewal;`.
//!
//! The chain-wide `PermanentStorageUsed` counter and per-account `bytes_permanent`
//! accounting stay in the storage pallet (called via
//! [`pallet_bulletin_transaction_storage::Pallet::check_authorization`] with
//! `is_renew = true`).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod extension;
pub mod migrations;
pub mod types;
pub mod weights;

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

	/// Maps content hash to the account that registered it for auto-renewal.
	#[pallet::storage]
	pub type AutoRenewals<T: Config> =
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
		/// Schedule a **one-shot** auto-renewal of previously stored data. The renewal fires
		/// exactly once, when the data reaches its `RetentionPeriod` boundary, and then the
		/// registration is removed. For continuous renewal, use
		/// [`enable_auto_renew`](Self::enable_auto_renew) instead.
		///
		/// `entry` identifies the data either by `(block, index)` or by content hash.
		///
		/// Feeless. Registration cost (one transaction unit) is charged in
		/// `check_signed`; the eventual renewal cycle charges bytes against
		/// `bytes_permanent` and the chain-wide cap.
		///
		/// Rejects with [`Error::AutoRenewalAlreadyEnabled`] if a scheduled renewal already
		/// exists for this content hash.
		///
		/// Emits [`Event::RenewalEnabled`] `{ recurring: false }`.
		///
		/// For synchronous renewal at dispatch time, see [`force_renew`](Self::force_renew).
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
				pallet_bulletin_transaction_storage::Pallet::<T>::resolve_transaction_ref(&entry)?;
			let content_hash = info.content_hash;

			ensure!(
				!AutoRenewals::<T>::contains_key(content_hash),
				Error::<T>::AutoRenewalAlreadyEnabled
			);

			AutoRenewals::<T>::insert(
				content_hash,
				RenewalData { account: who.clone(), recurring: false, paid: true },
			);
			Self::deposit_event(Event::RenewalEnabled { content_hash, who, recurring: false });
			Ok(())
		}

		/// Immediately renew previously stored data, synchronous at dispatch time.
		///
		/// Authorization is required (as with `store`). Charges `info.size` against
		/// `bytes_permanent` (per-account renew cap) and `PermanentStorageUsed`
		/// (chain-wide cap).
		///
		/// Emits [`Event::Renewed`] when successful.
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
				pallet_bulletin_transaction_storage::Pallet::<T>::resolve_transaction_ref(&entry)?;

			pallet_bulletin_transaction_storage::Pallet::<T>::ensure_data_size_ok(
				info.size as usize,
			)
			.map_err(|_| Error::<T>::TooManyTransactions)?;

			let content_hash = info.content_hash;
			let new_index = Self::do_renew(info)?;
			Self::deposit_event(Event::Renewed { index: new_index, content_hash });
			Ok(().into())
		}

		/// Enable automatic renewal for a previously stored piece of data.
		///
		/// Recurring scheduler with pre-paid first cycle. The extension's `check_signed`
		/// charges `bytes_permanent`, `PermanentStorageUsed`, and one tx slot at
		/// registration (same hard-cap accounting as `force_renew` / one-shot `renew`).
		/// The registration is inserted as `RenewalData { recurring: true, paid: true }`.
		/// The first renewal cycle fires at the next `RetentionPeriod` boundary **without**
		/// re-charging — the slot is already paid for; the cycle then flips `paid` to
		/// `false`. From that point on, every subsequent cycle charges the owner's
		/// authorization in [`Self::do_process_auto_renewals`], dropping the registration
		/// with [`Event::AutoRenewalFailed`] if the quota is exhausted at cycle time.
		///
		/// Emits [`Event::RenewalEnabled`] `{ recurring: true }` for the registration;
		/// the first actual renewal is emitted as [`Event::DataAutoRenewed`] at cycle
		/// time.
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
				!AutoRenewals::<T>::contains_key(content_hash),
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

			AutoRenewals::<T>::insert(
				content_hash,
				RenewalData { account: who.clone(), recurring: true, paid: true },
			);
			Self::deposit_event(Event::RenewalEnabled { content_hash, who, recurring: true });
			Ok(())
		}

		/// Disable automatic renewal for a piece of data.
		///
		/// Signed: the caller must be the account that originally enabled the renewal,
		/// and the registration must not be in its prepaid window — see
		/// [`Error::CannotDisablePrepaidAutoRenewal`]. Both registrations from
		/// [`Self::renew`] and [`Self::enable_auto_renew`] start with `paid: true`;
		/// the owner has to wait for the first cycle to consume the prepayment before
		/// they can disable.
		///
		/// Root: bypasses the owner check and the prepaid-window check (governance/cleanup).
		///
		/// Emits [`Event::AutoRenewalDisabled`] when successful.
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
				AutoRenewals::<T>::get(content_hash).ok_or(Error::<T>::AutoRenewalNotEnabled)?;
			match caller {
				AuthorizedCaller::Signed { who, .. } => {
					ensure!(renewal_data.account == who, Error::<T>::NotAutoRenewalOwner);
					ensure!(!renewal_data.paid, Error::<T>::CannotDisablePrepaidAutoRenewal);
				},
				AuthorizedCaller::Root => {},
				AuthorizedCaller::Unsigned => return Err(DispatchError::BadOrigin),
			}

			AutoRenewals::<T>::remove(content_hash);
			Self::deposit_event(Event::AutoRenewalDisabled {
				content_hash,
				who: renewal_data.account,
			});
			Ok(())
		}

		/// Drain [`PendingAutoRenewals`] queued by [`OnObsoleteTransactions::handle_obsolete`].
		///
		/// Emitted by [`Pallet::create_inherent`] when the pending vec is non-empty. Refunds
		/// to the actually-drained count via `PostDispatchInfo`.
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
			Err(InvalidTransaction::Call.into())
		}

		fn pre_dispatch(call: &Self::Call) -> Result<(), TransactionValidityError> {
			if Self::is_inherent(call) {
				return Ok(());
			}
			Err(InvalidTransaction::Call.into())
		}
	}
}

// -----------------------------------------------------------------------------
// Implementations outside the `#[pallet]` module
// -----------------------------------------------------------------------------

impl<T: Config> Pallet<T> {
	/// Single-renewal entry point for [`Pallet::force_renew`].
	///
	/// Wraps [`Self::do_renew_in_memory`] (the centralized renewal mechanics) with a
	/// [`BlockTransactions`] read/write. Auto-renewals do not go through this wrapper
	/// — [`Self::do_process_auto_renewals`] amortizes a single read/write across the
	/// whole drain loop instead.
	///
	/// Hard-cap accounting (per-account `bytes_permanent`, chain-wide
	/// `PermanentStorageUsed`) is enforced by `check_authorization` in the
	/// extension's `pre_dispatch` before this runs.
	pub(crate) fn do_renew(info: TransactionInfo) -> Result<u32, Error<T>> {
		let extrinsic_index =
			<frame_system::Pallet<T>>::extrinsic_index().ok_or(Error::<T>::BadContext)?;
		<BlockTransactions<T>>::try_mutate(|transactions| {
			Self::do_renew_in_memory(transactions, &info, extrinsic_index)
				.ok_or(Error::<T>::TooManyTransactions)
		})
	}

	/// Push a `kind = Renew` entry onto the in-memory accumulator and update
	/// [`TransactionByContentHash`]. Returns `None` at `MaxBlockTransactions`.
	///
	/// Called by:
	/// - [`Self::do_renew`] for the single-renewal manual flow (`force_renew`).
	/// - [`Self::do_process_auto_renewals`] in a loop, amortizing one [`BlockTransactions`]
	///   read/write across all pending entries.
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

	/// Drain [`PendingAutoRenewals`] and return the count drained.
	///
	/// Batches the [`BlockTransactions`] read/write across all `n` renewals by threading
	/// an in-memory accumulator through repeated [`Self::do_renew_in_memory`] calls.
	///
	/// On failure (auth missing/expired, per-account or chain-wide cap exceeded, or
	/// per-block slot cap reached), the registration is removed from `AutoRenewals` and
	/// `AutoRenewalFailed` is emitted. The data is **gone** at that point: the obsolete
	/// `Transactions` entry was already taken by storage pallet's `on_initialize`.
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
					AutoRenewals::<T>::remove(content_hash);
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
						AutoRenewals::<T>::remove(content_hash);
					} else if was_paid {
						// Recurring: consume the prepayment so subsequent cycles
						// charge per-cycle, and unblock `disable_auto_renew` for the
						// owner now that the prepaid renewal has been delivered.
						AutoRenewals::<T>::mutate(content_hash, |entry| {
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
					AutoRenewals::<T>::remove(content_hash);
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

// -----------------------------------------------------------------------------
// OnObsoleteTransactions implementation — the one upward callback in this split
// -----------------------------------------------------------------------------

impl<T: Config> OnObsoleteTransactions<BlockNumberFor<T>> for Pallet<T> {
	/// Invoked by storage pallet's `on_initialize` with the obsolete-block sweep
	/// results. Sums renewed bytes (chain-wide counter decrement is left to storage
	/// pallet's own loop), and queues auto-renewals for any `is_latest` entry that
	/// has a matching `AutoRenewals` registration.
	fn handle_obsolete(_obsolete: BlockNumberFor<T>, items: &[(TransactionInfo, bool)]) {
		// Build the queue in memory, write once.
		let mut pending = PendingAutoRenewals::<T>::get();
		for (tx_info, is_latest) in items.iter() {
			if !is_latest {
				continue;
			}
			let hash = tx_info.content_hash;
			if let Some(renewal_data) = AutoRenewals::<T>::get(hash) {
				// try_push cannot overflow under the on_finalize empty-pending invariant
				// plus `items.len() <= MaxBlockTransactions`.
				let _ = pending.try_push((hash, tx_info.clone(), renewal_data));
			}
		}
		if !pending.is_empty() {
			PendingAutoRenewals::<T>::put(&pending);
		}
	}
}
