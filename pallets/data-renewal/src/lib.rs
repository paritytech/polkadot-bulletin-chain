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
//! - **Storage:** [`Renewals`] (per-content-hash registration), [`PendingRenewals`] (per-block
//!   scratch queue, drained by the inherent), and [`PermanentStorageUsed`] (chain-wide renewed-byte
//!   counter, capped by `MaxPermanentStorageSize`).
//!
//! ## Cross-pallet contract
//!
//! The storage pallet has no renewal vocabulary; this pallet owns all of it through
//! two opaque payloads the runtime wires (an `EntryMeta` implementing [`RenewMeta`],
//! [`PermanentExtent`] as `AuthorizationExtra`):
//!
//! - **Down → storage:** dispatchables use the storage pallet's public API; the per-account renew
//!   quota is mutated atomically through `try_mutate_active_authorization`.
//! - **Up ← storage:** [`OnObsoleteTransactions::handle_obsolete`] fires at the `RetentionPeriod`
//!   boundary — it decrements [`PermanentStorageUsed`] for aged-out `Renew` entries and queues
//!   registered entries into [`PendingRenewals`] for the same block's inherent.
//! - **Per-cycle accounting** is charged by `Pallet::check_renew_authorization`.
//!
//! ## Prepayment model
//!
//! Both `renew` and `enable_auto_renew` are *feeless* registrations: the
//! transaction-extension's `pre_dispatch` charges one tx slot + `size` bytes
//! up front. The first cycle then fires free (`paid = true` on the inserted
//! [`RenewalData`]), and every subsequent recurring cycle charges per-cycle in
//! `Pallet::do_process_pending_renewals`.

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
pub use types::{PermanentExtent, RenewMeta, RenewalData};
pub use weights::WeightInfo;

use bulletin_transaction_storage_primitives::ContentHash;
use pallet_bulletin_transaction_storage::{
	AuthorizationScope, AuthorizationScopeFor, AuthorizedCaller, CheckContext,
	OnObsoleteTransactions, TransactionInfoFor, TransactionRef, ValidTransactionParams,
	BAD_DATA_SIZE,
};
use polkadot_sdk_frame::{deps::*, prelude::*};

#[cfg(feature = "try-runtime")]
const LOG_TARGET: &str = "runtime::data-renewal";

// `InvalidTransaction::Custom` codes owned by this pallet. They share one `u8`
// namespace with `pallet-bulletin-transaction-storage`'s codes (which reserves these
// values) and are matched on by clients — keep them wire-stable.
/// Renewed extrinsic not found.
pub const RENEWED_NOT_FOUND: InvalidTransaction = InvalidTransaction::Custom(2);
/// Renew rejected: would push the signer's `bytes_permanent` past their `bytes_allowance`
/// (per-account hard cap).
pub const PERMANENT_ALLOWANCE_EXCEEDED: InvalidTransaction = InvalidTransaction::Custom(5);
/// Renew rejected: would push `PermanentStorageUsed` past `MaxPermanentStorageSize`
/// (chain-wide hard cap).
pub const CHAIN_PERMANENT_CAP_REACHED: InvalidTransaction = InvalidTransaction::Custom(6);
/// `disable_auto_renew`: no auto-renewal is registered for the given content hash.
pub const AUTO_RENEWAL_NOT_ENABLED: InvalidTransaction = InvalidTransaction::Custom(9);
/// `disable_auto_renew`: caller is not the account that registered the auto-renewal.
pub const NOT_AUTO_RENEWAL_OWNER: InvalidTransaction = InvalidTransaction::Custom(10);
/// `renew` / `enable_auto_renew`: a renewal is already registered for this content hash.
pub const RENEWAL_ALREADY_ENABLED: InvalidTransaction = InvalidTransaction::Custom(11);
/// `disable_auto_renew`: the registration has been prepaid for its next cycle and
/// cannot be disabled by the owner until the cycle fires and consumes the prepayment.
/// Root can still disable for governance cleanup.
pub const CANNOT_DISABLE_PREPAID_AUTO_RENEWAL: InvalidTransaction = InvalidTransaction::Custom(12);

