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

//! Storage auto-renewal pallet — companion to `pallet-bulletin-transaction-storage`.
//!
//! Lets accounts register a stored content hash for automatic renewal: when the
//! retention period elapses, the chain re-indexes the data into the current block
//! (consuming one transaction's worth of the registered account's authorization).
//!
//! Crossing the boundary into `pallet-bulletin-transaction-storage` happens exclusively
//! through the [`OnTransactionExpiring`] notification (going *in*) and the
//! [`StorageRenewer`] trait (coming *out*); this pallet never reaches into
//! transaction-storage's internals directly.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use bulletin_transaction_storage_primitives::ContentHash;
use codec::{Decode, Encode, MaxEncodedLen};
use pallet_bulletin_transaction_storage::{
	traits::{OnTransactionExpiring, StorageRenewer},
	TransactionInfo,
};
use polkadot_sdk_frame::{
	deps::sp_inherents::{InherentIdentifier, IsFatalError},
	prelude::*,
};

pub use pallet::*;
pub use weights::WeightInfo;

#[cfg(feature = "try-runtime")]
const LOG_TARGET: &str = "runtime::storage-auto-renewal";

/// Inherent identifier for the `process_auto_renewals` mandatory inherent.
pub const INHERENT_IDENTIFIER: InherentIdentifier = *b"autorenw";

/// Inherent error type for this pallet. The `process_auto_renewals` inherent has
/// no failure mode that can be reported from `check_inherent`.
#[derive(Encode, Decode, Debug, PartialEq, Eq)]
pub enum InherentError {
	Unknown,
}

impl IsFatalError for InherentError {
	fn is_fatal_error(&self) -> bool {
		true
	}
}

#[polkadot_sdk_frame::pallet]
pub mod pallet {
	use super::*;

