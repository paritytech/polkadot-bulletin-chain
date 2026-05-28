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

//! Transaction storage pallet. Indexes transactions and manages storage proofs.
//!
//! This pallet is designed to be used on chains with no transaction fees. It must be used with a
//! `TransactionExtension` implementation that calls the
//! [`validate_signed`](Pallet::validate_signed) and
//! [`pre_dispatch_signed`](Pallet::pre_dispatch_signed) functions.

// Ensure we're `no_std` when compiling for Wasm.
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "runtime-benchmarks")]
pub mod benchmarking;
pub mod weights;

pub mod migrations;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
mod types;

use alloc::vec::Vec;
use bulletin_transaction_storage_primitives::{
	cids::{calculate_cid, Cid, CidCodec, CidConfig, HashingAlgorithm, RAW_CODEC},
	ContentHash,
};
use codec::{Decode, Encode, MaxEncodedLen};
use core::fmt::Debug;
use pallet_bulletin_transaction_storage_runtime_api::AccountAuthorization;
use polkadot_sdk_frame::{
	deps::*,
	prelude::*,
	traits::{
		fungible::{hold::Balanced, Mutate, MutateHold},
		parameter_types, OriginTrait,
	},
};
use sp_transaction_storage_proof::{
	encode_index, num_chunks, random_chunk, ChunkIndex, InherentError, TransactionStorageProof,
	CHUNK_SIZE, INHERENT_IDENTIFIER,
};

// Re-export pallet items so that they can be accessed from the crate namespace.
pub use pallet::*;
pub use types::*;
pub use weights::WeightInfo;

const LOG_TARGET: &str = "runtime::transaction-storage";

/// Default retention period for data (in blocks). 14 days at 6s block time.
pub const DEFAULT_RETENTION_PERIOD: u32 = 2 * 100800;
parameter_types! {
	pub const DefaultRetentionPeriod: u32 = DEFAULT_RETENTION_PERIOD;
}

/// Maximum bytes that can be stored in one transaction.
/// Setting a higher limit may exceed the WASM allocator's 128 MiB heap and cause OOM errors.
///
/// Note: 2 MiB is aligned with the Bitswap maximum block size.
pub const DEFAULT_MAX_TRANSACTION_SIZE: u32 = 2 * 1024 * 1024;
/// Default maximum number of indexed transactions in a block.
pub const DEFAULT_MAX_BLOCK_TRANSACTIONS: u32 = 512;

/// Encountered an impossible situation, implies a bug.
pub const IMPOSSIBLE: InvalidTransaction = InvalidTransaction::Custom(0);
/// Data size is not in the allowed range.
pub const BAD_DATA_SIZE: InvalidTransaction = InvalidTransaction::Custom(1);
/// Renewed extrinsic not found.
pub const RENEWED_NOT_FOUND: InvalidTransaction = InvalidTransaction::Custom(2);
/// Authorization was not found.
pub const AUTHORIZATION_NOT_FOUND: InvalidTransaction = InvalidTransaction::Custom(3);
/// Authorization has not expired.
pub const AUTHORIZATION_NOT_EXPIRED: InvalidTransaction = InvalidTransaction::Custom(4);
/// Renew rejected: would push the signer's `bytes_permanent` past their `bytes_allowance`
/// (per-account hard cap).
pub const PERMANENT_ALLOWANCE_EXCEEDED: InvalidTransaction = InvalidTransaction::Custom(5);
/// Renew rejected: would push `PermanentStorageUsed` past `MaxPermanentStorageSize`
/// (chain-wide hard cap).
pub const CHAIN_PERMANENT_CAP_REACHED: InvalidTransaction = InvalidTransaction::Custom(6);
/// Authorizer account was not found.
pub const AUTHORIZER_NOT_FOUND: InvalidTransaction = InvalidTransaction::Custom(7);
/// Authorizer budget has not been exhausted.
pub const AUTHORIZATION_NOT_EXHAUSTED: InvalidTransaction = InvalidTransaction::Custom(8);
/// Relay-chain block number not yet available (genesis sentinel `0`).
pub const RELAY_CHAIN_TIME_UNAVAILABLE: InvalidTransaction = InvalidTransaction::Custom(9);
/// `disable_auto_renew`: no auto-renewal is registered for the given content hash.
pub const AUTO_RENEWAL_NOT_ENABLED: InvalidTransaction = InvalidTransaction::Custom(10);
/// `disable_auto_renew`: caller is not the account that registered the auto-renewal.
pub const NOT_AUTO_RENEWAL_OWNER: InvalidTransaction = InvalidTransaction::Custom(11);
/// `enable_auto_renew`: an auto-renewal is already registered for this content hash.
pub const AUTO_RENEWAL_ALREADY_ENABLED: InvalidTransaction = InvalidTransaction::Custom(12);
/// `disable_auto_renew`: the registration has been prepaid for its next cycle and
/// cannot be disabled by the owner until the cycle fires and consumes the prepayment.
/// Root can still disable for governance cleanup.
pub const CANNOT_DISABLE_PREPAID_AUTO_RENEWAL: InvalidTransaction = InvalidTransaction::Custom(13);

/// Percent of `MaxPermanentStorageSize` at which the pallet emits
/// [`Event::PermanentStorageNearCap`] (rising-edge only). Off-chain governance consumers
/// can use this as a "raise the cap or coordinate another bulletin chain" trigger.
pub const PERMANENT_STORAGE_NEAR_CAP_PERCENT: u64 = 80;

pub use extension::{CallInspector, MAX_WRAPPER_DEPTH};

#[polkadot_sdk_frame::pallet]
pub mod pallet {
	use super::*;

	/// A reason for this pallet placing a hold on funds.
	#[pallet::composite_enum]
	pub enum HoldReason {
		/// The funds are held as deposit for the used storage.
		StorageFeeHold,
	}

	#[pallet::config]
	pub trait Config:
		frame_system::Config<
		RuntimeOrigin: OriginTrait<PalletsOrigin: From<Origin<Self>> + TryInto<Origin<Self>>>,
	>
	{
		/// The overarching event type.
		#[allow(deprecated)]
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// A dispatchable call.
		type RuntimeCall: Parameter
			+ Dispatchable<RuntimeOrigin = Self::RuntimeOrigin>
			+ GetDispatchInfo
			+ From<frame_system::Call<Self>>;
		/// The fungible type for this pallet.
		type Currency: Mutate<Self::AccountId>
			+ MutateHold<Self::AccountId, Reason = Self::RuntimeHoldReason>
			+ Balanced<Self::AccountId>;
		/// The overarching runtime hold reason.
		type RuntimeHoldReason: From<HoldReason>;
		/// Handler for the unbalanced decrease when fees are burned.
		type FeeDestination: OnUnbalanced<CreditOf<Self>>;
		/// Weight information for extrinsics in this pallet.
		type WeightInfo: WeightInfo;
		/// Maximum number of indexed transactions in the block.
		#[pallet::constant]
		type MaxBlockTransactions: Get<u32>;
		/// Maximum data set in a single transaction in bytes.
		#[pallet::constant]
		type MaxTransactionSize: Get<u32>;
		/// Cap, in bytes, on total permanent storage (via `renew`) committed across
		/// all authorizations. Tracks chain-wide capacity for permanent data.
		#[pallet::constant]
		type MaxPermanentStorageSize: Get<u64>;
		/// Default window length, in **relay** blocks, used by
		/// [`Self::authorize_account`] / [`Self::authorize_preimage`] when the caller
		/// does not specify an explicit window.
		#[pallet::constant]
		type DefaultAuthorizationWindow: Get<u32>;
		/// Maximum allowed `effective_starts_at - relay_now` for the `_window`
		/// extrinsics. Caps how far into the future a slot may be scheduled.
		#[pallet::constant]
		type MaxStartsAtFuture: Get<u32>;
		/// Maximum number of overlapping authorization slots stored per scope.
		#[pallet::constant]
		type MaxAuthorizationSlots: Get<u32>;
		/// Provides the current relay-chain block number. On parachains this is
		/// [`cumulus_pallet_parachain_system::RelaychainDataProvider`]; on tests a
		/// simple advance-able u32 storage value.
		type RelayChainBlockNumberProvider: BlockNumberProvider<BlockNumber = u32>;
		/// The origin that manages the authorizer list.
		type AuthorizerRegistrarOrigin: EnsureOrigin<Self::RuntimeOrigin>;
		/// The origin that can authorize data storage.
		type Authorizer: EnsureOrigin<Self::RuntimeOrigin>;
		/// Priority of store/renew transactions.
		#[pallet::constant]
		type StoreRenewPriority: Get<TransactionPriority>;
		/// Longevity of store/renew transactions.
		#[pallet::constant]
		type StoreRenewLongevity: Get<TransactionLongevity>;
		/// Priority of unsigned transactions to remove expired authorizations.
		#[pallet::constant]
		type RemoveExpiredAuthorizationPriority: Get<TransactionPriority>;
		/// Longevity of unsigned transactions to remove expired authorizations.
		#[pallet::constant]
		type RemoveExpiredAuthorizationLongevity: Get<TransactionLongevity>;
		/// Benchmark helper — provides pre-computed proof matching this runtime's config.
		/// Use [`DefaultCheckProofHelper`](crate::benchmarking::DefaultCheckProofHelper) for
		/// [`DEFAULT_MAX_TRANSACTION_SIZE`] / [`DEFAULT_MAX_BLOCK_TRANSACTIONS`].
		#[cfg(feature = "runtime-benchmarks")]
		type BenchmarkHelper: crate::benchmarking::BenchmarkHelper<Self>;
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Attempted to call `store`/`renew` outside of block execution.
		BadContext,
		/// Data size is not in the allowed range.
		BadDataSize,
		/// Too many transactions in the block.
		TooManyTransactions,
		/// Invalid configuration.
		NotConfigured,
		/// Renewed extrinsic is not found.
		RenewedNotFound,
		/// Proof was not expected in this block.
		UnexpectedProof,
		/// Proof failed verification.
		InvalidProof,
		/// Missing storage proof.
		MissingProof,
		/// Unable to verify proof because state data is missing.
		MissingStateData,
		/// Double proof check in the block.
		DoubleCheck,
		/// Storage proof was not checked in the block.
		ProofNotChecked,
		/// Authorization was not found.
		AuthorizationNotFound,
		/// Authorization has not expired.
		AuthorizationNotExpired,
		/// Renew rejected: would push the signer's `bytes_permanent` past their
		/// `bytes_allowance` (per-account hard cap).
		PermanentAllowanceExceeded,
		/// Renew rejected: would push `PermanentStorageUsed` past
		/// `MaxPermanentStorageSize` (chain-wide hard cap).
		ChainPermanentCapReached,
		/// Content hash was not calculated.
		InvalidContentHash,
		/// Authorizer account was not found.
		AuthorizerNotFound,
		/// Authorizer is not eligible for permissionless removal — it still has budget on both
		/// axes AND (if `valid_until` is set) has not yet expired.
		AuthorizerBudgetNotExhausted,
		/// Auto-renewal is already enabled for this content hash.
		AutoRenewalAlreadyEnabled,
		/// Auto-renewal is not enabled for this content hash.
		AutoRenewalNotEnabled,
		/// Caller is not the owner of the auto-renewal registration.
		NotAutoRenewalOwner,
		/// Push of a new authorization slot would exceed
		/// [`Config::MaxAuthorizationSlots`] (after the lazy prune).
		TooManySlots,
		/// `(starts_at, expiration)` is not a valid window: `expiration <= starts_at`,
		/// `expiration <= relay_now`, or `starts_at` is more than
		/// [`Config::MaxStartsAtFuture`] blocks into the future. A `starts_at`
		/// in the past is accepted (treated as already-active).
		InvalidWindow,
		/// The relay-chain block number is unavailable (genesis sentinel `0`,
		/// before the parachain system inherent has populated validation data).
		RelayChainTimeUnavailable,
		/// `disable_auto_renew` rejected: the registration has been prepaid for its next
		/// cycle. The owner must wait until that cycle consumes the prepayment before
		/// disabling. Root can disable regardless.
		CannotDisablePrepaidAutoRenewal,
		/// `valid_until` supplied to `add_authorizer` is in the past (`<= now`, would
		/// expire immediately). Pass `None` for no expiration.
		InvalidValidUntil,
		/// `authorize_account` / `authorize_preimage` called by a signer whose
		/// `AllowedAuthorizers` budget cannot cover the requested
		/// `transactions` / `bytes` (or `max_size`).
		InsufficientAuthorizerBudget,
		/// `add_authorizer` rejected: the `authorization_period` override is either
		/// zero or `>= DefaultAuthorizationWindow`. The override exists to *shorten*
		/// this authorizer's window; pass `None` to use the default length.
		InvalidAuthorizationPeriodOverride,
	}

	const STORAGE_VERSION: StorageVersion = StorageVersion::new(5);

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	/// Data associated with a renewal registration in [`AutoRenewals`].
	///
	/// Holds the owner account, a `recurring` flag that decides whether the
	/// registration is consumed after a single successful renewal (`false`, set by
	/// [`Pallet::renew`]) or persists forever (`true`, set by
	/// [`Pallet::enable_auto_renew`]), and a `paid` flag indicating that the next
	/// cycle has already been charged against the owner's authorization at
	/// registration time.
	///
	/// Both [`Pallet::renew`] and [`Pallet::enable_auto_renew`] insert with
	/// `paid: true`: the extension's `check_signed` charges `bytes_permanent`,
	/// `PermanentStorageUsed`, and one tx slot up front (same as `force_renew`).
	/// [`Pallet::do_process_auto_renewals`] keys its charge-skip off `paid`: when
	/// `paid` is true the cycle renews without re-charging and then flips `paid`
	/// to false (for recurring entries) so subsequent cycles pay per-cycle, as
	/// before. One-shot entries (`recurring: false`) are removed after the single
	/// renewal so the flag is inert after that point.
	///
	/// While `paid` is true, [`Pallet::disable_auto_renew`] rejects signed callers
	/// — the owner must wait for the first cycle to consume the prepayment. This
	/// is what makes `enable_auto_renew` honestly cost a renewal even if the
	/// owner immediately disables (bytes already left the per-account quota at
	/// registration). Root can still disable for governance cleanup.
	#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
	pub struct RenewalData<AccountId> {
		/// Account whose authorization will be consumed each time data is auto-renewed.
		pub account: AccountId,
		/// `true` — auto-renew forever (set by `enable_auto_renew`).
		/// `false` — one-shot: removed from `AutoRenewals` after the first successful
		/// renewal cycle (set by `renew`).
		pub recurring: bool,
		/// `true` — the next renewal cycle has already been charged at registration
		/// time and will fire free. After the cycle delivers, the flag is flipped to
		/// `false` for recurring entries; for one-shot entries the registration is
		/// removed outright.
		pub paid: bool,
	}