/// Percent of `MaxPermanentStorageSize` at which the pallet emits
/// [`Event::PermanentStorageNearCap`] (rising-edge only). Off-chain governance consumers
/// can use this as a "raise the cap or coordinate another bulletin chain" trigger.
pub const PERMANENT_STORAGE_NEAR_CAP_PERCENT: u64 = 80;

#[polkadot_sdk_frame::pallet]
pub mod pallet {
	use super::*;

	#[pallet::config]
	pub trait Config:
		frame_system::Config
		+ pallet_bulletin_transaction_storage::Config<
			EntryMeta: RenewMeta,
			AuthorizationExtra = PermanentExtent,
		>
	{
		#[allow(deprecated)]
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// Weight info for renewal dispatchables.
		type WeightInfo: WeightInfo;
		/// Cap, in bytes, on total permanent storage (via `renew`) committed across
		/// all authorizations.
		#[pallet::constant]
		type MaxPermanentStorageSize: Get<u64>;
		/// Pool params for every renewal call. One prefix, so at most one of `renew`,
		/// `force_renew` and `enable_auto_renew` per account and content hash is queued at
		/// a time. Preimage `force_renew` tags on the content hash alone, so it dedups
		/// separately.
		#[pallet::constant]
		type RenewTxParams: Get<ValidTransactionParams>;
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Attempted to call `force_renew` outside of block execution.
		BadContext,
		/// Renewed extrinsic is not found.
		RenewedNotFound,
		/// Block already contains the maximum number of transactions.
		TooManyTransactions,
		/// A renewal is already registered for this content hash.
		RenewalAlreadyEnabled,
		/// Auto-renewal is not enabled for this content hash.
		AutoRenewalNotEnabled,
		/// Caller is not the owner of the auto-renewal registration.
		NotAutoRenewalOwner,
		/// `disable_auto_renew` rejected: the registration has been prepaid for its next
		/// cycle and cannot be disabled by the owner until the cycle fires and consumes
		/// the prepayment. Root can still disable for governance cleanup.
		CannotDisablePrepaidAutoRenewal,
		/// Data size of the renewed entry is not in the allowed range. Appended last: the
		/// earlier indices are wire-visible.
		BadDataSize,
	}

	const STORAGE_VERSION: StorageVersion = StorageVersion::new(2);

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	/// Renewal registrations by content hash.
	#[pallet::storage]
	pub type Renewals<T: Config> =
		StorageMap<_, Blake2_128Concat, ContentHash, RenewalData<T::AccountId>, OptionQuery>;

	/// Transactions to renew in the current block.
	///
	/// Filled by [`OnObsoleteTransactions::handle_obsolete`] at the retention boundary,
	/// drained by the [`Pallet::process_pending_renewals`] inherent in the same block.
	#[pallet::storage]
	pub type PendingRenewals<T: Config> = StorageValue<
		_,
		BoundedVec<
			(ContentHash, TransactionInfoFor<T>, RenewalData<T::AccountId>),
			T::MaxBlockTransactions,
		>,
		ValueQuery,
	>;

	/// Chain-wide total of currently-on-chain renewed bytes. Source of truth for the
	/// chain-wide hard cap: a `renew` of `size` bytes is rejected when
	/// `PermanentStorageUsed + size > MaxPermanentStorageSize`.
	///
	/// Bumped on each successful renew consume. Decremented by
	/// [`OnObsoleteTransactions::handle_obsolete`] when an obsolete
	/// `Transactions[block]` is removed: each `meta == Renew` entry contributes its
	/// `size` to the decrement.
	#[pallet::storage]
	pub type PermanentStorageUsed<T: Config> = StorageValue<_, u64, ValueQuery>;

	/// Live references to `content_hash`'s renewed bytes: one per refcounted `Renew` entry,
	/// plus one per outstanding prepaid registration — charged before its entry exists, and
	/// converting into exactly one.
	///
	/// [`PermanentStorageUsed`] moves only on the 0↔1 edge, keeping it a proxy for bytes on
	/// disk rather than for references to them.
	#[pallet::storage]
	pub type RenewRefCount<T: Config> =
		StorageMap<_, Blake2_128Concat, ContentHash, u32, OptionQuery>;