	/// Data associated with an auto-renewal registration.
	#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
	pub struct AutoRenewalData<AccountId> {
		/// Account whose authorization will be consumed each time the data is auto-renewed.
		pub account: AccountId,
	}

	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// The overarching event type.
		#[allow(deprecated)]
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// Weight information for extrinsics in this pallet.
		type WeightInfo: WeightInfo;
		/// Maximum number of auto-renewals processable per block. Must match the
		/// `MaxBlockTransactions` of the underlying `pallet-bulletin-transaction-storage`
		/// so that all expiring transactions can be enqueued.
		#[pallet::constant]
		type MaxBlockTransactions: Get<u32>;
		/// Bridge to `pallet-bulletin-transaction-storage` for renewal operations and
		/// authorization queries. Wire to
		/// `pallet_bulletin_transaction_storage::Pallet<Runtime>`.
		type StorageRenewer: StorageRenewer<Self::AccountId>;
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Auto-renewal is already enabled for this content hash.
		AutoRenewalAlreadyEnabled,
		/// Auto-renewal is not enabled for this content hash.
		AutoRenewalNotEnabled,
		/// Caller is not the owner of the auto-renewal registration.
		NotAutoRenewalOwner,
		/// The content hash is not currently stored.
		ContentNotFound,
		/// Caller does not have sufficient account authorization to enable auto-renewal.
		InsufficientAuthorization,
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Auto-renewal was enabled for `content_hash` by `who`.
		AutoRenewalEnabled { content_hash: ContentHash, who: T::AccountId },
		/// Auto-renewal was disabled for `content_hash` by `who`.
		AutoRenewalDisabled { content_hash: ContentHash, who: T::AccountId },
		/// Data was automatically renewed at `index` with `content_hash` for `account`.
		DataAutoRenewed { index: u32, content_hash: ContentHash, account: T::AccountId },
		/// Auto-renewal failed for `content_hash` (insufficient authorization or block full).
		AutoRenewalFailed { content_hash: ContentHash, account: T::AccountId },
	}

	const STORAGE_VERSION: StorageVersion = StorageVersion::new(0);

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	/// Maps content hash to the registration that requested its automatic renewal.
	#[pallet::storage]
	pub type AutoRenewals<T: Config> =
		StorageMap<_, Blake2_128Concat, ContentHash, AutoRenewalData<T::AccountId>, OptionQuery>;

	/// Transactions that must be auto-renewed in the current block.
	///
	/// Populated via [`OnTransactionExpiring::on_expiring`] when transaction-storage
	/// drops a block's transactions. Cleared by the [`Call::process_auto_renewals`]
	/// mandatory inherent executed in the same block. If the inherent fails to run,
	/// `on_finalize` panics — the queue must always be drained.
	#[pallet::storage]
	pub(super) type PendingAutoRenewals<T: Config> = StorageValue<
		_,
		BoundedVec<
			(ContentHash, TransactionInfo, AutoRenewalData<T::AccountId>),
			T::MaxBlockTransactions,
		>,
		ValueQuery,
	>;

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_finalize(_n: BlockNumberFor<T>) {
			// During try-runtime testing, no inherents are submitted, so we log instead
			// of panicking and clear the queue to avoid leaking state into the next block.
			#[cfg(feature = "try-runtime")]
			if !PendingAutoRenewals::<T>::get().is_empty() {
				tracing::warn!(
					target: LOG_TARGET,
					"Pending auto-renewals were not processed (expected during try-runtime)",
				);
				PendingAutoRenewals::<T>::kill();
			}

			#[cfg(not(feature = "try-runtime"))]
			assert!(
				PendingAutoRenewals::<T>::get().is_empty(),
				"All pending auto-renewals must be processed by process_auto_renewals",
			);
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Enable automatic renewal for a previously stored piece of data.
		///
		/// `who` must currently have account authorization sufficient to renew the data
		/// once (transactions > 0 and `bytes_permanent + size <= bytes_allowance`). The
		/// authorization is **not** consumed here; it is consumed each time the data is
		/// auto-renewed. Authorization is checked here but might still be missing when
		/// actually renewed.
		///
		/// Emits [`Event::AutoRenewalEnabled`] when successful.
		#[pallet::call_index(0)]
		#[pallet::weight(T::WeightInfo::enable_auto_renew())]
		pub fn enable_auto_renew(
			origin: OriginFor<T>,
			content_hash: ContentHash,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			ensure!(
				!AutoRenewals::<T>::contains_key(content_hash),
				Error::<T>::AutoRenewalAlreadyEnabled
			);

			let tx_info = T::StorageRenewer::transaction_info_for_content_hash(content_hash)
				.ok_or(Error::<T>::ContentNotFound)?;
			let extent = T::StorageRenewer::account_authorization_extent(&who);
			// Mirror `check_authorization`'s `is_renew` path: the authorization must
			// have room for one more renewal of `tx_info.size` on the permanent axis.
			// The transactions counter is uncapped (saturating) — only the per-account
			// `bytes_permanent` cap is enforced. We also reject when the extent is empty
			// (i.e. authorization is absent or expired — `account_authorization_extent`
			// returns the default in that case, which has `bytes_allowance = 0`).
			ensure!(
				extent.bytes_allowance > 0 && extent.has_permanent_capacity(tx_info.size as u64),
				Error::<T>::InsufficientAuthorization
			);

			AutoRenewals::<T>::insert(content_hash, AutoRenewalData { account: who.clone() });
			Self::deposit_event(Event::AutoRenewalEnabled { content_hash, who });
			Ok(())
		}

		/// Disable automatic renewal for a piece of data.
		///
		/// Can only be called by the account that originally enabled auto-renewal.
		///
		/// Emits [`Event::AutoRenewalDisabled`] when successful.
		#[pallet::call_index(1)]
		#[pallet::weight(T::WeightInfo::disable_auto_renew())]
		pub fn disable_auto_renew(
			origin: OriginFor<T>,
			content_hash: ContentHash,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let renewal_data =
				AutoRenewals::<T>::get(content_hash).ok_or(Error::<T>::AutoRenewalNotEnabled)?;
			ensure!(renewal_data.account == who, Error::<T>::NotAutoRenewalOwner);

			AutoRenewals::<T>::remove(content_hash);
			Self::deposit_event(Event::AutoRenewalDisabled { content_hash, who });
			Ok(())
		}

		/// Process all pending auto-renewals for this block.
		///
		/// **Mandatory inherent.** The block author must include this whenever
		/// [`PendingAutoRenewals`] is non-empty; failure to do so causes `on_finalize`
		/// to panic and the block to be rejected.
		///
		/// For each pending item the registered account's authorization is consumed.
		/// If the account no longer has sufficient authorization, the renewal is
		/// skipped (not panicked) and an [`Event::AutoRenewalFailed`] event is emitted;
		/// the auto-renewal registration is removed for failed items so that the chain
		/// does not retry.
		#[pallet::call_index(2)]
		#[pallet::weight((T::WeightInfo::process_auto_renewals(T::MaxBlockTransactions::get()), DispatchClass::Mandatory))]
		pub fn process_auto_renewals(origin: OriginFor<T>) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			let pending = PendingAutoRenewals::<T>::take();

			for (content_hash, tx_info, renewal_data) in pending.into_iter() {
				let consumed = T::StorageRenewer::try_consume_account_authorization(
					&renewal_data.account,
					tx_info.size,
				);

				if !consumed {
					// Insufficient authorization — remove the registration so the chain
					// doesn't try (and fail) to renew it again next cycle.
					AutoRenewals::<T>::remove(content_hash);
					Self::deposit_event(Event::AutoRenewalFailed {
						content_hash,
						account: renewal_data.account,
					});
					continue;
				}

				match T::StorageRenewer::do_renew(tx_info) {
					Ok(new_index) => {
						Self::deposit_event(Event::DataAutoRenewed {
							index: new_index,
							content_hash,
							account: renewal_data.account,
						});
					},
					Err(_) => {
						// Block is full or other dispatch error — remove the registration.
						// The data will expire; the user can re-register if desired.
						AutoRenewals::<T>::remove(content_hash);
						Self::deposit_event(Event::AutoRenewalFailed {
							content_hash,
							account: renewal_data.account,
						});
					},
				}
			}

			Ok(().into())
		}
	}

	#[pallet::inherent]
	impl<T: Config> ProvideInherent for Pallet<T> {
		type Call = Call<T>;
		type Error = InherentError;
		const INHERENT_IDENTIFIER: InherentIdentifier = INHERENT_IDENTIFIER;

		fn create_inherent(_data: &InherentData) -> Option<Self::Call> {
			if PendingAutoRenewals::<T>::exists() {
				Some(Call::process_auto_renewals {})
			} else {
				None
			}
		}

		fn check_inherent(_call: &Self::Call, _data: &InherentData) -> Result<(), Self::Error> {
			Ok(())
		}

		fn is_inherent(call: &Self::Call) -> bool {
			matches!(call, Call::process_auto_renewals { .. })
		}
	}

	// `ValidateUnsigned` is deprecated upstream (will be removed after April 2027) in favour
	// of `#[pallet::authorize]` + `frame_system::AuthorizeCall`. Migration is tracked
	// separately; silence the deprecation here so `-D warnings` in CI does not block the
	// SDK bump.
	#[allow(deprecated)]
	#[pallet::validate_unsigned]
	impl<T: Config> ValidateUnsigned for Pallet<T> {
		type Call = Call<T>;

		fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
			// `process_auto_renewals` is injected by the block author, not the pool.
			// Return a valid (empty) transaction if one arrives here.
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

/// Implementation of [`OnTransactionExpiring`] that enqueues registered content
/// hashes for renewal in the current block.
impl<T: Config> OnTransactionExpiring for Pallet<T> {
	fn on_expiring(content_hash: ContentHash, tx_info: &TransactionInfo) {
		let Some(renewal_data) = AutoRenewals::<T>::get(content_hash) else {
			return;
		};
		PendingAutoRenewals::<T>::mutate(|pending| {
			// `try_push` silently drops items beyond `MaxBlockTransactions` —
			// auto-renewal is best-effort; excess items simply won't be renewed
			// this block. In practice this cannot happen because the queue is
			// bounded by the same constant as the pool of expiring transactions.
			let _ = pending.try_push((content_hash, tx_info.clone(), renewal_data));
		});
	}

	fn on_expiring_weight(n: u32) -> Weight {
		// Each call performs a single read of `AutoRenewals` and (on hit) a mutate of
		// `PendingAutoRenewals`. Charge worst-case: 1 read + 1 write per call.
		let db = <T as frame_system::Config>::DbWeight::get();
		db.reads_writes(n as u64, n as u64)
	}
}