	/// Custom origin for authorized signed transaction storage operations.
	///
	/// This origin is set by the [`extension::ValidateStorageCalls`] transaction extension
	/// for signed transactions that pass authorization checks. Unsigned transactions
	/// do not use this origin (they are validated via [`ValidateUnsigned`]).
	#[pallet::origin]
	#[derive(
		Clone,
		PartialEq,
		Eq,
		Debug,
		codec::Encode,
		codec::Decode,
		codec::DecodeWithMemTracking,
		scale_info::TypeInfo,
		codec::MaxEncodedLen,
	)]
	pub enum Origin<T: Config> {
		/// A signed transaction that has been authorized to store data.
		/// Contains the signer and the scope of authorization that was consumed.
		Authorized { who: T::AccountId, scope: AuthorizationScopeFor<T> },
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		/// Mandatory per-block hook: ages out the obsolete `Transactions[obsolete]` entry,
		/// decrements [`PermanentStorageUsed`] for any `kind == Renew` items in it, cleans
		/// up `TransactionByContentHash`, and queues auto-renewals for `process_auto_renewals`.
		///
		/// Weight is charged via the [`WeightInfo::on_initialize_with_expiry`] benchmark.
		/// The fit within `max_block` is asserted by [`ensure_weight_sanity`] — every
		/// runtime should exercise it from a test.
		fn on_initialize(n: BlockNumberFor<T>) -> Weight {
			let mut weight = Weight::zero();

			// Run v0→v1 migration if it hasn't been applied yet.
			// This handles the case where `codeSubstitutes` loaded the fix runtime
			// without triggering `on_runtime_upgrade` (spec_version unchanged).
			// Safe alongside the regular `MigrateV0ToV1` wired in Executive: both
			// check `on_chain_storage_version() < 1`, so whichever runs first bumps
			// the version and the other becomes a no-op.
			// TODO: Remove once all chains have been migrated past v1 — after that
			// this is just a redundant storage read per block.
			weight.saturating_accrue(migrations::v1::maybe_migrate_v0_to_v1::<T>());

			// Drop obsolete roots and decrement the chain-wide permanent counter for any
			// renewed bytes that just aged out. The proof for `obsolete` will be checked
			// later in this block, so we drop `obsolete` - 1.
			let period = Self::retention_period();
			let obsolete = n.saturating_sub(period.saturating_add(One::one()));
			let mut num_expiring: u32 = 0;
			if obsolete > Zero::zero() {
				if let Some(transactions) = <Transactions<T>>::take(obsolete) {
					num_expiring = transactions.len() as u32;

					// Decrement the chain-wide permanent counter for any renewed bytes that
					// just aged out (covers entries flagged `TransactionKind::Renew`).
					let renewed_sum: u64 = transactions
						.iter()
						.filter(|t| matches!(t.kind, TransactionKind::Renew))
						.fold(0u64, |acc, t| acc.saturating_add(t.size as u64));
					if renewed_sum > 0 {
						Self::update_permanent_storage_used(|used| {
							used.saturating_sub(renewed_sum)
						});
					}

					// Before removing, collect any transactions that are registered for
					// auto-renewal and schedule them for processing this block.
					let mut pending = PendingAutoRenewals::<T>::get();
					for tx_info in transactions.iter() {
						let hash: ContentHash = tx_info.content_hash;
						// Only act on this entry if it is the latest reference for `hash`:
						// a newer `store`/`renew` (or the force-renew inside
						// `enable_auto_renew`) may have moved `TransactionByContentHash` to a
						// later block, in which case this is a stale shadow entry that should
						// not trigger cleanup or re-schedule auto-renewal — the later entry's
						// own expiry will.
						let is_latest = TransactionByContentHash::<T>::get(hash)
							.is_some_and(|(block, _)| block == obsolete);
						if !is_latest {
							continue;
						}
						TransactionByContentHash::<T>::remove(hash);
						// `try_push` cannot overflow: `pending` is empty per `on_finalize`'s
						// drain invariant, and `transactions.len() <= MaxBlockTransactions`.
						if let Some(renewal_data) = AutoRenewals::<T>::get(hash) {
							let _ = pending.try_push((hash, tx_info.clone(), renewal_data));
						}
					}
					if !pending.is_empty() {
						PendingAutoRenewals::<T>::put(&pending);
					}
				}
			}

			// Charge the expiry-sweep cost via the benchmarked weight. `n = 0` covers the
			// no-expiry path (early blocks, blocks where obsolete had no transactions);
			// the constant component captures the RetentionPeriod read and the
			// reservation for `on_finalize`.
			weight.saturating_accrue(T::WeightInfo::on_initialize_with_expiry(num_expiring));

			weight
		}

		fn on_finalize(n: BlockNumberFor<T>) {
			let proof_ok = <ProofChecked<T>>::take() || {
				// Proof is not required for early or empty blocks.
				let period = Self::retention_period();
				let target_number = n.saturating_sub(period);

				target_number.is_zero() || {
					// An empty block means no transactions were stored, relying on the fact
					// below that we store transactions only if they contain chunks.
					!Transactions::<T>::contains_key(target_number)
				}
			};

			// During try-runtime testing, no inherents (including storage proofs) are
			// submitted, so we log instead of panicking.
			#[cfg(feature = "try-runtime")]
			if !proof_ok {
				tracing::warn!(
					target: LOG_TARGET,
					"Storage proof was not checked in this block (expected during try-runtime)"
				);
			}
			#[cfg(not(feature = "try-runtime"))]
			assert!(proof_ok, "Storage proof must be checked once in the block");

			// All pending auto-renewals must have been processed by the
			// `apply_block_inherents` inherent.
			#[cfg(feature = "try-runtime")]
			if !PendingAutoRenewals::<T>::get().is_empty() {
				tracing::warn!(
					target: LOG_TARGET,
					"Pending auto-renewals were not processed (expected during try-runtime)"
				);
				// Clear pending renewals so try-runtime doesn't leave stale state.
				PendingAutoRenewals::<T>::kill();
			}

			#[cfg(not(feature = "try-runtime"))]
			assert!(
				PendingAutoRenewals::<T>::get().is_empty(),
				"All pending auto-renewals must be processed by apply_block_inherents"
			);

			// Insert new transactions, iff they have chunks.
			let transactions = <BlockTransactions<T>>::take();
			let total_chunks = TransactionInfo::total_chunks(&transactions);
			if total_chunks != 0 {
				<Transactions<T>>::insert(n, transactions);
			}
		}

		#[cfg(feature = "try-runtime")]
		fn try_state(n: BlockNumberFor<T>) -> Result<(), sp_runtime::TryRuntimeError> {
			Self::do_try_state(n)
		}

		fn integrity_test() {
			assert!(
				!T::MaxBlockTransactions::get().is_zero(),
				"MaxBlockTransactions must be greater than zero"
			);
			assert!(
				!T::MaxTransactionSize::get().is_zero(),
				"MaxTransactionSize must be greater than zero"
			);
			let default_period = DEFAULT_RETENTION_PERIOD.into();
			let retention_period = GenesisConfig::<T>::default().retention_period;
			assert_eq!(
				retention_period, default_period,
				"GenesisConfig.retention_period must match DEFAULT_RETENTION_PERIOD"
			);
			assert!(
				!T::DefaultAuthorizationWindow::get().is_zero(),
				"DefaultAuthorizationWindow must be greater than zero"
			);
			assert!(
				!T::MaxAuthorizationSlots::get().is_zero(),
				"MaxAuthorizationSlots must be greater than zero"
			);
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Index and store data off chain. Minimum data size is 1 byte, maximum is
		/// `MaxTransactionSize`. Data will be removed after `RetentionPeriod` blocks, unless
		/// `renew` is called.
		///
		/// Authorization is required to store data using regular signed/unsigned transactions.
		/// Regular signed transactions require account authorization (see
		/// [`authorize_account`](Self::authorize_account)), regular unsigned transactions require
		/// preimage authorization (see [`authorize_preimage`](Self::authorize_preimage)).
		///
		/// Emits [`Stored`](Event::Stored) when successful.
		///
		/// ## Complexity
		///
		/// O(n*log(n)) of data size, as all data is pushed to an in-memory trie.
		#[pallet::call_index(0)]
		#[pallet::weight(T::WeightInfo::store(data.len() as u32))]
		#[pallet::feeless_if(|origin: &OriginFor<T>, data: &Vec<u8>| -> bool { true })]
		pub fn store(origin: OriginFor<T>, data: Vec<u8>) -> DispatchResult {
			let _caller = Self::ensure_authorized(origin)?;
			Self::do_store(data, HashingAlgorithm::Blake2b256, RAW_CODEC)
		}

		/// Index and store data off chain with an explicit CID configuration.
		///
		/// Behaves identically to [`store`](Self::store), but the CID configuration
		/// (codec and hashing algorithm) is passed directly as a parameter.
		///
		/// Emits [`Stored`](Event::Stored) when successful.
		#[pallet::call_index(9)]
		#[pallet::weight(T::WeightInfo::store(data.len() as u32))]
		#[pallet::feeless_if(|_origin: &OriginFor<T>, _cid: &CidConfig, _data: &Vec<u8>| -> bool { true })]
		pub fn store_with_cid_config(
			origin: OriginFor<T>,
			cid: CidConfig,
			data: Vec<u8>,
		) -> DispatchResult {
			let _caller = Self::ensure_authorized(origin)?;
			Self::do_store(data, cid.hashing, cid.codec)
		}

		/// Schedule a **one-shot** auto-renewal of previously stored data. The renewal fires
		/// exactly once, when the data reaches its `RetentionPeriod` boundary, and then the
		/// registration is removed. For continuous renewal, use
		/// [`enable_auto_renew`](Self::enable_auto_renew) instead.
		///
		/// `entry` identifies the data either by `(block, index)` or by content hash.
		///
		/// Feeless. Registration cost (one transaction unit) is charged in `check_signed`;
		/// the eventual renewal cycle charges bytes against `bytes_permanent` and the
		/// chain-wide cap.
		///
		/// Rejects with [`AutoRenewalAlreadyEnabled`](Error::AutoRenewalAlreadyEnabled) if a
		/// scheduled renewal already exists for this content hash.
		///
		/// Emits [`RenewalEnabled`](Event::RenewalEnabled) `{ recurring: false }`.
		///
		/// For synchronous renewal at dispatch time, see [`force_renew`](Self::force_renew).
		#[pallet::call_index(1)]
		#[pallet::weight(T::WeightInfo::renew())]
		#[pallet::feeless_if(|_origin: &OriginFor<T>, _entry: &TransactionRef<BlockNumberFor<T>>| -> bool { true })]
		pub fn renew(
			origin: OriginFor<T>,
			entry: TransactionRef<BlockNumberFor<T>>,
		) -> DispatchResult {
			let AuthorizedCaller::Signed { who, scope: _ } = Self::ensure_authorized(origin)?
			else {
				return Err(DispatchError::BadOrigin);
			};
			let info = Self::resolve_transaction_ref(&entry)?;
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
		/// Authorization is required (as with [`store`](Self::store)). Charges `info.size`
		/// against `bytes_permanent` (per-account renew cap) and `PermanentStorageUsed`
		/// (chain-wide cap).
		///
		/// Emits [`Renewed`](Event::Renewed) when successful.
		#[pallet::call_index(2)]
		#[pallet::weight((T::WeightInfo::force_renew(), DispatchClass::Operational))]
		#[pallet::feeless_if(|_origin: &OriginFor<T>, _entry: &TransactionRef<BlockNumberFor<T>>| -> bool { true })]
		pub fn force_renew(
			origin: OriginFor<T>,
			entry: TransactionRef<BlockNumberFor<T>>,
		) -> DispatchResultWithPostInfo {
			let _caller = Self::ensure_authorized(origin)?;
			let info = Self::resolve_transaction_ref(&entry)?;

			// In the case of a regular unsigned transaction, this should have been checked by
			// pre_dispatch. In the case of a regular signed transaction, this should have been
			// checked by pre_dispatch_signed.
			Self::ensure_data_size_ok(info.size as usize)?;

			let content_hash = info.content_hash;
			let new_index = Self::do_renew(info)?;
			Self::deposit_event(Event::Renewed { index: new_index, content_hash });
			Ok(().into())
		}

		/// Authorize an account to store up to `bytes` of arbitrary data in
		/// `transactions` boost-tier transactions, with the default authorization
		/// window applied: `starts_at = relay_now`,
		/// `expiration = relay_now + DefaultAuthorizationWindow`.
		///
		/// If a slot with that exact window already exists for the account, the
		/// new caps are **added** to it (existing used counters preserved).
		/// Otherwise a new slot is appended; if the bounded vec is already full
		/// (after the lazy prune), the call fails with
		/// [`Error::TooManySlots`]. To target a specific window, use
		/// [`Self::authorize_account_window`].
		///
		/// Parameters:
		///
		/// - `who`: The account to be credited with an authorization to store data.
		/// - `transactions`: The number of boost-tier transactions that `who` may submit.
		/// - `bytes`: The number of bytes that `who` may submit.
		///
		/// The origin for this call must be the pallet's `Authorizer`. Emits
		/// [`AccountAuthorized`](Event::AccountAuthorized) when successful.
		#[pallet::call_index(3)]
		#[pallet::weight(T::WeightInfo::authorize_account())]
		pub fn authorize_account(
			origin: OriginFor<T>,
			who: T::AccountId,
			transactions: u32,
			bytes: u64,
		) -> DispatchResult {
			Self::do_authorize_account(origin, who, transactions, bytes, None, None)
		}

		/// Authorize an account with an explicit `(starts_at, expiration)` slot
		/// window expressed in **relay** block numbers.
		///
		/// `starts_at = None` means "active immediately" (`relay_now`). The
		/// effective `starts_at` must be `>= relay_now`, no more than
		/// [`Config::MaxStartsAtFuture`] blocks ahead, and strictly less than
		/// `expiration`. If a slot with the same exact window already exists,
		/// the call is **additive** (caps merged, used counters preserved);
		/// otherwise a new slot is pushed.
		///
		/// The origin must be the pallet's `Authorizer`. Emits
		/// [`AccountAuthorized`](Event::AccountAuthorized) when successful.
		#[pallet::call_index(7)]
		#[pallet::weight(T::WeightInfo::authorize_account_window())]
		#[pallet::feeless_if(|origin: &OriginFor<T>, _who: &T::AccountId, _transactions: &u32, _bytes: &u64, _starts_at: &Option<u32>, _expiration: &u32| -> bool {
			T::Authorizer::try_origin(origin.clone()).is_ok()
		})]
		pub fn authorize_account_window(
			origin: OriginFor<T>,
			who: T::AccountId,
			transactions: u32,
			bytes: u64,
			starts_at: Option<u32>,
			expiration: u32,
		) -> DispatchResult {
			Self::do_authorize_account(
				origin,
				who,
				transactions,
				bytes,
				starts_at,
				Some(expiration),
			)
		}

		/// Authorize anyone to store a preimage of the given content hash with
		/// the default authorization window.
		///
		/// If a slot with the default window already exists for this hash, the
		/// new `max_size` is **added** to its `bytes_allowance` (additive).
		/// Otherwise a new slot is pushed; preimage slots carry
		/// `transactions_allowance = 2` so the canonical store-then-renew flow
		/// fits (the slot's tx-counter axis is gated hard on consume and each
		/// of store/renew bumps it). To target a specific window use
		/// [`Self::authorize_preimage_window`].
		///
		/// The origin for this call must be the pallet's `Authorizer`. Emits
		/// [`PreimageAuthorized`](Event::PreimageAuthorized) when successful.
		#[pallet::call_index(4)]
		#[pallet::weight(T::WeightInfo::authorize_preimage())]
		pub fn authorize_preimage(
			origin: OriginFor<T>,
			content_hash: ContentHash,
			max_size: u64,
		) -> DispatchResult {
			Self::do_authorize_preimage(origin, content_hash, max_size, None, None)
		}

		/// Authorize a preimage with an explicit `(starts_at, expiration)` slot
		/// window. Same validation rules as [`Self::authorize_account_window`].
		///
		/// Emits [`PreimageAuthorized`](Event::PreimageAuthorized) when successful.
		#[pallet::call_index(8)]
		#[pallet::weight(T::WeightInfo::authorize_preimage_window())]
		#[pallet::feeless_if(|origin: &OriginFor<T>, _content_hash: &ContentHash, _max_size: &u64, _starts_at: &Option<u32>, _expiration: &u32| -> bool {
			T::Authorizer::try_origin(origin.clone()).is_ok()
		})]
		pub fn authorize_preimage_window(
			origin: OriginFor<T>,
			content_hash: ContentHash,
			max_size: u64,
			starts_at: Option<u32>,
			expiration: u32,
		) -> DispatchResult {
			Self::do_authorize_preimage(origin, content_hash, max_size, starts_at, Some(expiration))
		}

		/// Remove an expired account authorization from storage. Anyone can call this.
		///
		/// Parameters:
		///
		/// - `who`: The account with an expired authorization to remove.
		///
		/// Emits [`ExpiredAccountAuthorizationRemoved`](Event::ExpiredAccountAuthorizationRemoved)
		/// when successful.
		#[pallet::call_index(5)]
		#[pallet::weight(T::WeightInfo::remove_expired_account_authorization())]
		pub fn remove_expired_account_authorization(
			_origin: OriginFor<T>,
			who: T::AccountId,
		) -> DispatchResult {
			Self::remove_expired_authorization(AuthorizationScope::Account(who.clone()))?;
			Self::deposit_event(Event::ExpiredAccountAuthorizationRemoved { who });
			Ok(())
		}

		/// Remove an expired preimage authorization from storage. Anyone can call this.
		///
		/// Parameters:
		///
		/// - `content_hash`: The BLAKE2b hash that was authorized.
		///
		/// Emits
		/// [`ExpiredPreimageAuthorizationRemoved`](Event::ExpiredPreimageAuthorizationRemoved)
		/// when successful.
		#[pallet::call_index(6)]
		#[pallet::weight(T::WeightInfo::remove_expired_preimage_authorization())]
		pub fn remove_expired_preimage_authorization(
			_origin: OriginFor<T>,
			content_hash: ContentHash,
		) -> DispatchResult {
			Self::remove_expired_authorization(AuthorizationScope::Preimage(content_hash))?;
			Self::deposit_event(Event::ExpiredPreimageAuthorizationRemoved { content_hash });
			Ok(())
		}

		/// Refresh the latest-expiring slot of an existing authorization for an account
		/// by extending its expiration to `relay_now + DefaultAuthorizationWindow`.
		/// Per-slot counters (`bytes`, `transactions`, `bytes_permanent`) and caps
		/// (`bytes_allowance`, `transactions_allowance`) are left untouched. Use
		/// [`authorize_account_window`](Self::authorize_account_window) to add a new slot
		/// or extend with fresh caps.
		///
		/// Fails with [`Error::AuthorizationNotFound`] when no slot remains after the
		/// lazy prune.
		#[pallet::call_index(18)]
		#[pallet::weight(T::WeightInfo::refresh_account_authorization())]
		pub fn refresh_account_authorization(
			origin: OriginFor<T>,
			who: T::AccountId,
		) -> DispatchResult {
			T::Authorizer::ensure_origin(origin)?;
			Self::refresh_authorization(AuthorizationScope::Account(who.clone()))?;
			Self::deposit_event(Event::AccountAuthorizationRefreshed { who });
			Ok(())
		}

		/// Refresh the latest-expiring slot of an existing authorization for a preimage
		/// content hash. See [`Self::refresh_account_authorization`] for semantics.
		#[pallet::call_index(19)]
		#[pallet::weight(T::WeightInfo::refresh_preimage_authorization())]
		pub fn refresh_preimage_authorization(
			origin: OriginFor<T>,
			content_hash: ContentHash,
		) -> DispatchResult {
			T::Authorizer::ensure_origin(origin)?;
			Self::refresh_authorization(AuthorizationScope::Preimage(content_hash))?;
			Self::deposit_event(Event::PreimageAuthorizationRefreshed { content_hash });
			Ok(())
		}

		/// Enable automatic renewal for a previously stored piece of data.
		///
		/// **Recurring scheduler with pre-paid first cycle.** The extension's
		/// `check_signed` charges `bytes_permanent`, `PermanentStorageUsed`, and
		/// one tx slot at registration (same hard-cap accounting as `force_renew`
		/// / one-shot `renew`). The registration is inserted as
		/// [`RenewalData`] `{ recurring: true, paid: true }`. The first renewal
		/// cycle fires at the next `RetentionPeriod` boundary **without**
		/// re-charging — the slot is already paid for; the cycle then flips
		/// `paid` to `false`. From that point on, every subsequent cycle charges
		/// the owner's authorization in [`Self::do_process_auto_renewals`],
		/// dropping the registration with [`Event::AutoRenewalFailed`] if the
		/// quota is exhausted at cycle time.
		///
		/// Feeless: no token fee. Spam is bounded structurally by the up-front
		/// hard-cap charge — the caller cannot over-schedule past their
		/// `bytes_allowance` or the chain-wide `MaxPermanentStorageSize`.
		/// [`Self::disable_auto_renew`] additionally rejects the owner while
		/// `paid` is `true`, so the prepayment cannot be reclaimed before the
		/// first cycle fires.
		///
		/// Emits [`RenewalEnabled`](Event::RenewalEnabled) `{ recurring: true }`
		/// for the registration; the first actual renewal is emitted as
		/// [`DataAutoRenewed`](Event::DataAutoRenewed) at cycle time.
		#[pallet::call_index(12)]
		#[pallet::weight(T::WeightInfo::enable_auto_renew())]
		#[pallet::feeless_if(|_origin: &OriginFor<T>, _content_hash: &ContentHash| -> bool { true })]
		pub fn enable_auto_renew(
			origin: OriginFor<T>,
			content_hash: ContentHash,
		) -> DispatchResult {
			let AuthorizedCaller::Signed { who, scope: _ } = Self::ensure_authorized(origin)?
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
			// `do_renew`, otherwise `bytes_permanent` would be double-charged (once
			// by registration, once by `do_process_auto_renewals`'s prepaid cycle).
			let (block, index) = TransactionByContentHash::<T>::get(content_hash)
				.ok_or(Error::<T>::RenewedNotFound)?;
			Self::transaction_info(block, index).ok_or(Error::<T>::RenewedNotFound)?;

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
		/// [`Pallet::renew`] and [`Pallet::enable_auto_renew`] start with `paid: true`;
		/// the owner has to wait for the first cycle to consume the prepayment before
		/// they can disable.
		///
		/// Root: bypasses the owner check and the prepaid-window check
		/// (governance/cleanup).
		///
		/// Feeless: no token fee and no authorization is consumed. Signed admission is
		/// gated in [`check_signed`](Self::check_signed) on ownership and the prepaid
		/// flag, so a caller can issue at most one successful `disable_auto_renew` per
		/// registration it owns — and only after the first cycle has fired.
		///
		/// Emits [`AutoRenewalDisabled`](Event::AutoRenewalDisabled) when successful.
		#[pallet::call_index(13)]
		#[pallet::weight(T::WeightInfo::disable_auto_renew())]
		#[pallet::feeless_if(|_origin: &OriginFor<T>, _content_hash: &ContentHash| -> bool { true })]
		pub fn disable_auto_renew(
			origin: OriginFor<T>,
			content_hash: ContentHash,
		) -> DispatchResult {
			let caller = Self::ensure_authorized(origin)?;
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

		/// Composite block-level inherent: optionally validates a transaction storage proof and
		/// always drains [`PendingAutoRenewals`].
		///
		/// `ProvideInherent::create_inherent` only returns a single `Call`, but this pallet
		/// has two block-end concerns — verifying the storage proof for the block at
		/// `n - RetentionPeriod`, and renewing entries flagged via [`AutoRenewals`] before
		/// they expire at `n - RetentionPeriod - 1`. Both effects collapse into this single
		/// mandatory inherent so that block authors emit one extrinsic that satisfies both
		/// `on_finalize` invariants (`ProofChecked` and "PendingAutoRenewals empty").
		///
		/// `proof` is `Some` when the inherent data provider supplied one; otherwise the
		/// proof step is skipped (early or empty blocks). The auto-renewal drain runs
		/// unconditionally — emitting an inherent at all implies that `on_initialize` may
		/// have populated `PendingAutoRenewals`.
		#[pallet::call_index(14)]
		#[pallet::weight((
			T::WeightInfo::apply_block_inherents(T::MaxBlockTransactions::get()),
			DispatchClass::Mandatory,
		))]
		pub fn apply_block_inherents(
			origin: OriginFor<T>,
			proof: Option<TransactionStorageProof>,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			if let Some(proof) = proof {
				Self::do_check_proof(proof)?;
			}
			// Refund from the worst-case declaration to the count actually drained.
			let n_actual = Self::do_process_auto_renewals();
			Ok(Some(T::WeightInfo::apply_block_inherents(n_actual)).into())
		}

		/// Add an account to the set of allowed authorizers. Allowed authorizers can call
		/// [`authorize_account`](Self::authorize_account) and
		/// [`authorize_preimage`](Self::authorize_preimage) to grant storage access.
		///
		/// If the account is already an allowed authorizer, its `budget` is **overwritten**
		/// with the new values.
		///
		/// `budget` constraints:
		///
		/// - `valid_until`: when `Some(t)`, must satisfy `t > now`; the entry stops authorizing
		///   once `now >= t` and becomes eligible for permissionless cleanup via
		///   [`remove_exhausted_authorizer`](Self::remove_exhausted_authorizer).
		///
		/// The origin for this call must satisfy `AuthorizerRegistrarOrigin`. Emits
		/// [`AuthorizerAdded`](Event::AuthorizerAdded) when successful.
		#[pallet::call_index(15)]
		#[pallet::weight(T::WeightInfo::add_authorizer())]
		pub fn add_authorizer(
			origin: OriginFor<T>,
			who: T::AccountId,
			budget: AuthorizerBudget<BlockNumberFor<T>>,
		) -> DispatchResult {
			T::AuthorizerRegistrarOrigin::ensure_origin(origin)?;
			ensure!(!budget.is_expired(Self::now()), Error::<T>::InvalidValidUntil);
			if let Some(period) = budget.authorization_period {
				ensure!(
					period > 0 && period < T::DefaultAuthorizationWindow::get(),
					Error::<T>::InvalidAuthorizationPeriodOverride,
				);
			}
			AllowedAuthorizers::<T>::insert(&who, budget);
			Self::deposit_event(Event::AuthorizerAdded { who });
			Ok(())
		}

		/// Remove an account from the set of allowed authorizers. The removed account will no
		/// longer be able to call [`authorize_account`](Self::authorize_account) or
		/// [`authorize_preimage`](Self::authorize_preimage).
		///
		/// If the account is not currently an allowed authorizer, this is a no-op.
		///
		/// Parameters:
		///
		/// - `who`: The account to remove from the allowed authorizers.
		///
		/// The origin for this call must satisfy `AuthorizerRegistrarOrigin`. Emits
		/// [`AuthorizerRemoved`](Event::AuthorizerRemoved) when successful.
		#[pallet::call_index(16)]
		#[pallet::weight(T::WeightInfo::remove_authorizer())]
		pub fn remove_authorizer(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AuthorizerRegistrarOrigin::ensure_origin(origin)?;
			// `take` returns the previous value; only emit the event when an entry
			// actually existed so observers don't see phantom "removed" events.
			if AllowedAuthorizers::<T>::take(&who).is_some() {
				Self::deposit_event(Event::AuthorizerRemoved { who });
			}
			Ok(())
		}

		/// Remove an authorizer that is exhausted (budget zero on either axis) or expired
		/// (`now >= valid_until` for an entry that set `valid_period`). Anyone can call this.
		///
		/// Parameters:
		///
		/// - `who`: The authorizer to remove.
		///
		/// Emits [`ExhaustedAuthorizerRemoved`](Event::ExhaustedAuthorizerRemoved)
		/// when successful.
		#[pallet::call_index(17)]
		#[pallet::weight(T::WeightInfo::remove_exhausted_authorizer())]
		pub fn remove_exhausted_authorizer(
			_origin: OriginFor<T>,
			who: T::AccountId,
		) -> DispatchResult {
			AllowedAuthorizers::<T>::try_mutate_exists(&who, |maybe_budget| {
				let budget = maybe_budget.as_ref().ok_or(Error::<T>::AuthorizerNotFound)?;
				ensure!(
					Self::authorizer_removable(budget),
					Error::<T>::AuthorizerBudgetNotExhausted,
				);
				*maybe_budget = None;
				Ok::<_, DispatchError>(())
			})?;
			Self::deposit_event(Event::ExhaustedAuthorizerRemoved { who });
			Ok(())
		}
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Stored data under specified index.
		Stored { index: u32, content_hash: ContentHash, cid: Option<Cid> },
		/// Renewed data under specified index.
		Renewed { index: u32, content_hash: ContentHash },
		/// Storage proof was successfully checked.
		ProofChecked,
		/// An account `who` was authorized to store `bytes` bytes in `transactions` boost-tier
		/// transactions over the slot window `[starts_at, expiration)` (relay block numbers).
		AccountAuthorized {
			who: T::AccountId,
			transactions: u32,
			bytes: u64,
			starts_at: u32,
			expiration: u32,
		},
		/// Authorization was given for a preimage of `content_hash` (not exceeding `max_size`) to
		/// be stored by anyone over the slot window `[starts_at, expiration)`.
		PreimageAuthorized {
			content_hash: ContentHash,
			max_size: u64,
			starts_at: u32,
			expiration: u32,
		},
		/// An expired account authorization was removed.
		ExpiredAccountAuthorizationRemoved { who: T::AccountId },
		/// An expired preimage authorization was removed.
		ExpiredPreimageAuthorizationRemoved { content_hash: ContentHash },
		/// The latest active slot's expiration of an account authorization was extended
		/// by [`refresh_account_authorization`](Pallet::refresh_account_authorization).
		AccountAuthorizationRefreshed { who: T::AccountId },
		/// The latest active slot's expiration of a preimage authorization was extended
		/// by [`refresh_preimage_authorization`](Pallet::refresh_preimage_authorization).
		PreimageAuthorizationRefreshed { content_hash: ContentHash },
		/// An authorizer was added to the allowed list.
		AuthorizerAdded { who: T::AccountId },
		/// An authorizer was removed from the allowed list by the manager.
		AuthorizerRemoved { who: T::AccountId },
		/// An authorizer was removed from the allowed list due to budget exhaustion.
		ExhaustedAuthorizerRemoved { who: T::AccountId },
		/// A renewal was enabled for `content_hash` by `who`.
		RenewalEnabled { content_hash: ContentHash, who: T::AccountId, recurring: bool },
		/// Auto-renewal disabled for `content_hash`. `who` is the registration's owner
		/// (not the caller when Root issued the disable).
		AutoRenewalDisabled { content_hash: ContentHash, who: T::AccountId },
		/// Data was automatically renewed at `index` with `content_hash` for `account`.
		DataAutoRenewed { index: u32, content_hash: ContentHash, account: T::AccountId },
		/// Auto-renewal failed for `content_hash` (insufficient authorization for `account`).
		AutoRenewalFailed { content_hash: ContentHash, account: T::AccountId },
		/// `PermanentStorageUsed` changed (a `renew` bumped it, or the lazy drain
		/// decremented it). Off-chain capacity-planning consumers can drive their dashboards
		/// from these.
		PermanentStorageUsedUpdated { used: u64 },
		/// `PermanentStorageUsed` just crossed the [`PERMANENT_STORAGE_NEAR_CAP_PERCENT`]
		/// threshold of `MaxPermanentStorageSize` on the rising edge. Emitted once per
		/// crossing — no re-emission while still above the threshold.
		PermanentStorageNearCap { used: u64, cap: u64 },
	}

	/// Per-scope [`Authorization`] entries. Lazily pruned: every read or
	/// mutate that goes through [`Pallet::prune_expired`] drops **expired**
	/// slots (`expiration <= relay_now`). When the inner `slots` vec becomes
	/// empty the entry is removed and the provider-ref (for `Account` scope)
	/// is decremented.
	#[pallet::storage]
	pub(super) type Authorizations<T: Config> =
		StorageMap<_, Blake2_128Concat, AuthorizationScopeFor<T>, Authorization<T>, OptionQuery>;

	/// List of accounts allowed to give authorizations.
	#[pallet::storage]
	pub type AllowedAuthorizers<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, AuthorizerBudgetFor<T>, OptionQuery>;

	/// Collection of transaction metadata by block number.
	#[pallet::storage]
	#[pallet::getter(fn transaction_roots)]
	pub type Transactions<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		BlockNumberFor<T>,
		BoundedVec<TransactionInfo, T::MaxBlockTransactions>,
		OptionQuery,
	>;

	#[pallet::storage]
	/// Storage fee per byte.
	pub type ByteFee<T: Config> = StorageValue<_, BalanceOf<T>>;

	#[pallet::storage]
	/// Storage fee per transaction.
	pub type EntryFee<T: Config> = StorageValue<_, BalanceOf<T>>;

	/// Number of blocks for which stored data must be retained.
	///
	/// Data older than `RetentionPeriod` blocks is eligible for removal unless it
	/// has been explicitly renewed. Validators are required to prove possession of
	/// data corresponding to block `N - RetentionPeriod` when producing block `N`.
	#[pallet::storage]
	pub type RetentionPeriod<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

	// Intermediates
	#[pallet::storage]
	pub(super) type BlockTransactions<T: Config> =
		StorageValue<_, BoundedVec<TransactionInfo, T::MaxBlockTransactions>, ValueQuery>;

	/// Maps content hash to its most recent (block_number, tx_index) location.
	#[pallet::storage]
	pub(super) type TransactionByContentHash<T: Config> =
		StorageMap<_, Blake2_128Concat, ContentHash, (BlockNumberFor<T>, u32), OptionQuery>;

	/// Maps content hash to the account that registered it for auto-renewal.
	#[pallet::storage]
	pub type AutoRenewals<T: Config> =
		StorageMap<_, Blake2_128Concat, ContentHash, RenewalData<T::AccountId>, OptionQuery>;

	/// Transactions that must be auto-renewed in the current block.
	///
	/// Populated by `on_initialize` when a block's data is about to expire.
	/// Cleared by the `apply_block_inherents` mandatory inherent executed in the same block.
	#[pallet::storage]
	pub(super) type PendingAutoRenewals<T: Config> = StorageValue<
		_,
		BoundedVec<
			(ContentHash, TransactionInfo, RenewalData<T::AccountId>),
			T::MaxBlockTransactions,
		>,
		ValueQuery,
	>;

	/// Was the proof checked in this block?
	#[pallet::storage]
	pub(super) type ProofChecked<T: Config> = StorageValue<_, bool, ValueQuery>;

	/// Chain-wide total of currently-on-chain renewed bytes. Source of truth for the
	/// chain-wide hard cap: a `renew` of `size` bytes is rejected when
	/// `PermanentStorageUsed + size > MaxPermanentStorageSize`.
	///
	/// Bumped on each successful `renew`. Decremented by `on_initialize` when an obsolete
	/// `Transactions[block]` is removed: each entry with `kind == Renew` contributes its
	/// `size` to the decrement.
	#[pallet::storage]
	pub type PermanentStorageUsed<T: Config> = StorageValue<_, u64, ValueQuery>;

	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		pub byte_fee: BalanceOf<T>,
		pub entry_fee: BalanceOf<T>,
		pub retention_period: BlockNumberFor<T>,
		/// Initial additional accounts that are allowed to issue authorizations and their budgets
		/// as (account, transaction, bytes) tuples.
		pub allowed_authorizers: Vec<(T::AccountId, u32, u64)>,
		/// Initial account authorizations as (account, transactions_allowance, bytes_allowance)
		/// tuples.
		pub account_authorizations: Vec<(T::AccountId, u32, u64)>,
		/// Initial preimage authorizations as (content_hash, max_size) tuples. Each preimage
		/// gets `transactions_allowance = 2` to match the runtime-authorized flow
		/// (store-then-renew).
		pub preimage_authorizations: Vec<(ContentHash, u64)>,
	}

	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> Self {
			Self {
				byte_fee: 10u32.into(),
				entry_fee: 1000u32.into(),
				retention_period: DEFAULT_RETENTION_PERIOD.into(),
				allowed_authorizers: Vec::new(),
				account_authorizations: Vec::new(),
				preimage_authorizations: Vec::new(),
			}
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
		fn build(&self) {
			ByteFee::<T>::put(self.byte_fee);
			EntryFee::<T>::put(self.entry_fee);
			RetentionPeriod::<T>::put(self.retention_period);
			// Genesis runs before the parachain inherent populates validation data,
			// so `RelayChainBlockNumberProvider::current_block_number()` returns the
			// default `0`. We accept `0` here on purpose: genesis-supplied slots
			// are a build-time convenience for dev/test chains, and a `starts_at`
			// of `0` simply means "active immediately".
			let starts_at: u32 = T::RelayChainBlockNumberProvider::current_block_number();
			let expiration = starts_at.saturating_add(T::DefaultAuthorizationWindow::get());
			for (who, transactions_allowance, bytes_allowance) in &self.account_authorizations {
				Pallet::<T>::add_slot(
					AuthorizationScope::Account(who.clone()),
					*transactions_allowance,
					*bytes_allowance,
					starts_at,
					expiration,
				)
				.expect("genesis account authorization fits in MaxAuthorizationSlots; qed");
			}
			for (content_hash, max_size) in &self.preimage_authorizations {
				Pallet::<T>::add_slot(
					AuthorizationScope::Preimage(*content_hash),
					2,
					*max_size,
					starts_at,
					expiration,
				)
				.expect("genesis preimage authorization fits in MaxAuthorizationSlots; qed");
			}
			for (account, transactions, bytes) in &self.allowed_authorizers {
				AllowedAuthorizers::<T>::insert(
					account,
					AuthorizerBudget {
						quota: Some(Quota { transactions: *transactions, bytes: *bytes }),
						// Genesis authorizers use the default window; root can re-add
						// them later to set a custom `authorization_period` if needed.
						authorization_period: None,
						// Genesis authorizers never expire; root can re-add them later to set
						// a `valid_until` if needed.
						valid_until: None,
					},
				);
			}
		}
	}

	#[pallet::inherent]
	impl<T: Config> ProvideInherent for Pallet<T> {
		type Call = Call<T>;
		type Error = InherentError;
		const INHERENT_IDENTIFIER: InherentIdentifier = INHERENT_IDENTIFIER;

		fn create_inherent(data: &InherentData) -> Option<Self::Call> {
			// `ProvideInherent::create_inherent` returns a single `Call`, but two block-end
			// concerns may need to land in the same block: verifying the storage proof for the
			// block at `n - RetentionPeriod`, and draining `PendingAutoRenewals` populated by
			// `on_initialize`. Both effects collapse into the composite
			// `Call::apply_block_inherents { proof: Option<_> }`, emitted whenever either side
			// has work to do.
			let proof = data
				.get_data::<TransactionStorageProof>(&Self::INHERENT_IDENTIFIER)
				.unwrap_or(None);
			let has_pending_renewals = !PendingAutoRenewals::<T>::get().is_empty();

			if proof.is_none() && !has_pending_renewals {
				return None;
			}
			Some(Call::apply_block_inherents { proof })
		}

		fn check_inherent(_call: &Self::Call, _data: &InherentData) -> Result<(), Self::Error> {
			Ok(())
		}

		fn is_inherent(call: &Self::Call) -> bool {
			matches!(call, Call::apply_block_inherents { .. })
		}
	}

	// `ValidateUnsigned` is deprecated upstream (will be removed after April 2027) in favour of
	// `#[pallet::authorize]` + `frame_system::AuthorizeCall`. Migration is tracked separately;
	// silence the deprecation here so `-D warnings` in CI does not block the SDK bump.
	#[allow(deprecated)]
	#[pallet::validate_unsigned]
	impl<T: Config> ValidateUnsigned for Pallet<T> {
		type Call = Call<T>;

		fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
			// Mandatory inherent (`apply_block_inherents`) is injected by the block author,
			// not the transaction pool. Return a valid but empty transaction if one arrives
			// here.
			if Self::is_inherent(call) {
				return Ok(ValidTransaction::default());
			}
			Self::check_unsigned(call, CheckContext::Validate)?.ok_or(IMPOSSIBLE.into())
		}

		fn pre_dispatch(call: &Self::Call) -> Result<(), TransactionValidityError> {
			// Allow inherents here.
			if Self::is_inherent(call) {
				return Ok(());
			}

			Self::check_unsigned(call, CheckContext::PreDispatch).map(|_| ())
		}
	}

	impl<T: Config> Pallet<T> {
		/// Verify a transaction storage proof for the block at `n - RetentionPeriod` and mark
		/// [`ProofChecked`]. Invoked by the [`Self::apply_block_inherents`] mandatory inherent.
		pub(super) fn do_check_proof(proof: TransactionStorageProof) -> DispatchResult {
			ensure!(!ProofChecked::<T>::get(), Error::<T>::DoubleCheck);

			let number = <frame_system::Pallet<T>>::block_number();
			let period = Self::retention_period();
			let target_number = number.saturating_sub(period);
			ensure!(!target_number.is_zero(), Error::<T>::UnexpectedProof);
			// Shape-tolerant: `transactions_at` falls back to the v2 layout while the
			// v2→v3 multi-block migration is still in flight, so historical entries
			// that have not yet been rewritten can still be proof-verified.
			let transactions =
				Self::transactions_at(target_number).ok_or(Error::<T>::MissingStateData)?;

			let parent_hash = frame_system::Pallet::<T>::parent_hash();
			Self::verify_chunk_proof(proof, parent_hash.as_ref(), transactions.to_vec())?;
			ProofChecked::<T>::put(true);
			Self::deposit_event(Event::ProofChecked);
			Ok(())
		}

		/// Drain [`PendingAutoRenewals`] and return the count drained.
		///
		/// Batches the [`BlockTransactions`] read/write across all `n` renewals by threading
		/// an in-memory accumulator through repeated [`Self::do_renew_in_memory`] calls.
		/// A naive `do_renew`-per-item loop would re-encode the full vec per iteration
		/// (O(n²)), which a linear weight model underestimates by ~17% at saturation.
		///
		/// **Failure handling.** A pending renewal is treated as failed (the registration
		/// is removed from [`AutoRenewals`] and [`Event::AutoRenewalFailed`] is emitted)
		/// when any of the following hold:
		///
		/// - [`Self::check_authorization`] rejects — the auth was missing/expired, the per-account
		///   renew quota (`bytes_permanent + size > bytes_allowance`) was exhausted, or the
		///   chain-wide cap (`PermanentStorageUsed + size > MaxPermanentStorageSize`) would be
		///   breached.
		/// - [`Self::do_renew_in_memory`] returns `None` because the per-block transaction slot cap
		///   (`MaxBlockTransactions`) is reached.
		///
		/// On failure the data is **gone**: the same `on_initialize` that queued the
		/// pending renewal already `take`-d the obsolete `Transactions` entry and cleared
		/// [`TransactionByContentHash`]. The caller cannot re-`enable_auto_renew` because
		/// the content hash no longer resolves to a stored entry — to keep the data alive
		/// they must re-`store` it first.
		pub(super) fn do_process_auto_renewals() -> u32 {
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
						Self::check_authorization(&scope, tx_info.size, true, true).is_ok();
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
							// `mutate` (not `insert`) so a Root `disable_auto_renew`
							// executed earlier in the same block — between the
							// `on_initialize` queue and this inherent — is not silently
							// re-armed by a fresh insert.
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
							// pathological event (the inherent runs before any user extrinsics,
							// and `len(pending) <= MaxBlockTransactions`), and reaching into the
							// current `Authorizations` entry to refund would silently apply
							// across auth roll-overs.
							let size_u64: u64 = tx_info.size.into();
							Self::update_permanent_storage_used(|used| {
								used.saturating_sub(size_u64)
							});
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

		/// Push a `kind = Renew` entry onto the in-memory accumulator and update
		/// [`TransactionByContentHash`]. Returns `None` at `MaxBlockTransactions`.
		///
		/// Called by:
		/// - [`Self::do_renew`] for the single-renewal manual flow (`force_renew`).
		/// - [`Self::do_process_auto_renewals`] in a loop, amortizing one [`BlockTransactions`]
		///   read/write across all pending entries.
		///
		/// The hard-cap accounting (per-account `bytes_permanent`, chain-wide
		/// [`PermanentStorageUsed`]) is performed by [`Self::check_authorization`] —
		/// invoked by the extension's `pre_dispatch` for the manual flow and by
		/// [`Self::do_process_auto_renewals`] for the auto flow before this is called.
		fn do_renew_in_memory(
			transactions: &mut BoundedVec<TransactionInfo, T::MaxBlockTransactions>,
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
			TransactionByContentHash::<T>::insert(info.content_hash, (Self::now(), new_index));
			Some(new_index)
		}
	}

	impl<T: Config> Pallet<T> {
		/// Read [`PermanentStorageUsed`], apply `f` to compute the new value, write it back,
		/// and emit [`Event::PermanentStorageUsedUpdated`]. If the value was below the
		/// [`PERMANENT_STORAGE_NEAR_CAP_PERCENT`] threshold and crossed it (rising edge),
		/// also emit [`Event::PermanentStorageNearCap`].
		///
		/// Centralising read + write + events in one helper guarantees every change to the
		/// chain-wide counter is observable off-chain, and that the near-cap signal fires
		/// exactly once per crossing.
		fn update_permanent_storage_used(f: impl FnOnce(u64) -> u64) {
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

		/// Validate that `origin` is one of the accepted caller types for store/renew
		/// extrinsics, and return a typed description of the caller.
		///
		/// Accepted origins:
		///
		/// - [`Origin::Authorized`] (set by [`extension::ValidateStorageCalls`]) →
		///   [`AuthorizedCaller::Signed`]
		/// - Root → [`AuthorizedCaller::Root`]
		/// - None (unsigned) → [`AuthorizedCaller::Unsigned`]
		///
		/// Any other origin (including plain `Signed`) returns
		/// [`DispatchError::BadOrigin`].
		pub fn ensure_authorized(
			origin: OriginFor<T>,
		) -> Result<AuthorizedCallerFor<T>, DispatchError> {
			// 1. Try pallet::Origin::Authorized (set by ValidateStorageCalls extension)
			if let Ok(Origin::Authorized { who, scope }) = origin.clone().into_caller().try_into() {
				return Ok(AuthorizedCaller::Signed { who, scope });
			}

			// 2. Try root
			if ensure_root(origin.clone()).is_ok() {
				return Ok(AuthorizedCaller::Root);
			}

			// 3. Try none (unsigned)
			ensure_none(origin)?;
			Ok(AuthorizedCaller::Unsigned)
		}

		/// Common implementation for [`store`](Self::store) and
		/// [`store_with_cid_config`](Self::store_with_cid_config).
		pub fn do_store(
			data: Vec<u8>,
			hashing: HashingAlgorithm,
			cid_codec: CidCodec,
		) -> DispatchResult {
			let data_len = data.len() as u32;

			// In the case of a regular unsigned transaction, this should have been checked by
			// pre_dispatch. In the case of a regular signed transaction, this should have been
			// checked by pre_dispatch_signed.
			Self::ensure_data_size_ok(data_len as usize)?;

			let cid_config = CidConfig { codec: cid_codec, hashing };
			let cid =
				calculate_cid(&data, cid_config).map_err(|_| Error::<T>::InvalidContentHash)?;

			// Chunk data and compute storage root
			let chunks: Vec<_> = data.chunks(CHUNK_SIZE).map(|c| c.to_vec()).collect();

			// We don't need `data` anymore.
			core::mem::drop(data);

			let chunk_count = chunks.len() as u32;
			debug_assert_eq!(chunk_count, num_chunks(data_len));
			let root = sp_io::trie::blake2_256_ordered_root(chunks, sp_runtime::StateVersion::V1);

			let extrinsic_index =
				<frame_system::Pallet<T>>::extrinsic_index().ok_or(Error::<T>::BadContext)?;

			let index = Self::append_to_block_transactions(
				root,
				data_len,
				cid.content_hash,
				hashing,
				cid_codec,
				extrinsic_index,
				TransactionKind::Store,
			)?;
			// Index after the runtime mutation — index ops aren't rolled back on dispatch error.
			sp_io::transaction_index::index(extrinsic_index, data_len, cid.content_hash);

			Self::deposit_event(Event::Stored {
				index,
				content_hash: cid.content_hash,
				cid: cid.to_bytes(),
			});

			Ok(())
		}

		/// Single-renewal entry point for the [`force_renew`](Self::force_renew) and
		/// [`enable_auto_renew`](Self::enable_auto_renew) dispatchables.
		///
		/// Wraps [`Self::do_renew_in_memory`] (the centralized renewal mechanics) with a
		/// [`BlockTransactions`] read/write. Auto-renewals do not go through this wrapper
		/// — [`Self::do_process_auto_renewals`] amortizes a single read/write across the
		/// whole drain loop instead.
		///
		/// Hard-cap accounting (per-account `bytes_permanent`, chain-wide
		/// [`PermanentStorageUsed`]) is enforced by [`Self::check_authorization`] in the
		/// extension's `pre_dispatch` before this runs.
		fn do_renew(info: TransactionInfo) -> Result<u32, Error<T>> {
			let extrinsic_index =
				<frame_system::Pallet<T>>::extrinsic_index().ok_or(Error::<T>::BadContext)?;
			<BlockTransactions<T>>::try_mutate(|transactions| {
				Self::do_renew_in_memory(transactions, &info, extrinsic_index)
					.ok_or(Error::<T>::TooManyTransactions)
			})
		}

		/// Append a new entry to [`BlockTransactions`] (with the cumulative `block_chunks`)
		/// and update [`TransactionByContentHash`]. Caller must call
		/// `transaction_index::{index,renew}` AFTER this — host calls aren't rolled back on
		/// dispatch error.
		fn append_to_block_transactions(
			chunk_root: <BlakeTwo256 as Hash>::Output,
			size: u32,
			content_hash: ContentHash,
			hashing: HashingAlgorithm,
			cid_codec: CidCodec,
			extrinsic_index: u32,
			kind: TransactionKind,
		) -> Result<u32, Error<T>> {
			let new_index = <BlockTransactions<T>>::try_mutate(|transactions| {
				let block_chunks =
					TransactionInfo::total_chunks(transactions).saturating_add(num_chunks(size));
				let new_index = transactions.len() as u32;
				transactions
					.try_push(TransactionInfo {
						chunk_root,
						size,
						content_hash,
						hashing,
						cid_codec,
						extrinsic_index,
						block_chunks,
						kind,
					})
					.map_err(|_| Error::<T>::TooManyTransactions)?;
				Ok::<_, Error<T>>(new_index)
			})?;
			TransactionByContentHash::<T>::insert(content_hash, (Self::now(), new_index));
			Ok(new_index)
		}

		/// Returns the current relay-chain block number reported by
		/// [`Config::RelayChainBlockNumberProvider`]. `0` is the genesis sentinel
		/// (validation data not yet populated by `set_validation_data`); callers
		/// that need a real time should go through [`Self::ensure_relay_now`].
		pub(crate) fn relay_now() -> u32 {
			T::RelayChainBlockNumberProvider::current_block_number()
		}

		/// Current parachain block number — used for authorizer validity checks
		/// (`AuthorizerBudget::valid_until`). Slot windows use [`Self::relay_now`].
		pub(crate) fn now() -> BlockNumberFor<T> {
			frame_system::Pallet::<T>::block_number()
		}

		/// Like [`Self::relay_now`] but rejects the genesis sentinel `0` with
		/// [`Error::RelayChainTimeUnavailable`] / [`RELAY_CHAIN_TIME_UNAVAILABLE`].
		pub(crate) fn ensure_relay_now() -> Result<u32, Error<T>> {
			let n = Self::relay_now();
			if n == 0 {
				return Err(Error::<T>::RelayChainTimeUnavailable);
			}
			Ok(n)
		}

		/// Validate an `(effective_starts_at, expiration)` slot window against
		/// the current relay block:
		///
		/// - The window must be non-empty: `expiration > effective_starts_at`.
		/// - The window must not be already expired: `expiration > relay_now`.
		/// - A future `starts_at` may not be more than [`Config::MaxStartsAtFuture`] blocks ahead.
		///   A `starts_at` in the past is **accepted** — semantically it just means "already
		///   active".
		fn ensure_valid_window(
			relay_now: u32,
			effective_starts_at: u32,
			expiration: u32,
		) -> Result<(), Error<T>> {
			ensure!(expiration > effective_starts_at, Error::<T>::InvalidWindow);
			ensure!(expiration > relay_now, Error::<T>::InvalidWindow);
			ensure!(
				effective_starts_at.saturating_sub(relay_now) <= T::MaxStartsAtFuture::get(),
				Error::<T>::InvalidWindow
			);
			Ok(())
		}

		/// Resolve `(starts_at, expiration)` for an authorize call.
		///
		/// `expiration = None` selects the default window — `relay_now +
		/// authorizer_period`, where `authorizer_period` is the authorizer's
		/// [`AuthorizerBudget::authorization_period`] override (if set) or
		/// [`Config::DefaultAuthorizationWindow`].
		///
		/// `expiration = Some(_)` is the explicit `_window` form: validated via
		/// [`Self::ensure_valid_window`], **and** capped to `authorizer_period`
		/// when the override is set — the requested span
		/// `expiration - effective_starts_at` must not exceed it. This is the
		/// policy-enforcement half of the override: an authorizer with a
		/// shorter period cannot issue a longer window via `_window`.
		fn resolve_window(
			relay_now: u32,
			starts_at: Option<u32>,
			expiration: Option<u32>,
			override_period: Option<u32>,
		) -> Result<(u32, u32), Error<T>> {
			match expiration {
				None => {
					let period = override_period.unwrap_or_else(T::DefaultAuthorizationWindow::get);
					Ok((relay_now, relay_now.saturating_add(period)))
				},
				Some(expiration) => {
					let effective_starts_at = starts_at.unwrap_or(relay_now);
					Self::ensure_valid_window(relay_now, effective_starts_at, expiration)?;
					if let Some(period) = override_period {
						let span = expiration.saturating_sub(effective_starts_at);
						ensure!(span <= period, Error::<T>::InvalidWindow);
					}
					Ok((effective_starts_at, expiration))
				},
			}
		}

		/// Returns the `authorization_period` override registered for the
		/// signer of `origin`, if any. Root / XCM / non-signed origins have no
		/// entry in [`AllowedAuthorizers`] and return `None`. Callers must
		/// invoke this *before* `T::Authorizer::ensure_origin` consumes the
		/// origin.
		fn authorization_period_override_for(origin: &OriginFor<T>) -> Option<u32> {
			let signer = frame_system::ensure_signed(origin.clone()).ok()?;
			AllowedAuthorizers::<T>::get(&signer)?.authorization_period
		}

		/// Shared body for [`Self::authorize_account`] and
		/// [`Self::authorize_account_window`]. `expiration = None` selects the
		/// default window; `Some` runs full window validation.
		fn do_authorize_account(
			origin: OriginFor<T>,
			who: T::AccountId,
			transactions: u32,
			bytes: u64,
			starts_at: Option<u32>,
			expiration: Option<u32>,
		) -> DispatchResult {
			// Capture the signer (if any) and read their `authorization_period`
			// override before `ensure_origin` consumes the origin. Root / XCM
			// origins have no signer, no budget, no override.
			let signer = frame_system::ensure_signed(origin.clone()).ok();
			let override_period = Self::authorization_period_override_for(&origin);
			T::Authorizer::ensure_origin(origin)?;
			ensure!(bytes > 0, Error::<T>::BadDataSize);
			let relay_now = Self::ensure_relay_now()?;
			let (starts_at, expiration) =
				Self::resolve_window(relay_now, starts_at, expiration, override_period)?;
			// Charge budget only after window + size validation succeed, so a
			// malformed call doesn't dock the authorizer.
			if let Some(signer) = signer {
				Self::consume_authorizer_budget(&signer, transactions, bytes)?;
			}
			Self::add_slot(
				AuthorizationScope::Account(who.clone()),
				transactions,
				bytes,
				starts_at,
				expiration,
			)?;
			Self::deposit_event(Event::AccountAuthorized {
				who,
				transactions,
				bytes,
				starts_at,
				expiration,
			});
			Ok(())
		}

		/// Shared body for [`Self::authorize_preimage`] and
		/// [`Self::authorize_preimage_window`]. Preimage slots carry a
		/// `transactions_allowance` of `2` so the canonical "store-then-renew"
		/// flow fits — the slot model gates the tx-counter axis hard and a
		/// single-tx budget would block the renew. The authorizer's budget is
		/// charged `(1, max_size)` per grant — preimage scope is single-use from
		/// the authorizer's perspective.
		fn do_authorize_preimage(
			origin: OriginFor<T>,
			content_hash: ContentHash,
			max_size: u64,
			starts_at: Option<u32>,
			expiration: Option<u32>,
		) -> DispatchResult {
			let signer = frame_system::ensure_signed(origin.clone()).ok();
			let override_period = Self::authorization_period_override_for(&origin);
			T::Authorizer::ensure_origin(origin)?;
			ensure!(max_size > 0, Error::<T>::BadDataSize);
			let relay_now = Self::ensure_relay_now()?;
			let (starts_at, expiration) =
				Self::resolve_window(relay_now, starts_at, expiration, override_period)?;
			if let Some(signer) = signer {
				Self::consume_authorizer_budget(&signer, 1, max_size)?;
			}
			Self::add_slot(
				AuthorizationScope::Preimage(content_hash),
				2,
				max_size,
				starts_at,
				expiration,
			)?;
			Self::deposit_event(Event::PreimageAuthorized {
				content_hash,
				max_size,
				starts_at,
				expiration,
			});
			Ok(())
		}

		pub(crate) fn authorization_added(scope: &AuthorizationScopeFor<T>) {
			match scope {
				AuthorizationScope::Account(who) => {
					// Allow nonce storage for transaction replay protection
					frame_system::Pallet::<T>::inc_providers(who);
				},
				AuthorizationScope::Preimage(_) => (),
			}
		}

		fn authorization_removed(scope: &AuthorizationScopeFor<T>) {
			match scope {
				AuthorizationScope::Account(who) => {
					// Cleanup nonce storage. Authorized accounts should be careful to use a short
					// enough lifetime for their store/renew transactions that they aren't at risk
					// of replay when the account is next authorized.
					if let Err(err) = frame_system::Pallet::<T>::dec_providers(who) {
						tracing::warn!(
							target: LOG_TARGET,
							error=?err, ?who,
							"Failed to decrement provider reference count for authorized account leaking reference"
						);
					}
				},
				AuthorizationScope::Preimage(_) => (),
			}
		}

		/// Read the slots vec for a scope, drop **expired** slots, and write
		/// back the result. If the vec becomes empty the entry is removed and
		/// [`Self::authorization_removed`] is called (decrementing the
		/// provider-ref for `Account` scope).
		///
		/// Drained slots — those whose `bytes`, `bytes_permanent`, or
		/// `transactions` counters have hit the corresponding allowance — are
		/// intentionally **not** pruned. `store()` never gates on the byte or
		/// tx caps (those drive only the priority boost), so a drained slot can
		/// still serve low-priority stores until it expires.
		///
		/// Every read or mutate path that exposes slot state should go through
		/// this helper first so the "no expired slots remain" lazy invariant
		/// holds for the rest of the call.
		pub(crate) fn prune_expired(scope: &AuthorizationScopeFor<T>) {
			let relay_now = Self::relay_now();
			// `mutate_exists` writes back unconditionally when the value is
			// `Some` after the closure, so guard with a cheap read so the
			// no-prune-needed path on every store/renew validate avoids
			// pointless storage churn.
			let needs_prune = Authorizations::<T>::get(scope)
				.is_some_and(|auth| auth.slots.iter().any(|s| s.expiration <= relay_now));
			if !needs_prune {
				return;
			}
			Authorizations::<T>::mutate_exists(scope, |maybe_auth| {
				let Some(auth) = maybe_auth.as_mut() else {
					return;
				};
				auth.slots.retain(|slot| slot.expiration > relay_now);
				if auth.slots.is_empty() {
					*maybe_auth = None;
					Self::authorization_removed(scope);
				}
			});
		}

		/// Insert a slot into the bounded vec, keeping it sorted by `expiration`
		/// ascending (tiebreak `starts_at`). The new caps are **added** to an
		/// existing slot when:
		///
		/// 1. the windows match exactly (`starts_at` and `expiration` equal), or
		/// 2. both slots share the same `expiration` AND are **already active**
		///    (`existing.starts_at <= relay_now` AND `new.starts_at <= relay_now`). A `starts_at`
		///    in the past is observationally equivalent to `relay_now` for an active slot, so two
		///    such slots that expire at the same time can be folded with no semantic loss.
		///
		/// Otherwise push; surfaces [`Error::TooManySlots`] when the bounded
		/// vec is full after the lazy prune.
		fn add_slot(
			scope: AuthorizationScopeFor<T>,
			transactions_allowance: u32,
			bytes_allowance: u64,
			starts_at: u32,
			expiration: u32,
		) -> DispatchResult {
			Self::prune_expired(&scope);
			let relay_now = Self::relay_now();

			Authorizations::<T>::try_mutate(&scope, |maybe_auth| -> DispatchResult {
				let was_empty = maybe_auth.is_none();
				let auth = maybe_auth.get_or_insert_with(Authorization::<T>::default);

				// Additive merge: exact match, or both slots already active.
				let new_is_active = starts_at <= relay_now;
				if let Some(existing) = auth.slots.iter_mut().find(|s| {
					s.expiration == expiration &&
						(s.starts_at == starts_at || (new_is_active && s.starts_at <= relay_now))
				}) {
					// Pre-clamp the saturating axes (`bytes`, `transactions`)
					// to the **old** caps before widening the allowance. The
					// merge is then a pure simplification — the folded extent
					// matches what it would have been if the new slot were
					// stored separately:
					//   two-slot view:  bytes = min(existing.bytes, old_alw) + 0
					//   merged view:    bytes = min(existing.bytes, old_alw + new)
					// pre-clamping ensures both equal `min(existing.bytes,
					// old_alw)`. `bytes_permanent` is already bounded by the
					// renew hard cap, so no clamp is needed there.
					existing.extent.bytes =
						existing.extent.bytes.min(existing.extent.bytes_allowance);
					existing.extent.transactions =
						existing.extent.transactions.min(existing.extent.transactions_allowance);
					existing.extent.bytes_allowance =
						existing.extent.bytes_allowance.saturating_add(bytes_allowance);
					existing.extent.transactions_allowance = existing
						.extent
						.transactions_allowance
						.saturating_add(transactions_allowance);
					return Ok(());
				}

				let new_slot = TimedAuthorization {
					extent: AuthorizationExtent {
						bytes: 0,
						bytes_permanent: 0,
						bytes_allowance,
						transactions: 0,
						transactions_allowance,
					},
					starts_at,
					expiration,
				};

				let insert_at = auth
					.slots
					.iter()
					.position(|s| {
						s.expiration > expiration ||
							(s.expiration == expiration && s.starts_at > starts_at)
					})
					.unwrap_or(auth.slots.len());

				auth.slots
					.try_insert(insert_at, new_slot)
					.map_err(|_| Error::<T>::TooManySlots)?;

				if was_empty {
					Self::authorization_added(&scope);
				}
				Ok(())
			})
		}

		/// Extend the latest-expiring active slot's `expiration` to
		/// `relay_now + DefaultAuthorizationWindow`. Per-slot counters and caps are
		/// preserved; only the expiration on that one slot moves forward (a no-op
		/// if the slot already expires later). Slots remain sorted because the
		/// touched slot was already the maximum.
		///
		/// Fails with [`Error::AuthorizationNotFound`] when no active slot remains
		/// after the lazy prune.
		fn refresh_authorization(scope: AuthorizationScopeFor<T>) -> DispatchResult {
			Self::prune_expired(&scope);
			let relay_now = Self::relay_now();
			let new_expiration = relay_now.saturating_add(T::DefaultAuthorizationWindow::get());
			Authorizations::<T>::try_mutate(&scope, |maybe_auth| -> DispatchResult {
				let auth = maybe_auth.as_mut().ok_or(Error::<T>::AuthorizationNotFound)?;
				let slot = auth
					.slots
					.iter_mut()
					.filter(|s| s.is_active(relay_now))
					.max_by_key(|s| s.expiration)
					.ok_or(Error::<T>::AuthorizationNotFound)?;
				if new_expiration > slot.expiration {
					slot.expiration = new_expiration;
				}
				Ok(())
			})
		}

		/// Remove the entry if every slot in it has expired. Anyone-callable
		/// cleanup; lazy maintenance already drops expired/drained slots on every
		/// read or mutate, so this just lets a pool-only path reclaim provider-ref
		/// without waiting for the next mutate.
		fn remove_expired_authorization(scope: AuthorizationScopeFor<T>) -> DispatchResult {
			let auth = Authorizations::<T>::get(&scope).ok_or(Error::<T>::AuthorizationNotFound)?;
			ensure!(auth.all_expired(Self::relay_now()), Error::<T>::AuthorizationNotExpired);
			Authorizations::<T>::remove(&scope);
			Self::authorization_removed(&scope);
			Ok(())
		}

		/// Returns the **effective extent** for `scope` at the current relay
		/// block. Prunes expired slots, loads the entry, and delegates the fold
		/// to [`Authorization::extent`]. The consumption path picks a single
		/// slot atomically (see [`Self::check_authorization`]); this folded
		/// view is for external queries and the priority boost.
		fn authorization_extent(scope: AuthorizationScopeFor<T>) -> AuthorizationExtent {
			Self::prune_expired(&scope);
			let Some(auth) = Authorizations::<T>::get(&scope) else {
				return AuthorizationExtent::default();
			};
			auth.extent(Self::relay_now())
		}

		/// Returns the (folded, active-only) authorization extent for `who`.
		pub fn account_authorization_extent(who: T::AccountId) -> AuthorizationExtent {
			Self::authorization_extent(AuthorizationScope::Account(who))
		}

		/// Active-authorization summary for `who`, shaped for the
		/// [`BulletinTransactionStorageApi`] runtime API. Returns `None` if the
		/// account has no slot active at the current relay block.
		///
		/// `expires_at` is the maximum `expiration` across active slots — i.e.
		/// the latest relay block at which any of the account's current slots
		/// would still admit `store`/`renew` traffic. `bytes_used`,
		/// `bytes_permanent_used`, and `bytes_allowance` come from the folded
		/// active-slot extent (see [`Authorization::extent`]).
		///
		/// [`BulletinTransactionStorageApi`]:
		/// pallet_bulletin_transaction_storage_runtime_api::BulletinTransactionStorageApi
		pub fn account_authorization(
			who: T::AccountId,
		) -> Option<AccountAuthorization<BlockNumberFor<T>>> {
			let scope = AuthorizationScope::Account(who);
			Self::prune_expired(&scope);
			let auth = Authorizations::<T>::get(&scope)?;
			let relay_now = Self::relay_now();
			let expires_at = auth
				.slots
				.iter()
				.filter(|slot| slot.is_active(relay_now))
				.map(|slot| slot.expiration)
				.max()?;
			let extent = auth.extent(relay_now);
			Some(AccountAuthorization {
				expires_at: expires_at.into(),
				bytes_allowance: extent.bytes_allowance,
				bytes_used: extent.bytes,
				bytes_permanent_used: extent.bytes_permanent,
				transactions_allowance: extent.transactions_allowance,
				transactions_used: extent.transactions,
			})
		}

		/// Returns `true` iff a `store(data)` call where `data.len() == data_len`
		/// would currently pass transaction validation for `who`.
		///
		/// Mirrors the preconditions enforced by [`Self::store`] +
		/// [`Self::check_authorization`] (`is_renew = false`):
		///
		/// - `data_len` is within `[1, MaxTransactionSize]`
		/// - `who` has at least one active slot
		///
		/// `store` saturates against `bytes` / `transactions` and uses the priority
		/// boost (soft limit), so no per-account or chain-wide hard cap applies here.
		pub fn can_store(who: &T::AccountId, data_len: u32) -> bool {
			if !Self::data_size_ok(data_len as usize) {
				return false;
			}
			Self::account_has_active_authorization(who)
		}

		/// Returns `true` iff a `renew(entry)` call would currently pass transaction
		/// validation for `who`.
		///
		/// Mirrors the preconditions enforced by [`Self::renew`] +
		/// [`Self::check_authorization`] (`is_renew = true`):
		///
		/// - `entry` resolves to currently-stored data
		/// - the stored data's size is within `[1, MaxTransactionSize]`
		/// - `who` has at least one active slot with permanent capacity for `size`
		/// - chain-wide hard cap: `PermanentStorageUsed + size <= MaxPermanentStorageSize`
		pub fn can_renew(who: &T::AccountId, entry: &TransactionRef<BlockNumberFor<T>>) -> bool {
			let Ok(info) = Self::resolve_transaction_ref(entry) else { return false };
			if !Self::data_size_ok(info.size as usize) {
				return false;
			}
			let scope = AuthorizationScope::Account(who.clone());
			Self::prune_expired(&scope);
			let Some(auth) = Authorizations::<T>::get(&scope) else { return false };
			let relay_now = Self::relay_now();
			let size: u64 = info.size.into();
			let has_slot_capacity = auth
				.slots
				.iter()
				.any(|slot| slot.is_active(relay_now) && slot.extent.has_permanent_capacity(size));
			if !has_slot_capacity {
				return false;
			}
			PermanentStorageUsed::<T>::get().saturating_add(size) <=
				T::MaxPermanentStorageSize::get()
		}

		/// Returns `true` if `who` has at least one slot active at the current
		/// relay block (`starts_at <= relay_now < expiration`). Future-only
		/// slots (`starts_at > relay_now`) and expired slots both report
		/// `false`. Drained-but-active slots count — they can still serve
		/// low-priority `store()` calls.
		pub fn account_has_active_authorization(who: &T::AccountId) -> bool {
			let scope = AuthorizationScope::Account(who.clone());
			Self::prune_expired(&scope);
			let relay_now = Self::relay_now();
			Authorizations::<T>::get(&scope)
				.is_some_and(|auth| auth.slots.iter().any(|s| s.is_active(relay_now)))
		}

		/// Returns the (folded, active-only) authorization extent for the given
		/// content hash.
		pub fn preimage_authorization_extent(hash: ContentHash) -> AuthorizationExtent {
			Self::authorization_extent(AuthorizationScope::Preimage(hash))
		}

		/// Validate a signed TransactionStorage call.
		///
		/// Returns `(ValidTransaction, Some(scope))` for store/renew calls (origin should be
		/// transformed to carry authorization info).
		/// Returns `(ValidTransaction, None)` for authorizer calls (origin unchanged).
		/// Returns `Err(InvalidTransaction::Call)` for other calls.
		///
		/// This should be called from a `TransactionExtension` implementation.
		pub fn validate_signed(
			who: &T::AccountId,
			call: &Call<T>,
		) -> Result<(ValidTransaction, Option<AuthorizationScopeFor<T>>), TransactionValidityError>
		{
			let (valid_tx, scope) = Self::check_signed(who, call, CheckContext::Validate)?;
			Ok((valid_tx.ok_or(IMPOSSIBLE)?, scope))
		}

		/// Check the validity of the given call, signed by the given account, and consume
		/// authorization for it.
		///
		/// This is equivalent to `pre_dispatch` but for signed transactions. It should be called
		/// from a `TransactionExtension` implementation.
		pub fn pre_dispatch_signed(
			who: &T::AccountId,
			call: &Call<T>,
		) -> Result<(), TransactionValidityError> {
			let _ = Self::check_signed(who, call, CheckContext::PreDispatch)?;
			Ok(())
		}

		/// Get ByteFee storage information from the outside of this pallet.
		pub fn byte_fee() -> Option<BalanceOf<T>> {
			ByteFee::<T>::get()
		}

		/// Get EntryFee storage information from the outside of this pallet.
		pub fn entry_fee() -> Option<BalanceOf<T>> {
			EntryFee::<T>::get()
		}

		/// Get RetentionPeriod storage information from the outside of this pallet.
		pub fn retention_period() -> BlockNumberFor<T> {
			RetentionPeriod::<T>::get()
		}

		/// Whether `content_hash` is currently stored on-chain — i.e. some
		/// retained transaction in this pallet indexes it.
		///
		/// O(1): one [`TransactionByContentHash`] map read. The map's
		/// lifecycle matches the question's semantics — `store`/`renew`
		/// insert (or overwrite to the latest `(block, index)`), and
		/// `on_initialize` removes the entry when the block it points at
		/// ages out of [`RetentionPeriod`].
		pub fn contains_transaction(content_hash: ContentHash) -> bool {
			TransactionByContentHash::<T>::contains_key(content_hash)
		}

		/// Returns `true` if a blob of the given size can be stored.
		pub fn data_size_ok(size: usize) -> bool {
			(size > 0) && (size <= T::MaxTransactionSize::get() as usize)
		}

		/// Ensures that the given data size is valid for storage.
		fn ensure_data_size_ok(size: usize) -> Result<(), Error<T>> {
			ensure!(Self::data_size_ok(size), Error::<T>::BadDataSize);
			Ok(())
		}

		/// Returns the [`TransactionInfo`] for the specified store/renew transaction.
		pub(crate) fn transaction_info(
			block_number: BlockNumberFor<T>,
			index: u32,
		) -> Option<TransactionInfo> {
			let transactions = Transactions::<T>::get(block_number)?;
			transactions.into_iter().nth(index as usize)
		}

		/// Resolves a [`TransactionRef`] to its [`TransactionInfo`].
		fn resolve_transaction_ref(
			entry: &TransactionRef<BlockNumberFor<T>>,
		) -> Result<TransactionInfo, Error<T>> {
			let (block, index) = match entry {
				TransactionRef::Position { block, index } => (*block, *index),
				TransactionRef::ContentHash(hash) =>
					TransactionByContentHash::<T>::get(hash).ok_or(Error::<T>::RenewedNotFound)?,
			};
			Self::transaction_info(block, index).ok_or(Error::<T>::RenewedNotFound)
		}

		/// All transactions stored at the given block, in the current `TransactionInfo` layout.
		///
		/// Shape-tolerant against entries that are still in the pre-v3 layout.
		pub fn transactions_at(
			block: BlockNumberFor<T>,
		) -> Option<BoundedVec<TransactionInfo, T::MaxBlockTransactions>> {
			let raw = sp_io::storage::get(&Transactions::<T>::hashed_key_for(block))?;

			if let Ok(v3) =
				BoundedVec::<TransactionInfo, T::MaxBlockTransactions>::decode(&mut &raw[..])
			{
				return Some(v3);
			}

			let v2 = BoundedVec::<
				crate::migrations::v3::V2TransactionInfo,
				T::MaxBlockTransactions,
			>::decode(&mut &raw[..])
			.ok()?;

			let materialized: Vec<TransactionInfo> = v2
				.into_iter()
				.map(|tx| TransactionInfo {
					chunk_root: tx.chunk_root,
					content_hash: tx.content_hash,
					hashing: tx.hashing,
					cid_codec: tx.cid_codec,
					size: tx.size,
					extrinsic_index: u32::MAX,
					block_chunks: tx.block_chunks,
					kind: TransactionKind::Store,
				})
				.collect();

			BoundedVec::<TransactionInfo, T::MaxBlockTransactions>::try_from(materialized).ok()
		}

		/// Returns `true` if no more store/renew transactions can be included in the current
		/// block.
		pub fn block_transactions_full() -> bool {
			BlockTransactions::<T>::decode_len()
				.is_some_and(|len| len >= T::MaxBlockTransactions::get() as usize)
		}

		/// Find the index of the earliest-expiring **active** slot to consume.
		///
		/// - For `store` (`is_renew == false`): any active slot is acceptable. `bytes` and
		///   `transactions` are soft counters — they saturate upward and only drive the priority
		///   boost. Returns the first active slot in expiration order.
		/// - For `renew` (`is_renew == true`): the per-slot hard cap requires `bytes_permanent +
		///   size <= bytes_allowance`. Returns the first active slot satisfying this; no cross-slot
		///   subsidy. The chain-wide cap is checked separately by the caller.
		fn pick_slot_for_consumption(
			slots: &[TimedAuthorization],
			relay_now: u32,
			size: u64,
			is_renew: bool,
		) -> Option<usize> {
			// Slots are stored sorted by `expiration` ascending; first match is
			// earliest-expiring.
			slots.iter().position(|slot| {
				if !slot.is_active(relay_now) {
					return false;
				}
				if is_renew {
					// Per-slot hard cap on the renew axis only.
					return slot.extent.bytes_permanent.saturating_add(size) <=
						slot.extent.bytes_allowance;
				}
				true
			})
		}

		/// Check that authorization exists for data of the given size at the
		/// current relay block and (optionally) consume capacity from the
		/// earliest-expiring active slot.
		///
		/// `store` (`is_renew == false`) never gates on byte or transaction
		/// counters — they are soft and saturate upward. Rejects only when no
		/// active slot exists (`InvalidTransaction::Payment`).
		///
		/// `renew` (`is_renew == true`) is gated by two hard caps:
		///   * Per-slot: `bytes_permanent + size <= bytes_allowance`. If no active slot satisfies
		///     this, returns [`PERMANENT_ALLOWANCE_EXCEEDED`].
		///   * Chain-wide: `PermanentStorageUsed + size <= MaxPermanentStorageSize` — checked
		///     first, returns [`CHAIN_PERMANENT_CAP_REACHED`].
		///
		/// On `consume`: increments the chosen slot's `bytes` (store) or
		/// `bytes_permanent` (renew) by `size`, and `transactions` by 1, all
		/// saturating. The `transactions` counter is bumped on **every**
		/// consume — including low-priority (over-cap) stores and every renew
		/// — because it feeds the priority boost; consumption itself never
		/// gates on it. For renew, also bumps the chain-wide
		/// `PermanentStorageUsed` counter; the matching decrement happens in
		/// `on_initialize` when the obsolete `Transactions[block]` is removed.
		fn check_authorization(
			scope: &AuthorizationScopeFor<T>,
			size: u32,
			consume: bool,
			is_renew: bool,
		) -> Result<(), TransactionValidityError> {
			let relay_now = Self::relay_now();
			if relay_now == 0 {
				return Err(RELAY_CHAIN_TIME_UNAVAILABLE.into());
			}
			let chain_used = PermanentStorageUsed::<T>::get();
			let chain_cap = T::MaxPermanentStorageSize::get();
			let size_u64: u64 = size.into();

			// Lazy prune so the chooser sees the current view.
			Self::prune_expired(scope);

			// Chain-wide hard cap is independent of which slot is chosen — check
			// it once up-front so a renew can't even probe slot capacity past
			// the chain's permanent cap.
			if is_renew && chain_used.saturating_add(size_u64) > chain_cap {
				return Err(CHAIN_PERMANENT_CAP_REACHED.into());
			}

			let work = |maybe_auth: &mut Option<Authorization<T>>|
			 -> Result<(), TransactionValidityError> {
				let Some(auth) = maybe_auth.as_mut() else {
					return Err(InvalidTransaction::Payment.into());
				};

				let Some(idx) =
					Self::pick_slot_for_consumption(&auth.slots, relay_now, size_u64, is_renew)
				else {
					// Renew with at least one active slot but none with
					// `bytes_permanent + size <= bytes_allowance` ⇒ per-slot
					// hard cap. Otherwise (no active slot at all, or store
					// without any active slot) it's the generic missing-auth
					// case.
					if is_renew && auth.slots.iter().any(|s| s.is_active(relay_now)) {
						return Err(PERMANENT_ALLOWANCE_EXCEEDED.into());
					}
					return Err(InvalidTransaction::Payment.into());
				};

				if consume {
					let slot = &mut auth.slots[idx];
					if is_renew {
						slot.extent.bytes_permanent =
							slot.extent.bytes_permanent.saturating_add(size_u64);
					} else {
						slot.extent.bytes = slot.extent.bytes.saturating_add(size_u64);
					}
					slot.extent.transactions = slot.extent.transactions.saturating_add(1);
				}
				Ok(())
			};

			let result = if consume {
				Authorizations::<T>::mutate(scope, work)
			} else {
				let mut auth = Authorizations::<T>::get(scope);
				work(&mut auth)
			};

			// On a successful renew consume: bump the chain-wide counter.
			if result.is_ok() && consume && is_renew {
				Self::update_permanent_storage_used(|used| used.saturating_add(size_u64));
			}

			result
		}

		/// Check that authorization with the given scope exists in storage and
		/// every slot in the entry has expired. Mirrors the dispatch-time guard
		/// in [`remove_expired_authorization`] so that `remove_expired_*` calls
		/// are rejected at pool ingress when they cannot succeed.
		fn check_authorization_expired(
			scope: &AuthorizationScopeFor<T>,
		) -> Result<(), TransactionValidityError> {
			let Some(auth) = Authorizations::<T>::get(scope) else {
				return Err(AUTHORIZATION_NOT_FOUND.into());
			};
			if !auth.all_expired(Self::relay_now()) {
				return Err(AUTHORIZATION_NOT_EXPIRED.into());
			}
			Ok(())
		}

		fn preimage_store_renew_valid_transaction(content_hash: ContentHash) -> ValidTransaction {
			ValidTransaction::with_tag_prefix("TransactionStorageStoreRenew")
				.and_provides(content_hash)
				.priority(T::StoreRenewPriority::get())
				.longevity(T::StoreRenewLongevity::get())
				.into()
		}

		fn check_store_renew_unsigned(
			size: usize,
			content_hash: impl FnOnce() -> ContentHash,
			context: CheckContext,
			is_renew: bool,
		) -> Result<Option<ValidTransaction>, TransactionValidityError> {
			if !Self::data_size_ok(size) {
				return Err(BAD_DATA_SIZE.into());
			}

			if Self::block_transactions_full() {
				return Err(InvalidTransaction::ExhaustsResources.into());
			}

			let content_hash = content_hash();

			Self::check_authorization(
				&AuthorizationScope::Preimage(content_hash),
				size as u32,
				context.consume_authorization(),
				is_renew,
			)?;

			Ok(context
				.want_valid_transaction()
				.then(|| Self::preimage_store_renew_valid_transaction(content_hash)))
		}

		fn check_unsigned(
			call: &Call<T>,
			context: CheckContext,
		) -> Result<Option<ValidTransaction>, TransactionValidityError> {
			match call {
				Call::<T>::store { data } => Self::check_store_renew_unsigned(
					data.len(),
					|| sp_io::hashing::blake2_256(data),
					context,
					false,
				),
				Call::<T>::store_with_cid_config { cid, data } => Self::check_store_renew_unsigned(
					data.len(),
					|| cid.hashing.hash(data),
					context,
					false,
				),
				Call::<T>::force_renew { entry } => {
					let info =
						Self::resolve_transaction_ref(entry).map_err(|_| RENEWED_NOT_FOUND)?;
					Self::check_store_renew_unsigned(
						info.size as usize,
						|| info.content_hash,
						context,
						true,
					)
				},
				// `renew` (one-shot scheduler) is signed-only — needs `who` to record in
				// `AutoRenewals`. Reject unsigned/preimage submissions.
				Call::<T>::renew { .. } => Err(InvalidTransaction::Call.into()),
				Call::<T>::remove_expired_account_authorization { who } => {
					Self::check_authorization_expired(&AuthorizationScope::Account(who.clone()))?;
					Ok(context.want_valid_transaction().then(|| {
						ValidTransaction::with_tag_prefix(
							"TransactionStorageRemoveExpiredAccountAuthorization",
						)
						.and_provides(who)
						.priority(T::RemoveExpiredAuthorizationPriority::get())
						.longevity(T::RemoveExpiredAuthorizationLongevity::get())
						.into()
					}))
				},
				Call::<T>::remove_expired_preimage_authorization { content_hash } => {
					Self::check_authorization_expired(&AuthorizationScope::Preimage(
						*content_hash,
					))?;
					Ok(context.want_valid_transaction().then(|| {
						ValidTransaction::with_tag_prefix(
							"TransactionStorageRemoveExpiredPreimageAuthorization",
						)
						.and_provides(content_hash)
						.priority(T::RemoveExpiredAuthorizationPriority::get())
						.longevity(T::RemoveExpiredAuthorizationLongevity::get())
						.into()
					}))
				},
				Call::<T>::remove_exhausted_authorizer { who } => {
					let budget = AllowedAuthorizers::<T>::get(who).ok_or(AUTHORIZER_NOT_FOUND)?;
					ensure!(Self::authorizer_removable(&budget), AUTHORIZATION_NOT_EXHAUSTED);
					Ok(context.want_valid_transaction().then(|| {
						ValidTransaction::with_tag_prefix(
							"TransactionStorageRemoveExhaustedAuthorizer",
						)
						.and_provides(who)
						.priority(T::RemoveExpiredAuthorizationPriority::get())
						.longevity(T::RemoveExpiredAuthorizationLongevity::get())
						.into()
					}))
				},
				// Mandatory inherent — always allowed, no pool validation needed.
				Call::<T>::apply_block_inherents { .. } => Ok(None),
				_ => Err(InvalidTransaction::Call.into()),
			}
		}

		fn check_signed(
			who: &T::AccountId,
			call: &Call<T>,
			context: CheckContext,
		) -> Result<
			(Option<ValidTransaction>, Option<AuthorizationScopeFor<T>>),
			TransactionValidityError,
		> {
			let (size, content_hash, is_renew) = match call {
				Call::<T>::store { data } => {
					let content_hash = sp_io::hashing::blake2_256(data);
					(data.len(), content_hash, false)
				},
				Call::<T>::store_with_cid_config { cid, data } => {
					let content_hash = cid.hashing.hash(data);
					(data.len(), content_hash, false)
				},
				Call::<T>::force_renew { entry } => {
					let info =
						Self::resolve_transaction_ref(entry).map_err(|_| RENEWED_NOT_FOUND)?;
					(info.size as usize, info.content_hash, true)
				},
				Call::<T>::authorize_account { .. } |
				Call::<T>::authorize_preimage { .. } |
				Call::<T>::authorize_account_window { .. } |
				Call::<T>::authorize_preimage_window { .. } => {
					// Verify that the signer satisfies the Authorizer origin. Budget
					// consumption (for `AllowedAuthorizers` signers on `authorize_*`)
					// happens inside the dispatch body, not here.
					let origin = frame_system::RawOrigin::Signed(who.clone()).into();
					T::Authorizer::ensure_origin(origin)
						.map_err(|_| InvalidTransaction::BadSigner)?;
					return Ok((
						context.want_valid_transaction().then(|| ValidTransaction {
							priority: T::StoreRenewPriority::get(),
							longevity: T::StoreRenewLongevity::get(),
							..Default::default()
						}),
						None,
					));
				},
				Call::<T>::add_authorizer { .. } | Call::<T>::remove_authorizer { .. } => {
					// `AuthorizerRegistrarOrigin` is enforced at dispatch; pool validation is a
					// passthrough.
					return Ok((
						context.want_valid_transaction().then(ValidTransaction::default),
						None,
					));
				},
				Call::<T>::renew { entry } => {
					// Pre-paid one-shot: charges the same as `force_renew`. Cycle delivers
					// without re-charging (see `do_process_auto_renewals`). Reject
					// duplicates before charging — mirrors `enable_auto_renew` below.
					let info =
						Self::resolve_transaction_ref(entry).map_err(|_| RENEWED_NOT_FOUND)?;
					if AutoRenewals::<T>::contains_key(info.content_hash) {
						return Err(AUTO_RENEWAL_ALREADY_ENABLED.into());
					}
					Self::check_authorization(
						&AuthorizationScope::Account(who.clone()),
						info.size,
						context.consume_authorization(),
						true,
					)?;
					let scope = AuthorizationScope::Account(who.clone());
					return Ok((
						context.want_valid_transaction().then(|| {
							ValidTransaction::with_tag_prefix("TransactionStorageRenew")
								.and_provides((who.clone(), info.content_hash))
								.priority(T::StoreRenewPriority::get())
								.longevity(T::StoreRenewLongevity::get())
								.into()
						}),
						Some(scope),
					));
				},
				Call::<T>::enable_auto_renew { content_hash } => {
					// Pre-paid recurring registration. Mirrors one-shot `renew`'s
					// hard-cap charge: `bytes_permanent`, the chain-wide
					// `PermanentStorageUsed` counter, and one tx slot are all consumed
					// here. The first cycle then fires free in
					// `do_process_auto_renewals` (`paid = true` on the inserted
					// `RenewalData`); subsequent cycles charge per-cycle.
					if AutoRenewals::<T>::contains_key(*content_hash) {
						return Err(AUTO_RENEWAL_ALREADY_ENABLED.into());
					}
					let (block, index) = TransactionByContentHash::<T>::get(*content_hash)
						.ok_or(RENEWED_NOT_FOUND)?;
					let info = Self::transaction_info(block, index).ok_or(RENEWED_NOT_FOUND)?;

					Self::check_authorization(
						&AuthorizationScope::Account(who.clone()),
						info.size,
						context.consume_authorization(),
						true,
					)?;

					let scope = AuthorizationScope::Account(who.clone());
					return Ok((
						context.want_valid_transaction().then(|| {
							ValidTransaction::with_tag_prefix("TransactionStorageRenew")
								.and_provides((who.clone(), info.content_hash))
								.priority(T::StoreRenewPriority::get())
								.longevity(T::StoreRenewLongevity::get())
								.into()
						}),
						Some(scope),
					));
				},
				Call::<T>::disable_auto_renew { content_hash } => {
					// Feeless. Pool admission is gated on ownership AND on the prepaid
					// window — the registration must exist, `who` must be its owner,
					// and the next cycle must not be pre-paid (otherwise the owner
					// would be reclaiming a slot they've already charged against their
					// quota). `Some(scope)` triggers the origin rewrite to
					// `Origin::Authorized` expected by the dispatch's `ensure_authorized`.
					let renewal_data =
						AutoRenewals::<T>::get(content_hash).ok_or(AUTO_RENEWAL_NOT_ENABLED)?;
					if &renewal_data.account != who {
						return Err(NOT_AUTO_RENEWAL_OWNER.into());
					}
					if renewal_data.paid {
						return Err(CANNOT_DISABLE_PREPAID_AUTO_RENEWAL.into());
					}
					let scope = AuthorizationScope::Account(who.clone());
					return Ok((
						context.want_valid_transaction().then(|| ValidTransaction {
							priority: T::StoreRenewPriority::get(),
							longevity: T::StoreRenewLongevity::get(),
							..Default::default()
						}),
						Some(scope),
					));
				},
				_ => return Err(InvalidTransaction::Call.into()),
			};

			if !Self::data_size_ok(size) {
				return Err(BAD_DATA_SIZE.into());
			}

			if Self::block_transactions_full() {
				return Err(InvalidTransaction::ExhaustsResources.into());
			}

			// Prefer preimage authorization if available.
			// This allows anyone to store/renew pre-authorized content without consuming their
			// own account authorization.
			let consume = context.consume_authorization();

			let used_preimage_auth = Self::check_authorization(
				&AuthorizationScope::Preimage(content_hash),
				size as u32,
				consume,
				is_renew,
			)
			.is_ok();

			if !used_preimage_auth {
				Self::check_authorization(
					&AuthorizationScope::Account(who.clone()),
					size as u32,
					consume,
					is_renew,
				)?;
			}

			// Only build `ValidTransaction` metadata during pool validation, not block
			// execution. The tx tag/priority differs depending on whether preimage or account
			// authorization was used.
			let (valid_tx, scope) = if context.want_valid_transaction() {
				let (valid_tx, scope) = if used_preimage_auth {
					(
						Self::preimage_store_renew_valid_transaction(content_hash),
						AuthorizationScope::Preimage(content_hash),
					)
				} else {
					// Tag prefix differs per family so store and renew operations don't
					// dedup against each other in the pool.
					let prefix = if is_renew {
						"TransactionStorageRenew"
					} else {
						"TransactionStorageStore"
					};
					(
						ValidTransaction::with_tag_prefix(prefix)
							.and_provides((who, content_hash))
							.priority(T::StoreRenewPriority::get())
							.longevity(T::StoreRenewLongevity::get())
							.into(),
						AuthorizationScope::Account(who.clone()),
					)
				};
				(Some(valid_tx), Some(scope))
			} else {
				(None, None)
			};

			Ok((valid_tx, scope))
		}

		/// Verifies that the provided proof corresponds to a randomly selected chunk from a list of
		/// transactions.
		pub(crate) fn verify_chunk_proof(
			proof: TransactionStorageProof,
			random_hash: &[u8],
			infos: Vec<TransactionInfo>,
		) -> Result<(), Error<T>> {
			// Get the random chunk index - from all transactions in the block = [0..total_chunks).
			let total_chunks: ChunkIndex = TransactionInfo::total_chunks(&infos);
			ensure!(total_chunks != 0, Error::<T>::UnexpectedProof);
			let selected_block_chunk_index = random_chunk(random_hash, total_chunks);

			// Let's find the corresponding transaction and its "local" chunk index for "global"
			// `selected_block_chunk_index`.
			let (tx_info, tx_chunk_index) = {
				// Binary search for the transaction that owns this `selected_block_chunk_index`
				// chunk.
				let tx_index = infos
					.binary_search_by_key(&selected_block_chunk_index, |info| {
						// Each `info.block_chunks` is cumulative count,
						// so last chunk index = count - 1.
						info.block_chunks.saturating_sub(1)
					})
					.unwrap_or_else(|tx_index| tx_index);

				// Get the transaction and its local chunk index.
				let tx_info = infos.get(tx_index).ok_or(Error::<T>::MissingStateData)?;
				// We shouldn't reach this point; we rely on the fact that `fn store` does not allow
				// empty transactions. Without this check, it would fail anyway below with
				// `InvalidProof`.
				ensure!(!tx_info.block_chunks.is_zero(), Error::<T>::BadDataSize);

				// Convert a global chunk index into a transaction-local one.
				let tx_chunks = num_chunks(tx_info.size);
				let prev_chunks = tx_info.block_chunks - tx_chunks;
				let tx_chunk_index = selected_block_chunk_index - prev_chunks;

				(tx_info, tx_chunk_index)
			};

			// Verify the tx chunk proof.
			ensure!(
				sp_io::trie::blake2_256_verify_proof(
					tx_info.chunk_root,
					&proof.proof,
					&encode_index(tx_chunk_index),
					&proof.chunk,
					sp_runtime::StateVersion::V1,
				),
				Error::<T>::InvalidProof
			);

			Ok(())
		}

		/// `true` if the authorizer entry is eligible for permissionless cleanup —
		/// either its budget is zero on at least one axis, or its `valid_until` has
		/// elapsed.
		fn authorizer_removable(budget: &AuthorizerBudgetFor<T>) -> bool {
			budget.is_exhausted() || budget.is_expired(Self::now())
		}

		/// Atomically decrement `who`'s [`AllowedAuthorizers`] budget by
		/// `transactions` / `bytes`. Returns [`Error::InsufficientAuthorizerBudget`]
		/// when either axis would go negative; on failure the budget is unchanged.
		///
		/// A missing entry (Root/XCM origins not in [`AllowedAuthorizers`]) is a no-op:
		/// they have no budget to track. Callers should invoke this *after*
		/// [`Config::Authorizer::ensure_origin`] to ensure unrelated signers can't
		/// trigger arbitrary budget reads.
		fn consume_authorizer_budget(
			who: &T::AccountId,
			transactions: u32,
			bytes: u64,
		) -> DispatchResult {
			AllowedAuthorizers::<T>::try_mutate(who, |maybe_budget| {
				let Some(budget) = maybe_budget else { return Ok(()) };
				budget
					.try_consume(transactions, bytes)
					.ok_or(Error::<T>::InsufficientAuthorizerBudget.into())
			})
		}
	}
}

pub mod extension;

#[cfg(any(test, feature = "try-runtime"))]
impl<T: Config> Pallet<T> {
	pub(crate) fn do_try_state(n: BlockNumberFor<T>) -> Result<(), sp_runtime::TryRuntimeError> {
		ensure!(!Self::retention_period().is_zero(), "RetentionPeriod must not be zero");
		Self::check_transactions_integrity()?;
		Self::check_no_stale_transactions(n)?;
		Self::check_authorizations_integrity()?;
		Self::check_permanent_storage_accounting(n)?;
		Ok(())
	}

	/// Verify that for each block's transaction list:
	/// - The `block_chunks` field is cumulative: each entry equals the previous cumulative total
	///   plus `num_chunks(size)`.
	fn check_transactions_integrity() -> Result<(), sp_runtime::TryRuntimeError> {
		for (_block, transactions) in Transactions::<T>::iter() {
			let mut cumulative_chunks: ChunkIndex = 0;
			for tx in transactions.iter() {
				let expected_chunks = num_chunks(tx.size);
				cumulative_chunks = cumulative_chunks.saturating_add(expected_chunks);
				ensure!(tx.block_chunks == cumulative_chunks, "tx.block_chunks is not cumulative");
			}

			// The last entry's block_chunks should equal total_chunks for the block.
			let total = TransactionInfo::total_chunks(&transactions);
			ensure!(
				total == cumulative_chunks,
				"total_chunks mismatch with cumulative block_chunks"
			);
		}

		Ok(())
	}

	/// Verify that no `Transactions` entries exist for blocks older than
	/// `current_block - retention_period`. These should have been cleaned up
	/// by `on_initialize`.
	fn check_no_stale_transactions(
		n: BlockNumberFor<T>,
	) -> Result<(), sp_runtime::TryRuntimeError> {
		let period = Self::retention_period();
		let oldest_valid = n.saturating_sub(period);

		for (block, _) in Transactions::<T>::iter() {
			ensure!(block >= oldest_valid, "Stale transaction entry found beyond retention period");
			ensure!(block <= n, "Transaction entry exists for a future block");
		}

		Ok(())
	}

	/// Verify slot-storage invariants:
	/// - every entry has a non-empty bounded vec (empty vecs should have been removed by lazy
	///   maintenance);
	/// - every slot has `expiration > starts_at` and `bytes_allowance > 0`;
	/// - slots are sorted by `expiration` ascending (tiebreak `starts_at`).
	fn check_authorizations_integrity() -> Result<(), sp_runtime::TryRuntimeError> {
		for (_, auth) in Authorizations::<T>::iter() {
			ensure!(!auth.slots.is_empty(), "Authorizations entry has empty slots");
			let mut prev: Option<(u32, u32)> = None;
			for slot in auth.slots.iter() {
				ensure!(
					slot.expiration > slot.starts_at,
					"Stored slot has expiration <= starts_at"
				);
				ensure!(slot.extent.bytes_allowance > 0, "Stored slot has zero bytes_allowance");
				if let Some((p_exp, p_starts)) = prev {
					let ok = (p_exp < slot.expiration) ||
						(p_exp == slot.expiration && p_starts <= slot.starts_at);
					ensure!(ok, "Slots not sorted by (expiration ASC, starts_at ASC)");
				}
				prev = Some((slot.expiration, slot.starts_at));
			}
		}

		Ok(())
	}

	/// Verify the chain-wide permanent-storage accounting invariants:
	/// - `PermanentStorageUsed == Σ Renew sizes in Transactions + Σ paid AutoRenewals sizes` — the
	///   paid term covers the prepayment window between `renew` / `enable_auto_renew` charging the
	///   counter and `do_process_auto_renewals` writing the `Renew` entry.
	/// - `PermanentStorageUsed <= MaxPermanentStorageSize`.
	fn check_permanent_storage_accounting(
		_n: BlockNumberFor<T>,
	) -> Result<(), sp_runtime::TryRuntimeError> {
		let used = PermanentStorageUsed::<T>::get();

		let renewed_sum: u64 = Transactions::<T>::iter().fold(0u64, |acc, (_, entries)| {
			entries
				.iter()
				.filter(|t| matches!(t.kind, TransactionKind::Renew))
				.fold(acc, |inner, t| inner.saturating_add(t.size as u64))
		});
		let prepaid_sum: u64 =
			AutoRenewals::<T>::iter()
				.filter(|(_, data)| data.paid)
				.fold(0u64, |acc, (hash, _)| {
					let size = TransactionByContentHash::<T>::get(hash)
						.and_then(|(block, index)| Self::transaction_info(block, index))
						.map_or(0, |info| info.size as u64);
					acc.saturating_add(size)
				});
		ensure!(
			renewed_sum.saturating_add(prepaid_sum) == used,
			"PermanentStorageUsed != Σ renewed sizes + Σ paid auto-renewal sizes",
		);

		ensure!(
			used <= T::MaxPermanentStorageSize::get(),
			"PermanentStorageUsed exceeds MaxPermanentStorageSize",
		);

		Ok(())
	}
}

/// Sanity-check that the runtime's weight/size configuration is consistent with
/// `MaxBlockTransactions` and `MaxTransactionSize`.
///
/// Verifies that the runtime's weight configuration, block length limits, and
/// `MaxBlockTransactions`/`MaxTransactionSize` constants are mutually consistent.
///
/// The available block weight accounts for:
/// - The `avg_block_initialization` margin that FRAME reserves from `max_total` for on_initialize
///   hooks (e.g. 5% for parachains, 10% for `with_sensible_defaults`).
/// - For parachains, the collator-side PoV cap: collators limit the actual PoV to a percentage of
///   `max_pov_size` to leave headroom for relay-chain state proof overhead. See
///   `cumulus/client/consensus/aura/src/collators/slot_based/block_builder_task.rs`.
///
/// # Parameters
///
/// - `collator_pov_percent`: for parachains, the collator-side PoV cap (e.g. `Some(85)`).
///   Solochains should pass `None`.
///
/// # Panics
///
/// Panics with a descriptive message if any check fails.
#[cfg(any(test, feature = "std"))]
pub fn ensure_weight_sanity<T: Config>(collator_pov_percent: Option<u64>) {
	use frame_support::{dispatch::DispatchClass, weights::Weight};

	let block_weights = <T as frame_system::Config>::BlockWeights::get();
	let normal_length =
		*<T as frame_system::Config>::BlockLength::get().max.get(DispatchClass::Normal);

	let max_block_txs = T::MaxBlockTransactions::get();
	let max_tx_size = T::MaxTransactionSize::get();

	let normal = block_weights.get(DispatchClass::Normal);
	let normal_max_total = normal.max_total.expect("Normal class must have a max_total weight");
	let base_extrinsic = normal.base_extrinsic;
	let max_extrinsic =
		normal.max_extrinsic.expect("Normal class must have a max_extrinsic weight");

	// init_weight = max_total - max_extrinsic - base_extrinsic (the avg_block_initialization
	// reservation that FRAME sets aside for on_initialize hooks).
	let init_weight = normal_max_total.saturating_sub(max_extrinsic).saturating_sub(base_extrinsic);

	let after_init = normal_max_total.saturating_sub(init_weight);
	let effective_normal = if let Some(pov_percent) = collator_pov_percent {
		// Collators cap the PoV to reserve headroom for the relay-chain state proof.
		// Reference: cumulus/client/consensus/aura/src/collators/lookahead.rs
		let pov_limit = block_weights.max_block.proof_size() * pov_percent / 100;
		Weight::from_parts(after_init.ref_time(), after_init.proof_size().min(pov_limit))
	} else {
		after_init
	};

	// 1. MaxTransactionSize must fit within the normal block length limit.
	assert!(
		max_tx_size < normal_length,
		"MaxTransactionSize ({max_tx_size}) >= normal block length ({normal_length}): \
		 a single max-size store extrinsic wouldn't fit by length",
	);

	// 2. A single store(MaxTransactionSize) must fit within max_extrinsic.
	let max_store_dispatch = T::WeightInfo::store(max_tx_size);
	assert!(
		max_store_dispatch.all_lte(max_extrinsic),
		"store({max_tx_size}) dispatch weight {max_store_dispatch:?} exceeds \
		 max_extrinsic {max_extrinsic:?} (which accounts for init overhead + base)",
	);

	// 3. MaxBlockTransactions store calls at an evenly-split size must fit in the effective normal
	//    budget (ref_time). Each extrinsic costs dispatch + base.
	let per_tx_size = normal_length / max_block_txs;
	let store_weight = T::WeightInfo::store(per_tx_size).saturating_add(base_extrinsic);
	let total_store_ref_time = store_weight.ref_time().saturating_mul(max_block_txs as u64);
	assert!(
		total_store_ref_time <= effective_normal.ref_time(),
		"MaxBlockTransactions ({max_block_txs}) store calls at {per_tx_size} bytes each: \
		 total ref_time {total_store_ref_time} exceeds effective normal limit {} \
		 (max_total {} minus init reservation {})",
		effective_normal.ref_time(),
		normal_max_total.ref_time(),
		init_weight.ref_time(),
	);

	// 4. MaxBlockTransactions renew calls must fit by ref_time.
	let renew_weight = T::WeightInfo::renew().saturating_add(base_extrinsic);
	let total_renew_ref_time = renew_weight.ref_time().saturating_mul(max_block_txs as u64);
	assert!(
		total_renew_ref_time <= effective_normal.ref_time(),
		"MaxBlockTransactions ({max_block_txs}) renew calls: \
		 total ref_time {total_renew_ref_time} exceeds effective normal limit {}",
		effective_normal.ref_time(),
	);

	// 5. apply_block_inherents (DispatchClass::Mandatory, once per block) must fit
	// in max block at worst case (proof check + draining MaxBlockTransactions
	// auto-renewals).
	let apply_inherents_weight = T::WeightInfo::apply_block_inherents(max_block_txs);
	assert!(
		apply_inherents_weight.all_lte(block_weights.max_block),
		"apply_block_inherents weight {apply_inherents_weight:?} exceeds max block {:?}",
		block_weights.max_block,
	);

	// 6. on_initialize at the worst-case expiry block (taking
	// `MaxBlockTransactions` items out of `Transactions[obsolete]` and looking up
	// `TransactionByContentHash` + `AutoRenewals` for each) must fit alongside
	// `apply_block_inherents` within `max_block`. Both run on the same block in
	// the mandatory class; their sum is the floor of the mandatory budget for
	// that block.
	let on_init_with_expiry_weight = T::WeightInfo::on_initialize_with_expiry(max_block_txs);
	let mandatory_floor = on_init_with_expiry_weight.saturating_add(apply_inherents_weight);
	assert!(
		mandatory_floor.all_lte(block_weights.max_block),
		"on_initialize_with_expiry({max_block_txs}) + apply_block_inherents({max_block_txs}) \
		 = {mandatory_floor:?} exceeds max block {:?}",
		block_weights.max_block,
	);

	// Diagnostics (visible with --nocapture).
	let max_txs_by_weight = effective_normal.ref_time() / store_weight.ref_time();
	println!("--- transaction_storage weight sanity ---");
	println!("  MaxBlockTransactions:       {max_block_txs}");
	println!(
		"  MaxTransactionSize:         {max_tx_size} bytes ({} MiB)",
		max_tx_size / (1024 * 1024)
	);
	println!("  Normal max_total:           {normal_max_total:?}");
	println!("  Init reservation:           {init_weight:?}");
	if let Some(pov_percent) = collator_pov_percent {
		let pov_limit = block_weights.max_block.proof_size() * pov_percent / 100;
		println!(
			"  Collator PoV cap ({pov_percent}%):      {pov_limit} bytes ({:.1} MiB)",
			pov_limit as f64 / (1024.0 * 1024.0)
		);
	}
	println!("  Effective normal budget:    {effective_normal:?}");
	println!("  max_extrinsic:              {max_extrinsic:?}");
	println!(
		"  Normal length limit:        {normal_length} bytes ({} MiB)",
		normal_length / (1024 * 1024)
	);
	println!("  store(max_size) weight:     {max_store_dispatch:?}");
	println!("  store(even_split) weight:   {store_weight:?} (at {per_tx_size} bytes)");
	println!("  renew weight:               {renew_weight:?}");
	println!("  block_weights.max_block:    {:?}", block_weights.max_block);
	println!("  apply_block_inherents wt:   {apply_inherents_weight:?}");
	println!("  on_initialize_with_expiry:  {on_init_with_expiry_weight:?}");
	println!("  Mandatory floor (sum):      {mandatory_floor:?}");
	println!("  Max store txs by weight:    {max_txs_by_weight}");
	println!("  Max store txs by length:    {}", normal_length / per_tx_size);
}