	/// First block whose aged-out `Renew` entries are credited through [`RenewRefCount`].
	/// Earlier entries were charged one by one, so they are credited one by one.
	///
	/// Splitting per aged-out block rather than per entry is what lets the two populations
	/// coexist without marking entries or scanning them at upgrade — `handle_obsolete`
	/// already knows which block it is sweeping. Set by [`migrations::v2::MigrateV1ToV2`];
	/// `None` reads as "refcount everything". Inert one `RetentionPeriod` after the upgrade,
	/// and removable then.
	#[pallet::storage]
	pub type RefcountFrom<T: Config> = StorageValue<_, BlockNumberFor<T>, OptionQuery>;

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
		/// A registered renewal fired, re-storing the data at `index`.
		DataRenewed { index: u32, content_hash: ContentHash, account: T::AccountId },
		/// A registered renewal failed on `account`'s authorization; the registration is
		/// dropped and the data expires.
		RenewalFailed { content_hash: ContentHash, account: T::AccountId },
		/// `PermanentStorageUsed` changed (a `renew` bumped it, or the obsolete sweep
		/// decremented it). Off-chain capacity-planning consumers can drive their dashboards
		/// from these.
		PermanentStorageUsedUpdated { used: u64 },
		/// `PermanentStorageUsed` just crossed the [`PERMANENT_STORAGE_NEAR_CAP_PERCENT`]
		/// threshold of `MaxPermanentStorageSize` on the rising edge. Emitted once per
		/// crossing — no re-emission while still above the threshold.
		PermanentStorageNearCap { used: u64, cap: u64 },
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_finalize(_: BlockNumberFor<T>) {
			// All pending renewals must have been processed by the
			// `process_pending_renewals` inherent.
			#[cfg(feature = "try-runtime")]
			if !PendingRenewals::<T>::get().is_empty() {
				tracing::warn!(
					target: LOG_TARGET,
					"Pending renewals were not processed (expected during try-runtime)"
				);
				PendingRenewals::<T>::kill();
			}

			#[cfg(not(feature = "try-runtime"))]
			assert!(
				PendingRenewals::<T>::get().is_empty(),
				"All pending renewals must be processed by process_pending_renewals"
			);
		}

		#[cfg(feature = "try-runtime")]
		fn try_state(n: BlockNumberFor<T>) -> Result<(), sp_runtime::TryRuntimeError> {
			Pallet::<T>::do_try_state(n)
		}

		/// Renewals tag like the storage pallet's families — signed `renew` like signed
		/// `store`, preimage `force_renew` like preimage `store` — so they must not share a
		/// prefix with them.
		fn integrity_test() {
			pallet_bulletin_transaction_storage::Pallet::<T>::assert_pool_families_distinct(&[(
				"RenewTxParams",
				<T as Config>::RenewTxParams::get(),
			)]);
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

			ensure!(!Renewals::<T>::contains_key(content_hash), Error::<T>::RenewalAlreadyEnabled);

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
			.map_err(|_| Error::<T>::BadDataSize)?;

			let content_hash = info.content_hash;
			let new_index = Self::do_renew(info)?;
			Self::deposit_event(Event::Renewed { index: new_index, content_hash });
			Ok(().into())
		}

		/// Register recurring auto-renewal for `content_hash`. First cycle is
		/// prepaid at registration (`paid = true`); subsequent cycles charge
		/// the owner's authorization in `do_process_pending_renewals` and
		/// drop the registration on quota exhaustion with
		/// [`Event::RenewalFailed`].
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

			ensure!(!Renewals::<T>::contains_key(content_hash), Error::<T>::RenewalAlreadyEnabled);

			// Defensive content-hash existence check. The hard-cap accounting
			// (`bytes_permanent`, `PermanentStorageUsed`, one tx slot) is performed by
			// the extension via `check_renew_authorization`, matching the one-shot
			// `renew`. Registering here must not call `do_renew`, otherwise
			// `bytes_permanent` would be double-charged.
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

		/// Mandatory inherent: drain [`PendingRenewals`] for the current
		/// block. Refunds to the actually-drained count via `PostDispatchInfo`.
		#[pallet::call_index(4)]
		#[pallet::weight((
			<T as Config>::WeightInfo::process_pending_renewals(
				T::MaxBlockTransactions::get(),
			),
			DispatchClass::Mandatory,
		))]
		pub fn process_pending_renewals(origin: OriginFor<T>) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			let n_actual = Self::do_process_pending_renewals();
			Ok(Some(<T as Config>::WeightInfo::process_pending_renewals(n_actual)).into())
		}
	}

	#[pallet::inherent]
	impl<T: Config> ProvideInherent for Pallet<T> {
		type Call = Call<T>;
		type Error = sp_inherents::MakeFatalError<()>;
		const INHERENT_IDENTIFIER: InherentIdentifier = *b"datarenw";

		fn create_inherent(_data: &InherentData) -> Option<Self::Call> {
			if PendingRenewals::<T>::get().is_empty() {
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
				return Self::check_renew_unsigned(
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
				Self::check_renew_unsigned(
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
	/// Single-renewal wrapper for [`Pallet::force_renew`]. Hard-cap accounting runs
	/// earlier in the extension's `pre_dispatch`.
	pub(crate) fn do_renew(info: TransactionInfoFor<T>) -> Result<u32, Error<T>> {
		let extrinsic_index =
			<frame_system::Pallet<T>>::extrinsic_index().ok_or(Error::<T>::BadContext)?;
		pallet_bulletin_transaction_storage::Pallet::<T>::with_block_transactions(|entries| {
			entries.renew(&info, extrinsic_index, T::EntryMeta::renew())
		})
		.ok_or(Error::<T>::TooManyTransactions)
	}

	/// Drain [`PendingRenewals`], returning the count drained. One
	/// `BlockTransactions` read/write for all entries. Per-cycle charges
	/// (recurring cycles past the prepaid one) go through `check_authorization`;
	/// the prepaid bump is refunded when a paid cycle is rejected by the
	/// per-block slot cap.
	///
	/// On any failure (auth, caps, slot cap) the registration is removed and
	/// `RenewalFailed` emitted — the data is gone, since the obsolete
	/// `Transactions` entry was already taken by storage pallet's
	/// `on_initialize`.
	pub(crate) fn do_process_pending_renewals() -> u32 {
		let pending = PendingRenewals::<T>::take();
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
					Self::deposit_event(Event::RenewalFailed {
						content_hash,
						account: renewal_data.account,
					});
				}
				return n_actual;
			},
		};
		pallet_bulletin_transaction_storage::Pallet::<T>::with_block_transactions(|entries| {
			for (content_hash, tx_info, renewal_data) in pending.into_iter() {
				// `paid = true` means the cycle was already charged at registration
				// (the one-shot `renew` path and the first cycle after
				// `enable_auto_renew`). All other recurring cycles charge here.
				let was_paid = renewal_data.paid;
				let scope = AuthorizationScope::Account(renewal_data.account.clone());
				let charged = was_paid ||
					Self::check_renew_authorization(&scope, content_hash, tx_info.size, true)
						.is_ok();
				let new_index = if charged {
					entries.renew(&tx_info, extrinsic_index, T::EntryMeta::renew())
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
					Self::deposit_event(Event::DataRenewed {
						index: new_index,
						content_hash,
						account: renewal_data.account,
					});
				} else {
					if charged {
						// The reference was taken above, or at registration when prepaid. The
						// per-account `bytes_permanent` / `transactions` increments are
						// intentionally left burned: slot-cap rejection at inherent time is a
						// chain-level pathological event.
						let freed = Self::release_renew_ref(content_hash, tx_info.size.into());
						if freed > 0 {
							Self::update_permanent_storage_used(|used| used.saturating_sub(freed));
						}
					}
					Renewals::<T>::remove(content_hash);
					Self::deposit_event(Event::RenewalFailed {
						content_hash,
						account: renewal_data.account,
					});
				}
			}
		});
		n_actual
	}
}

impl<T: Config> Pallet<T> {
	/// Hard-cap renew check in one atomic `Authorizations` mutate: existence +
	/// expiry, per-account cap ([`PERMANENT_ALLOWANCE_EXCEEDED`]), chain-wide cap
	/// ([`CHAIN_PERMANENT_CAP_REACHED`]). With `consume`, bumps `bytes_permanent`,
	/// one tx slot, and [`PermanentStorageUsed`]; the matching decrement happens in
	/// `handle_obsolete` when the renewed entry ages out.
	pub(crate) fn check_renew_authorization(
		scope: &AuthorizationScopeFor<T>,
		content_hash: ContentHash,
		size: u32,
		consume: bool,
	) -> Result<(), TransactionValidityError> {
		let size_u64: u64 = size.into();
		// Already-referenced content adds nothing to the chain-wide total, so it must not be
		// tested against the chain-wide cap either.
		let chain_delta = if Self::renew_refs(content_hash) > 0 { 0 } else { size_u64 };
		let chain_used = PermanentStorageUsed::<T>::get();
		let chain_cap = T::MaxPermanentStorageSize::get();

		pallet_bulletin_transaction_storage::Pallet::<T>::try_mutate_active_authorization(
			scope,
			consume,
			|authorization| {
				// Per-account hard cap (per-window quota) against the shared allowance.
				let used = authorization.extent().extra.bytes_permanent;
				if used.saturating_add(size_u64) > authorization.extent().bytes_allowance {
					return Err(PERMANENT_ALLOWANCE_EXCEEDED.into());
				}
				// Chain-wide hard cap.
				if chain_used.saturating_add(chain_delta) > chain_cap {
					return Err(CHAIN_PERMANENT_CAP_REACHED.into());
				}
				authorization.extra_mut().bytes_permanent = used.saturating_add(size_u64);
				authorization.note_transaction();
				Ok(())
			},
		)?;

		if consume {
			Self::acquire_renew_ref(content_hash, size_u64);
		}
		Ok(())
	}

	fn renew_refs(content_hash: ContentHash) -> u32 {
		RenewRefCount::<T>::get(content_hash).unwrap_or(0)
	}

	/// Charges `size` only on the 0→1 edge.
	fn acquire_renew_ref(content_hash: ContentHash, size: u64) {
		let refs = Self::renew_refs(content_hash).saturating_add(1);
		RenewRefCount::<T>::insert(content_hash, refs);
		if refs == 1 {
			Self::update_permanent_storage_used(|used| used.saturating_add(size));
		}
	}

	/// Returns the bytes freed: `size` on the 1→0 edge, zero otherwise — callers fold the
	/// total into one counter write.
	///
	/// An absent reference frees nothing. Crediting it would under-count, the direction that
	/// lets renewed bytes past the chain-wide cap.
	fn release_renew_ref(content_hash: ContentHash, size: u64) -> u64 {
		match Self::renew_refs(content_hash) {
			0 => 0,
			1 => {
				RenewRefCount::<T>::remove(content_hash);
				size
			},
			n => {
				RenewRefCount::<T>::insert(content_hash, n.saturating_sub(1));
				0
			},
		}
	}

	/// Update [`PermanentStorageUsed`] via `f`, emitting
	/// [`Event::PermanentStorageUsedUpdated`] and — on the rising edge across the
	/// [`PERMANENT_STORAGE_NEAR_CAP_PERCENT`] threshold —
	/// [`Event::PermanentStorageNearCap`], exactly once per crossing.
	pub(crate) fn update_permanent_storage_used(f: impl FnOnce(u64) -> u64) {
		let old = PermanentStorageUsed::<T>::get();
		let new = f(old);
		PermanentStorageUsed::<T>::put(new);
		Self::deposit_event(Event::PermanentStorageUsedUpdated { used: new });
		let cap = T::MaxPermanentStorageSize::get();
		// Divide-first to avoid u64 overflow on extreme caps (`cap * 80` saturates
		// above ~230 EiB). Loses ≤`pct` bytes of precision; harmless for the rising-edge.
		let threshold = (cap / 100).saturating_mul(PERMANENT_STORAGE_NEAR_CAP_PERCENT);
		if old < threshold && new >= threshold {
			Self::deposit_event(Event::PermanentStorageNearCap { used: new, cap });
		}
	}

	/// Signed-renew authorization with preimage-preference: try a
	/// `Preimage(content_hash)` grant first (lets anyone renew pre-authorized
	/// content without spending their own account quota), falling back to
	/// `Account(who)`. Runs the `data_size_ok` / `block_transactions_full`
	/// guards, then the hard-cap renew check against the chosen scope. Returns
	/// the scope charged so the caller can rewrite the origin; `consume` mutates
	/// the chosen authorization on success.
	pub(crate) fn authorize_renew(
		who: &T::AccountId,
		content_hash: ContentHash,
		size: u32,
		consume: bool,
	) -> Result<AuthorizationScopeFor<T>, TransactionValidityError> {
		if !pallet_bulletin_transaction_storage::Pallet::<T>::data_size_ok(size as usize) {
			return Err(BAD_DATA_SIZE.into());
		}
		if pallet_bulletin_transaction_storage::Pallet::<T>::block_transactions_full() {
			return Err(InvalidTransaction::ExhaustsResources.into());
		}
		if Self::check_renew_authorization(
			&AuthorizationScope::Preimage(content_hash),
			content_hash,
			size,
			consume,
		)
		.is_ok()
		{
			return Ok(AuthorizationScope::Preimage(content_hash));
		}
		Self::check_renew_authorization(
			&AuthorizationScope::Account(who.clone()),
			content_hash,
			size,
			consume,
		)?;
		Ok(AuthorizationScope::Account(who.clone()))
	}

	/// Pool/dispatch validation for an unsigned renew (preimage-only). Resolves
	/// `entry` then checks — and, in [`CheckContext::PreDispatch`],
	/// consumes — a `Preimage(content_hash)` authorization. No account fallback:
	/// unsigned renewals must be backed by a preimage grant. Shares the storage
	/// pallet's preimage tag so unsigned stores and renews of one preimage dedup.
	pub(crate) fn check_renew_unsigned(
		entry: &TransactionRef<BlockNumberFor<T>>,
		context: CheckContext,
	) -> Result<Option<ValidTransaction>, TransactionValidityError> {
		let info = pallet_bulletin_transaction_storage::Pallet::<T>::resolve_transaction_ref(entry)
			.map_err(|_| RENEWED_NOT_FOUND)?;
		if !pallet_bulletin_transaction_storage::Pallet::<T>::data_size_ok(info.size as usize) {
			return Err(BAD_DATA_SIZE.into());
		}
		if pallet_bulletin_transaction_storage::Pallet::<T>::block_transactions_full() {
			return Err(InvalidTransaction::ExhaustsResources.into());
		}
		Self::check_renew_authorization(
			&AuthorizationScope::Preimage(info.content_hash),
			info.content_hash,
			info.size,
			context.consume_authorization(),
		)?;
		Ok(context
			.want_valid_transaction()
			.then(|| <T as Config>::RenewTxParams::get().provides(info.content_hash)))
	}

	/// `true` iff `renew(entry)` would currently pass validation for `who`: `entry`
	/// resolves, size in range, unexpired authorization, and both hard caps clear.
	pub fn can_renew(who: &T::AccountId, entry: &TransactionRef<BlockNumberFor<T>>) -> bool {
		let Ok(info) =
			pallet_bulletin_transaction_storage::Pallet::<T>::resolve_transaction_ref(entry)
		else {
			return false;
		};
		if !pallet_bulletin_transaction_storage::Pallet::<T>::data_size_ok(info.size as usize) {
			return false;
		}
		Self::check_renew_authorization(
			&AuthorizationScope::Account(who.clone()),
			info.content_hash,
			info.size,
			false,
		)
		.is_ok()
	}
}

impl<T: Config> Pallet<T> {
	/// try-state invariants:
	/// - `PermanentStorageUsed` == Σ distinct refcounted content sizes + Σ legacy `Renew` entry
	///   sizes. Prepaid registrations need no term of their own — their charge already holds a
	///   reference.
	/// - `RenewRefCount[hash]` == live refcounted `Renew` entries for `hash`, plus any outstanding
	///   prepaid registration.
	/// - `PermanentStorageUsed <= MaxPermanentStorageSize`.
	#[cfg(any(feature = "try-runtime", test))]
	pub(crate) fn do_try_state(_n: BlockNumberFor<T>) -> Result<(), sp_runtime::TryRuntimeError> {
		use polkadot_sdk_frame::prelude::ensure;

		let used = PermanentStorageUsed::<T>::get();

		ensure!(
			used <= T::MaxPermanentStorageSize::get(),
			"PermanentStorageUsed exceeds MaxPermanentStorageSize",
		);

		// A hash straddling the cutoff contributes on both sides, matching the two
		// independent charges it took.
		let refcount_from = RefcountFrom::<T>::get();
		let legacy_block =
			|block: BlockNumberFor<T>| refcount_from.is_some_and(|from| block < from);
		let mut legacy_sum: u64 = 0;
		// Size comes from any live entry for the hash — same content, same bytes, same size.
		let mut live_refs: alloc::collections::BTreeMap<ContentHash, (u32, u64)> =
			Default::default();
		for (block, entries) in pallet_bulletin_transaction_storage::Transactions::<T>::iter() {
			for t in entries.iter().filter(|t| t.meta.is_renew()) {
				if legacy_block(block) {
					legacy_sum = legacy_sum.saturating_add(t.size as u64);
				} else {
					let slot = live_refs.entry(t.content_hash).or_insert((0, t.size as u64));
					slot.0 = slot.0.saturating_add(1);
				}
			}
		}

		// A prepaid registration holds a reference before its `Renew` entry exists, so it may
		// be the only thing keeping the hash counted.
		for (content_hash, registration) in Renewals::<T>::iter() {
			if !registration.paid {
				continue;
			}
			if let Some(slot) = live_refs.get_mut(&content_hash) {
				slot.0 = slot.0.saturating_add(1);
				continue;
			}
			let (block, index) =
				pallet_bulletin_transaction_storage::Pallet::<T>::lookup_by_content_hash(
					content_hash,
				)
				.ok_or("paid Renewals registration has no on-chain target")?;
			let info =
				pallet_bulletin_transaction_storage::Pallet::<T>::transaction_info(block, index)
					.ok_or("paid Renewals registration target has no TransactionInfo")?;
			live_refs.insert(content_hash, (1, info.size as u64));
		}

		let mut refcounted_sum: u64 = 0;
		for (content_hash, (expected_refs, size)) in live_refs.iter() {
			ensure!(
				RenewRefCount::<T>::get(content_hash).unwrap_or(0) == *expected_refs,
				"RenewRefCount does not match live refcounted Renew entries + prepaid \
				 registrations",
			);
			refcounted_sum = refcounted_sum.saturating_add(*size);
		}

		let expected = legacy_sum.saturating_add(refcounted_sum);

		// `used` gates admission, so under-counting is what lets renewed bytes past the cap;
		// over-counting only rejects early. Live state drifts both ways through history this
		// pallet did not create — pre-counter `Renew` entries were never charged, and Root
		// dropping a prepaid registration never decrements — so the equality is enforced only
		// where state is built from nothing.
		#[cfg(test)]
		ensure!(
			used == expected,
			"PermanentStorageUsed != Σ refcounted content sizes + Σ legacy renewed sizes",
		);
		#[cfg(all(feature = "try-runtime", not(test)))]
		if used != expected {
			tracing::warn!(
				target: LOG_TARGET,
				used,
				expected,
				"PermanentStorageUsed drifts from Σ refcounted content + Σ legacy renewed sizes",
			);
		}

		Ok(())
	}
}

/// Obsolete-block sweep callback: releases the [`RenewRefCount`] references held by aged-out
/// `Renew` entries and queues `is_latest` entries with a [`Renewals`] registration into
/// [`PendingRenewals`] for the same block's inherent.
impl<T: Config> OnObsoleteTransactions<BlockNumberFor<T>, T::EntryMeta> for Pallet<T> {
	fn handle_obsolete(obsolete: BlockNumberFor<T>, items: &[(TransactionInfoFor<T>, bool)]) {
		// Renewed bytes free up capacity only once the *last* reference to that content goes.
		// Stale shadows (`is_latest == false`) hold a reference each, so they are dropped
		// here too — they just usually aren't the one that frees the bytes.
		let legacy = RefcountFrom::<T>::get().is_some_and(|from| obsolete < from);
		let mut freed: u64 = 0;
		for (tx_info, _) in items.iter().filter(|(tx_info, _)| tx_info.meta.is_renew()) {
			let size_u64: u64 = tx_info.size.into();
			freed = freed.saturating_add(if legacy {
				size_u64
			} else {
				Self::release_renew_ref(tx_info.content_hash, size_u64)
			});
		}
		if freed > 0 {
			Self::update_permanent_storage_used(|used| used.saturating_sub(freed));
		}

		// One read, one write — `try_push` cannot overflow under
		// `items.len() <= MaxBlockTransactions` plus the `on_finalize`
		// empty-pending invariant.
		let mut pending = PendingRenewals::<T>::get();
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
			PendingRenewals::<T>::put(&pending);
		}
	}
}

/// Panics unless this pallet's weights fit the block limits. Runs alongside
/// [`pallet_bulletin_transaction_storage::ensure_weight_sanity`], which owns the mandatory
/// floor. `collator_pov_percent` is the collator PoV cap; solochains pass `None`.
#[cfg(any(test, feature = "std"))]
pub fn ensure_weight_sanity<T: Config>(collator_pov_percent: Option<u64>) {
	use frame_support::dispatch::DispatchClass;

	let block_weights = <T as frame_system::Config>::BlockWeights::get();
	let base_extrinsic = block_weights.get(DispatchClass::Normal).base_extrinsic;
	let max_block_txs = T::MaxBlockTransactions::get();
	let effective_normal =
		pallet_bulletin_transaction_storage::effective_normal_budget::<T>(collator_pov_percent);

	// A full block of `renew` calls must fit the normal budget by ref_time. `renew` is the
	// heaviest of the renewal dispatchables, so it bounds the others.
	let renew_weight = <T as Config>::WeightInfo::renew().saturating_add(base_extrinsic);
	let total_renew_ref_time = renew_weight.ref_time().saturating_mul(max_block_txs as u64);
	assert!(
		total_renew_ref_time <= effective_normal.ref_time(),
		"MaxBlockTransactions ({max_block_txs}) renew calls: total ref_time \
		 {total_renew_ref_time} exceeds effective normal limit {}",
		effective_normal.ref_time(),
	);

	// The drain inherent alone must fit `max_block`. Its sum with the storage pallet's
	// mandatory work is checked by that pallet, which is the only side that can see both.
	let drain_weight = <T as Config>::WeightInfo::process_pending_renewals(max_block_txs);
	assert!(
		drain_weight.all_lte(block_weights.max_block),
		"process_pending_renewals({max_block_txs}) weight {drain_weight:?} exceeds \
		 max block {:?}",
		block_weights.max_block,
	);

	println!("--- data_renewal weight sanity ---");
	println!("  renew + base:               {renew_weight:?}");
	println!("  full block of renews:       {total_renew_ref_time} ref_time");
	println!("  process_pending_renewals:   {drain_weight:?}");
	println!("  Effective normal budget:    {effective_normal:?}");
}

/// Storage-pallet [`BenchmarkHelper`](txs_benchmarking::BenchmarkHelper) for runtimes
/// wiring this pallet: delegates the pre-computed check proof to
/// `DefaultCheckProofHelper` and marks worst-case expiry-sweep entries `Renew` so
/// the `on_initialize_with_expiry` benchmark exercises the counter decrement in
/// [`OnObsoleteTransactions::handle_obsolete`].
#[cfg(feature = "runtime-benchmarks")]
pub struct RenewalBenchmarkHelper;

#[cfg(feature = "runtime-benchmarks")]
use pallet_bulletin_transaction_storage::benchmarking as txs_benchmarking;

#[cfg(feature = "runtime-benchmarks")]
impl<T: Config> txs_benchmarking::BenchmarkHelper<T> for RenewalBenchmarkHelper {
	fn encoded_check_proof(random_hash: &[u8]) -> alloc::vec::Vec<u8> {
		<txs_benchmarking::DefaultCheckProofHelper as txs_benchmarking::BenchmarkHelper<T>>::encoded_check_proof(random_hash)
	}

	fn worst_case_entry_meta() -> T::EntryMeta {
		T::EntryMeta::renew()
	}

	fn register_worst_case_entry(content_hash: ContentHash) {
		// Recurring and awaiting its cycle, so the lookup hits and the entry queues.
		Renewals::<T>::insert(
			content_hash,
			RenewalData {
				account: frame_benchmarking::account("renewal_owner", 0, 0),
				recurring: true,
				paid: false,
			},
		);
	}
}
