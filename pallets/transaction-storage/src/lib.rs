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
use codec::Decode;
use core::fmt::Debug;
use polkadot_sdk_frame::{
	deps::*,
	prelude::*,
	traits::{
		fungible::{hold::Balanced, Mutate, MutateHold},
		parameter_types, OriginTrait,
	},
};
use sp_runtime::traits::BlockNumberProvider;
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
/// Authorization was not found.
pub const AUTHORIZATION_NOT_FOUND: InvalidTransaction = InvalidTransaction::Custom(3);
/// Authorization has not expired.
pub const AUTHORIZATION_NOT_EXPIRED: InvalidTransaction = InvalidTransaction::Custom(4);
/// Authorizer account was not found.
pub const AUTHORIZER_NOT_FOUND: InvalidTransaction = InvalidTransaction::Custom(7);
/// Authorizer budget has not been exhausted.
pub const AUTHORIZATION_NOT_EXHAUSTED: InvalidTransaction = InvalidTransaction::Custom(8);
/// Relay-chain block number not yet available (genesis sentinel `0`).
pub const RELAY_CHAIN_TIME_UNAVAILABLE: InvalidTransaction = InvalidTransaction::Custom(13);
// NOTE: `Custom` codes 2, 5, 6 and 9-12 belong to
// `pallet-bulletin-data-renewal` (shared `u8` namespace); do not reuse.

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
		/// Slot window length, in relay blocks, for `authorize_account` /
		/// `authorize_preimage` and for `refresh_*`.
		#[pallet::constant]
		type DefaultAuthorizationWindow: Get<RelayBlockNumberFor<Self>>;
		/// How far ahead of `relay_now` a `_window` call may schedule a slot to start.
		#[pallet::constant]
		type MaxStartsAtFuture: Get<RelayBlockNumberFor<Self>>;
		/// Maximum number of slots stored per scope.
		#[pallet::constant]
		type MaxAuthorizationSlots: Get<u32>;
		/// Relay-chain clock for slot windows. On parachains this is
		/// `cumulus_pallet_parachain_system::RelaychainDataProvider`.
		type RelayChainBlockNumberProvider: BlockNumberProvider;
		/// The origin that manages the authorizer list.
		type AuthorizerRegistrarOrigin: EnsureOrigin<Self::RuntimeOrigin>;
		/// The origin that can authorize data storage. `Success` is
		/// `Some(AuthorizationOrigin { .. })` when the dispatcher is an
		/// [`AllowedAuthorizers`] entry — carrying the budget owner and the
		/// authorizer's `valid_until` and `feeless` flag. `None` for Root / XCM /
		/// other non-account authorizers, which have no budget.
		type Authorizer: EnsureOrigin<
			Self::RuntimeOrigin,
			Success = Option<AuthorizationOrigin<Self::AccountId, BlockNumberFor<Self>>>,
		>;
		/// Pool params for signed and preimage-authorized `store`. One prefix is safe: they
		/// tag on `(who, content_hash)` and `content_hash` respectively.
		#[pallet::constant]
		type StoreTxParams: Get<ValidTransactionParams>;
		/// Pool params for `authorize_*` and `refresh_*`, which validate untagged.
		#[pallet::constant]
		type AuthorizeTxParams: Get<ValidTransactionParams>;
		/// Pool params for `remove_expired_account_authorization`. The three cleanup calls get
		/// their own items because two provide `who` and a `ContentHash` encodes like an
		/// `AccountId32`, so they need distinct prefixes despite shared pricing.
		#[pallet::constant]
		type RemoveExpiredAccountAuthorizationTxParams: Get<ValidTransactionParams>;
		/// Pool params for `remove_expired_preimage_authorization`.
		#[pallet::constant]
		type RemoveExpiredPreimageAuthorizationTxParams: Get<ValidTransactionParams>;
		/// Pool params for `remove_exhausted_authorizer`.
		#[pallet::constant]
		type RemoveExhaustedAuthorizerTxParams: Get<ValidTransactionParams>;
		/// Opaque per-entry payload on [`TransactionInfo`], never interpreted here:
		/// `store` writes `Default::default()`, expiry hands it back through
		/// [`Config::OnObsoleteTransactions`]. Wire `()` or the primitives'
		/// [`EntryKind`](bulletin_transaction_storage_primitives::EntryKind).
		type EntryMeta: Member + codec::FullCodec + MaxEncodedLen + scale_info::TypeInfo + Default;
		/// Opaque per-authorization payload inside [`AuthorizationExtent`], never
		/// interpreted here: reset with the other counters, mutated only through
		/// [`Pallet::try_mutate_active_authorization`]. Wire `()` or the renewal
		/// pallet's `PermanentExtent`.
		type AuthorizationExtra: Member
			+ codec::FullCodec
			+ MaxEncodedLen
			+ scale_info::TypeInfo
			+ Default;
		/// Callback invoked from `on_initialize` with the entries being aged out by the
		/// obsolete-block sweep. Default `()` for runtimes that omit the renewal pallet;
		/// wire to `pallet-bulletin-data-renewal::Pallet<Runtime>` to enable manual
		/// / auto-renewal.
		type OnObsoleteTransactions: crate::OnObsoleteTransactions<
			BlockNumberFor<Self>,
			Self::EntryMeta,
		>;
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
		/// Content hash was not calculated.
		InvalidContentHash,
		/// Authorizer account was not found.
		AuthorizerNotFound,
		/// Authorizer is not eligible for permissionless removal — it still has budget on both
		/// axes AND (if `valid_until` is set) has not yet expired.
		AuthorizerBudgetNotExhausted,
		/// `valid_until` supplied to `add_authorizer` is in the past (`<= now`, would
		/// expire immediately). Pass `None` for no expiration.
		InvalidValidUntil,
		/// `authorize_account` / `authorize_preimage` called by a signer whose
		/// `AllowedAuthorizers` budget cannot cover the requested
		/// `transactions` / `bytes` (or `max_size`).
		InsufficientAuthorizerBudget,
		/// Adding a slot would exceed [`Config::MaxAuthorizationSlots`] (after the lazy prune).
		TooManySlots,
		/// `(starts_at, expiration)` is not a valid window: `expiration <= starts_at`,
		/// `expiration <= relay_now`, or `starts_at` is more than
		/// [`Config::MaxStartsAtFuture`] blocks ahead. A past `starts_at` is accepted.
		InvalidWindow,
		/// The relay-chain block number is unavailable (genesis sentinel `0`).
		RelayChainTimeUnavailable,
	}

	const STORAGE_VERSION: StorageVersion = StorageVersion::new(6);

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	/// Custom origin for authorized signed transaction storage operations.
	///
	/// This origin is set by the [`extension::ValidateAuthorizedCalls`] transaction extension
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
		/// cleans up `TransactionByContentHash`, and hands the swept entries to
		/// [`Config::OnObsoleteTransactions`].
		///
		/// Weight is charged via the [`WeightInfo::on_initialize_with_expiry`] benchmark.
		/// The fit within `max_block` is asserted by [`ensure_weight_sanity`] — every
		/// runtime should exercise it from a test.
		fn on_initialize(n: BlockNumberFor<T>) -> Weight {
			let mut weight = Weight::zero();

			// Drop obsolete roots. The proof for `obsolete` will be checked later in this
			// block, so we drop `obsolete` - 1.
			let period = Self::retention_period();
			let obsolete = n.saturating_sub(period.saturating_add(One::one()));
			let mut num_expiring: u32 = 0;
			if obsolete > Zero::zero() {
				if let Some(transactions) = <Transactions<T>>::take(obsolete) {
					num_expiring = transactions.len() as u32;

					// Single pass: compute is_latest per entry, clean up
					// `TransactionByContentHash`, and collect the `(info, is_latest)`
					// pairs into a bounded vec for the renewal-pallet callback.
					let mut pairs: BoundedVec<
						(TransactionInfoFor<T>, bool),
						T::MaxBlockTransactions,
					> = BoundedVec::new();
					for tx_info in transactions.into_iter() {
						let hash: ContentHash = tx_info.content_hash;

						// `is_latest` discriminates the most-recent reference for `hash`
						// from stale shadows left by a later store/renew that moved
						// `TransactionByContentHash` forward.
						let is_latest = TransactionByContentHash::<T>::get(hash)
							.is_some_and(|(block, _)| block == obsolete);
						if is_latest {
							TransactionByContentHash::<T>::remove(hash);
						}
						let _ = pairs.try_push((tx_info, is_latest));
					}

					// Hand the swept items to the renewal pallet (the only trait seam
					// in the storage/renewal split). It decrements the chain-wide
					// renewed-byte counter for aged-out permanent entries. Wired to `()`
					// for runtimes that omit
					// `pallet-bulletin-data-renewal`; in that case this is
					// a no-op.
					T::OnObsoleteTransactions::handle_obsolete(obsolete, &pairs);
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

			Self::seal_block(n);
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
			Self::assert_pool_families_distinct(&[]);
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

		// Retired call_index(1): renew(origin, entry: TransactionRef<BlockNumberFor<T>>)
		//   — moved to pallet-bulletin-data-renewal. Do not reuse this index.
		// Retired call_index(2): force_renew(origin, entry: TransactionRef<BlockNumberFor<T>>)
		//   — moved to pallet-bulletin-data-renewal. Do not reuse this index.

		/// Index and store data off chain with an explicit CID configuration.
		///
		/// Behaves identically to [`store`](Self::store), but the CID configuration
		/// (codec and hashing algorithm) is passed directly as a parameter.
		///
		/// Emits [`Stored`](Event::Stored) when successful.
		#[pallet::call_index(9)]
		#[pallet::weight(T::WeightInfo::store_with_cid_config(data.len() as u32))]
		#[pallet::feeless_if(|_origin: &OriginFor<T>, _cid: &CidConfig, _data: &Vec<u8>| -> bool { true })]
		pub fn store_with_cid_config(
			origin: OriginFor<T>,
			cid: CidConfig,
			data: Vec<u8>,
		) -> DispatchResult {
			let _caller = Self::ensure_authorized(origin)?;
			Self::do_store(data, cid.hashing, cid.codec)
		}

		// Retired call_index(10): renew_content_hash(origin, content_hash: ContentHash)
		//   — unified into `renew` via `TransactionRef`. Do not reuse this index.
		// Retired call_index(11): process_auto_renewals(origin)
		//   — folded into `on_initialize` of pallet-bulletin-data-renewal. Do not
		// reuse this index. Retired call_index(12): enable_auto_renew(origin, content_hash:
		// ContentHash)   — moved to pallet-bulletin-data-renewal. Do not reuse
		// this index. Retired call_index(13): disable_auto_renew(origin, content_hash:
		// ContentHash)   — moved to pallet-bulletin-data-renewal. Do not reuse
		// this index.

		/// Authorize an account to store up to `bytes` of arbitrary data in `transactions`
		/// boost-tier transactions.
		///
		/// If the account has an active slot, `bytes` and `transactions` are **added** to its
		/// caps (consumed counters preserved; the latest-expiring one when several are active).
		/// Otherwise a new slot over `[relay_now, relay_now + DefaultAuthorizationWindow)` is
		/// added; fails with [`Error::TooManySlots`] when the per-scope bound is reached. To
		/// target a specific window, use
		/// [`authorize_account_window`](Self::authorize_account_window).
		///
		/// The origin for this call must be the pallet's `Authorizer`. Emits
		/// [`AccountAuthorized`](Event::AccountAuthorized) when successful.
		#[pallet::call_index(3)]
		#[pallet::weight(T::WeightInfo::authorize_account())]
		#[pallet::feeless_if(
			|origin: &OriginFor<T>, _who: &T::AccountId, _transactions: &u32, _bytes: &u64| -> bool {
				Pallet::<T>::is_feeless_authorizer(origin)
			}
		)]
		pub fn authorize_account(
			origin: OriginFor<T>,
			who: T::AccountId,
			transactions: u32,
			bytes: u64,
		) -> DispatchResult {
			Self::do_authorize_account(origin, who, transactions, bytes, None, None)
		}

		/// [`authorize_account`](Self::authorize_account) with an explicit
		/// `[starts_at, expiration)` window in relay blocks.
		///
		/// `starts_at = None` means `relay_now`. The window must be non-empty, not already
		/// expired, and start no more than [`Config::MaxStartsAtFuture`] blocks ahead
		/// ([`Error::InvalidWindow`] otherwise). A slot with the same window is merged
		/// additively.
		#[pallet::call_index(18)]
		#[pallet::weight(T::WeightInfo::authorize_account_window())]
		#[pallet::feeless_if(
			|origin: &OriginFor<T>, _who: &T::AccountId, _transactions: &u32, _bytes: &u64, _starts_at: &Option<RelayBlockNumberFor<T>>, _expiration: &RelayBlockNumberFor<T>| -> bool {
				Pallet::<T>::is_feeless_authorizer(origin)
			}
		)]
		pub fn authorize_account_window(
			origin: OriginFor<T>,
			who: T::AccountId,
			transactions: u32,
			bytes: u64,
			starts_at: Option<RelayBlockNumberFor<T>>,
			expiration: RelayBlockNumberFor<T>,
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

		/// Authorize anyone to store a preimage of the given content hash.
		///
		/// If the hash has an active slot, `max_size` is **added** to its `bytes_allowance`.
		/// Otherwise a new slot over the default window is added with
		/// `transactions_allowance = 2` so the store-then-renew flow fits. To target a specific
		/// window, use [`authorize_preimage_window`](Self::authorize_preimage_window).
		///
		/// Parameters:
		///
		/// - `content_hash`: The hash of the data to be submitted. For [`store`](Self::store) this
		///   is the BLAKE2b-256 hash; for [`store_with_cid_config`](Self::store_with_cid_config)
		///   this is the hash produced by the CID config's hashing algorithm.
		/// - `max_size`: The maximum size, in bytes, of the preimage.
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

		/// [`authorize_preimage`](Self::authorize_preimage) with an explicit window; same
		/// validation as [`authorize_account_window`](Self::authorize_account_window).
		#[pallet::call_index(19)]
		#[pallet::weight(T::WeightInfo::authorize_preimage_window())]
		pub fn authorize_preimage_window(
			origin: OriginFor<T>,
			content_hash: ContentHash,
			max_size: u64,
			starts_at: Option<RelayBlockNumberFor<T>>,
			expiration: RelayBlockNumberFor<T>,
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

		/// Extend the latest-expiring active slot of an account authorization to
		/// `relay_now + DefaultAuthorizationWindow` (no-op if it already expires later).
		/// Counters and caps are untouched; to add capacity use `authorize_account*`.
		///
		/// Fails with [`Error::AuthorizationNotFound`] when no active slot exists.
		///
		/// The origin for this call must be the pallet's `Authorizer`. Emits
		/// [`AccountAuthorizationRefreshed`](Event::AccountAuthorizationRefreshed) when successful.
		#[pallet::call_index(7)]
		#[pallet::weight(T::WeightInfo::refresh_account_authorization())]
		#[pallet::feeless_if(|origin: &OriginFor<T>, _who: &T::AccountId| -> bool {
			Pallet::<T>::is_feeless_authorizer(origin)
		})]
		pub fn refresh_account_authorization(
			origin: OriginFor<T>,
			who: T::AccountId,
		) -> DispatchResult {
			T::Authorizer::ensure_origin(origin)?;
			Self::refresh_authorization(AuthorizationScope::Account(who.clone()))?;
			Self::deposit_event(Event::AccountAuthorizationRefreshed { who });
			Ok(())
		}

		/// [`refresh_account_authorization`](Self::refresh_account_authorization) for a
		/// preimage authorization.
		///
		/// Emits [`PreimageAuthorizationRefreshed`](Event::PreimageAuthorizationRefreshed) when
		/// successful.
		#[pallet::call_index(8)]
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

		/// Block-level mandatory inherent for the transaction-storage proof.
		///
		/// `proof` is `Some` when the inherent data provider supplied one; otherwise the
		/// proof step is skipped (early or empty blocks). The companion drain of pending
		/// auto-renewals lives in `pallet-bulletin-data-renewal`'s own inherent.
		#[pallet::call_index(14)]
		#[pallet::weight((T::WeightInfo::apply_block_inherents(), DispatchClass::Mandatory))]
		pub fn apply_block_inherents(
			origin: OriginFor<T>,
			proof: Option<TransactionStorageProof>,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			if let Some(proof) = proof {
				Self::do_check_proof(proof)?;
			}
			Ok(().into())
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
		/// - `valid_until`: when `Some(t)`, must satisfy `t > now`. The entry stops authorizing
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
			let is_new = !AllowedAuthorizers::<T>::contains_key(&who);
			AllowedAuthorizers::<T>::insert(&who, budget);
			if is_new {
				Self::inc_authorizer_providers(&who);
			}
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
				Self::dec_authorizer_providers(&who);
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
			let budget =
				AllowedAuthorizers::<T>::get(&who).ok_or(Error::<T>::AuthorizerNotFound)?;
			ensure!(budget.is_inactive(Self::now()), Error::<T>::AuthorizerBudgetNotExhausted,);
			AllowedAuthorizers::<T>::remove(&who);
			Self::dec_authorizer_providers(&who);
			Self::deposit_event(Event::ExhaustedAuthorizerRemoved { who });
			Ok(())
		}
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Stored data under specified index.
		Stored { index: u32, content_hash: ContentHash, cid: Option<Cid> },
		/// Storage proof was successfully checked.
		ProofChecked,
		/// An account `who` was authorized to store `bytes` bytes in `transactions` boost-tier
		/// transactions over the relay-block window `[starts_at, expiration)`.
		AccountAuthorized {
			who: T::AccountId,
			transactions: u32,
			bytes: u64,
			starts_at: RelayBlockNumberFor<T>,
			expiration: RelayBlockNumberFor<T>,
		},
		/// The latest active slot of account `who`'s authorization was extended.
		AccountAuthorizationRefreshed { who: T::AccountId },
		/// Authorization was given for a preimage of `content_hash` (not exceeding `max_size`) to
		/// be stored by anyone over the relay-block window `[starts_at, expiration)`.
		PreimageAuthorized {
			content_hash: ContentHash,
			max_size: u64,
			starts_at: RelayBlockNumberFor<T>,
			expiration: RelayBlockNumberFor<T>,
		},
		/// An authorization for a preimage of `content_hash` was refreshed.
		PreimageAuthorizationRefreshed { content_hash: ContentHash },
		/// An expired account authorization was removed.
		ExpiredAccountAuthorizationRemoved { who: T::AccountId },
		/// An expired preimage authorization was removed.
		ExpiredPreimageAuthorizationRemoved { content_hash: ContentHash },
		/// An authorizer was added to the allowed list.
		AuthorizerAdded { who: T::AccountId },
		/// An authorizer was removed from the allowed list by the manager.
		AuthorizerRemoved { who: T::AccountId },
		/// An authorizer was removed from the allowed list due to budget exhaustion.
		ExhaustedAuthorizerRemoved { who: T::AccountId },
	}

	/// Authorization slots, keyed by scope. Expired slots are pruned lazily on every mutation
	/// ([`Pallet::prune_expired`]); an entry whose slots all expired is removed and its
	/// provider reference dropped. `pub` for cross-pallet read access from
	/// `pallet-bulletin-data-renewal`'s tests.
	#[pallet::storage]
	pub type Authorizations<T: Config> =
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
		BoundedVec<TransactionInfoFor<T>, T::MaxBlockTransactions>,
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
	//
	// `pub` so `pallet-bulletin-data-renewal` can mutate while applying renewals
	// (the batched drain inherent amortizes a single read/write across the per-block pending
	// vec). Cross-pallet appends go through [`Pallet::with_block_transactions`].
	#[pallet::storage]
	pub(super) type BlockTransactions<T: Config> =
		StorageValue<_, BoundedVec<TransactionInfoFor<T>, T::MaxBlockTransactions>, ValueQuery>;

	/// Maps content hash to its most recent (block_number, tx_index) location.
	///
	/// `pub` for cross-pallet writes from `pallet-bulletin-data-renewal` (each
	/// renewal updates the mapping to the new `(block, index)`). Reads outside this pallet should
	/// go through [`Pallet::lookup_by_content_hash`].
	#[pallet::storage]
	pub(super) type TransactionByContentHash<T: Config> =
		StorageMap<_, Blake2_128Concat, ContentHash, (BlockNumberFor<T>, u32), OptionQuery>;

	/// Was the proof checked in this block?
	#[pallet::storage]
	pub(super) type ProofChecked<T: Config> = StorageValue<_, bool, ValueQuery>;

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
		/// gets `transactions_allowance = 2`, as with `authorize_preimage`.
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
			// Genesis runs before the parachain inherent populates validation data, so the
			// relay clock reads `0`: a `starts_at` of `0` just means "active immediately".
			let starts_at = T::RelayChainBlockNumberProvider::current_block_number();
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
				let is_new = !AllowedAuthorizers::<T>::contains_key(account);
				AllowedAuthorizers::<T>::insert(
					account,
					AuthorizerBudget {
						quota: Some(Quota { transactions: *transactions, bytes: *bytes }),
						// Genesis authorizers never expire; root can re-add them later to set
						// a `valid_until` or flip `feeless`.
						valid_until: None,
						feeless: true,
					},
				);
				if is_new {
					Pallet::<T>::inc_authorizer_providers(account);
				}
			}
		}
	}

	#[pallet::inherent]
	impl<T: Config> ProvideInherent for Pallet<T> {
		type Call = Call<T>;
		type Error = InherentError;
		const INHERENT_IDENTIFIER: InherentIdentifier = INHERENT_IDENTIFIER;

		fn create_inherent(data: &InherentData) -> Option<Self::Call> {
			// Emit the proof inherent only when the inherent data provider supplied a proof;
			// otherwise no inherent is needed (early/empty blocks). The companion drain of
			// pending auto-renewals lives in `pallet-bulletin-data-renewal`'s own
			// `ProvideInherent`.
			let proof = data
				.get_data::<TransactionStorageProof>(&Self::INHERENT_IDENTIFIER)
				.unwrap_or(None)?;
			Some(Call::apply_block_inherents { proof: Some(proof) })
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
	}

	impl<T: Config> Pallet<T> {
		/// Panics unless this pallet's families and `extra` use distinct tag prefixes. `extra`
		/// lets dependent pallets fold in their own.
		pub fn assert_pool_families_distinct(extra: &[(&str, ValidTransactionParams)]) {
			let mut all = extra.to_vec();
			all.extend([
				("StoreTxParams", T::StoreTxParams::get()),
				("AuthorizeTxParams", T::AuthorizeTxParams::get()),
				(
					"RemoveExpiredAccountAuthorizationTxParams",
					T::RemoveExpiredAccountAuthorizationTxParams::get(),
				),
				(
					"RemoveExpiredPreimageAuthorizationTxParams",
					T::RemoveExpiredPreimageAuthorizationTxParams::get(),
				),
				("RemoveExhaustedAuthorizerTxParams", T::RemoveExhaustedAuthorizerTxParams::get()),
			]);
			assert_distinct_tag_prefixes(&all);
		}

		/// Validate that `origin` is one of the accepted caller types for store/renew
		/// extrinsics, and return a typed description of the caller.
		///
		/// Accepted origins:
		///
		/// - [`Origin::Authorized`] (set by [`extension::ValidateAuthorizedCalls`]) →
		///   [`AuthorizedCaller::Signed`]
		/// - Root → [`AuthorizedCaller::Root`]
		/// - None (unsigned) → [`AuthorizedCaller::Unsigned`]
		///
		/// Any other origin (including plain `Signed`) returns
		/// [`DispatchError::BadOrigin`].
		pub fn ensure_authorized(
			origin: OriginFor<T>,
		) -> Result<AuthorizedCallerFor<T>, DispatchError> {
			// 1. Try pallet::Origin::Authorized (set by ValidateAuthorizedCalls extension)
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
		///
		/// FOOTGUN: `sp_io::transaction_index::index` (via `BlockTransactionsMut::store`) indexes
		/// the *trailing* `data_len` bytes of the encoded extrinsic. Since an extrinsic
		/// encodes as `preamble ++ call`, `data` must be the LAST field of any
		/// dispatchable that funnels into `do_store` (e.g. [`store`](Self::store),
		/// [`store_with_cid_config`](Self::store_with_cid_config),
		/// `pallet-bulletin-hop-promotion::promote`). A field encoded after `data`
		/// shifts the indexed window onto the wrong bytes and corrupts the stored
		/// blob — without any dispatch error to flag it.
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

			let index = Self::with_block_transactions(|entries| {
				entries.store(TransactionInfo {
					chunk_root: root,
					size: data_len,
					content_hash: cid.content_hash,
					hashing,
					cid_codec,
					extrinsic_index,
					block_chunks: 0,
					meta: Default::default(),
				})
			})
			.ok_or(Error::<T>::TooManyTransactions)?;

			Self::deposit_event(Event::Stored {
				index,
				content_hash: cid.content_hash,
				cid: cid.to_bytes(),
			});

			Ok(())
		}

		/// Run `f` against this block's entries — one `BlockTransactions` read/write for
		/// the whole closure, so a batch amortizes a single access.
		pub fn with_block_transactions<R>(f: impl FnOnce(&mut BlockTransactionsMut<T>) -> R) -> R {
			<BlockTransactions<T>>::mutate(|entries| f(&mut BlockTransactionsMut { entries }))
		}

		/// Move the entries accumulated for `n` into `Transactions[n]`, iff they have chunks.
		pub(crate) fn seal_block(n: BlockNumberFor<T>) {
			let entries = <BlockTransactions<T>>::take();
			if TransactionInfo::total_chunks(&entries) != 0 {
				<Transactions<T>>::insert(n, entries);
			}
		}

		/// Entries accumulated for the current block.
		pub fn block_transactions_count() -> u32 {
			<BlockTransactions<T>>::decode_len().unwrap_or_default() as u32
		}

		/// Current parachain block number; used for authorizer validity (`valid_until`) and
		/// data retention. Slot windows use [`Self::relay_now`].
		pub fn now() -> BlockNumberFor<T> {
			frame_system::Pallet::<T>::block_number()
		}

		/// Current relay-chain block number. `0` is the genesis sentinel (validation data not
		/// yet set); paths that need a real clock go through [`Self::ensure_relay_now`].
		pub fn relay_now() -> RelayBlockNumberFor<T> {
			T::RelayChainBlockNumberProvider::current_block_number()
		}

		/// [`Self::relay_now`], rejecting the genesis sentinel.
		pub fn ensure_relay_now() -> Result<RelayBlockNumberFor<T>, Error<T>> {
			let relay_now = Self::relay_now();
			ensure!(!relay_now.is_zero(), Error::<T>::RelayChainTimeUnavailable);
			Ok(relay_now)
		}

		fn ensure_valid_window(
			relay_now: RelayBlockNumberFor<T>,
			starts_at: RelayBlockNumberFor<T>,
			expiration: RelayBlockNumberFor<T>,
		) -> Result<(), Error<T>> {
			ensure!(expiration > starts_at, Error::<T>::InvalidWindow);
			ensure!(expiration > relay_now, Error::<T>::InvalidWindow);
			ensure!(
				starts_at.saturating_sub(relay_now) <= T::MaxStartsAtFuture::get(),
				Error::<T>::InvalidWindow
			);
			Ok(())
		}

		/// `expiration = None` is the default form: the window of the latest-expiring active slot
		/// at `scope`, so repeated grants add to it, or a fresh default window from `relay_now`
		/// when none is active. `Some` is the explicit `_window` form, validated by
		/// [`Self::ensure_valid_window`].
		fn resolve_window(
			scope: &AuthorizationScopeFor<T>,
			relay_now: RelayBlockNumberFor<T>,
			starts_at: Option<RelayBlockNumberFor<T>>,
			expiration: Option<RelayBlockNumberFor<T>>,
		) -> Result<(RelayBlockNumberFor<T>, RelayBlockNumberFor<T>), Error<T>> {
			match expiration {
				None => {
					let active = Self::get_active_authorization(scope).and_then(|auth| {
						auth.active_slots(relay_now)
							.max_by_key(|slot| slot.expiration)
							.map(|slot| (slot.starts_at, slot.expiration))
					});
					Ok(active.unwrap_or((
						relay_now,
						relay_now.saturating_add(T::DefaultAuthorizationWindow::get()),
					)))
				},
				Some(expiration) => {
					let starts_at = starts_at.unwrap_or(relay_now);
					Self::ensure_valid_window(relay_now, starts_at, expiration)?;
					Ok((starts_at, expiration))
				},
			}
		}

		fn do_authorize_account(
			origin: OriginFor<T>,
			who: T::AccountId,
			transactions: u32,
			bytes: u64,
			starts_at: Option<RelayBlockNumberFor<T>>,
			expiration: Option<RelayBlockNumberFor<T>>,
		) -> DispatchResult {
			let maybe_authorizer = T::Authorizer::ensure_origin(origin)?;
			ensure!(bytes > 0, Error::<T>::BadDataSize);
			let relay_now = Self::ensure_relay_now()?;
			let scope = AuthorizationScope::Account(who.clone());
			let (starts_at, expiration) =
				Self::resolve_window(&scope, relay_now, starts_at, expiration)?;
			// Charge the budget only once the request is known to be well-formed.
			if let Some(ctx) = maybe_authorizer {
				Self::consume_authorizer_budget(&ctx.authorizer, transactions, bytes)?;
			}
			Self::add_slot(scope, transactions, bytes, starts_at, expiration)?;
			Self::deposit_event(Event::AccountAuthorized {
				who,
				transactions,
				bytes,
				starts_at,
				expiration,
			});
			Ok(())
		}

		/// Preimage slots get `transactions_allowance = 2` so store-then-renew fits; the
		/// authorizer's budget is charged `(1, max_size)`.
		fn do_authorize_preimage(
			origin: OriginFor<T>,
			content_hash: ContentHash,
			max_size: u64,
			starts_at: Option<RelayBlockNumberFor<T>>,
			expiration: Option<RelayBlockNumberFor<T>>,
		) -> DispatchResult {
			let maybe_authorizer = T::Authorizer::ensure_origin(origin)?;
			ensure!(max_size > 0, Error::<T>::BadDataSize);
			let relay_now = Self::ensure_relay_now()?;
			let scope = AuthorizationScope::Preimage(content_hash);
			let (starts_at, expiration) =
				Self::resolve_window(&scope, relay_now, starts_at, expiration)?;
			if let Some(ctx) = maybe_authorizer {
				Self::consume_authorizer_budget(&ctx.authorizer, 1, max_size)?;
			}
			Self::add_slot(scope, 2, max_size, starts_at, expiration)?;
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

		pub(crate) fn authorization_removed(scope: &AuthorizationScopeFor<T>) {
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

		/// Keep `who` alive in System while it's registered in [`AllowedAuthorizers`],
		/// so a `feeless` authorizer with no balance doesn't get reaped between
		/// dispatches (which would reset its replay-protection nonce).
		pub(crate) fn inc_authorizer_providers(who: &T::AccountId) {
			frame_system::Pallet::<T>::inc_providers(who);
		}

		pub(crate) fn dec_authorizer_providers(who: &T::AccountId) {
			if let Err(err) = frame_system::Pallet::<T>::dec_providers(who) {
				tracing::warn!(
					target: LOG_TARGET, error=?err, ?who,
					"dec_providers failed for allowed authorizer; leaking reference",
				);
			}
		}

		/// Drop expired slots for `scope`; remove the entry (and its provider reference) when
		/// none remain. Drained-but-active slots are kept: `store` never gates on the caps.
		pub(crate) fn prune_expired(scope: &AuthorizationScopeFor<T>) {
			let relay_now = Self::relay_now();
			// `mutate_exists` writes back whenever the value stays `Some`; skip the write
			// on the common no-prune path.
			let needs_prune = Authorizations::<T>::get(scope)
				.is_some_and(|auth| auth.slots.iter().any(|s| s.expired(relay_now)));
			if !needs_prune {
				return;
			}
			Authorizations::<T>::mutate_exists(scope, |maybe_auth| {
				let Some(auth) = maybe_auth.as_mut() else { return };
				auth.slots.retain(|slot| !slot.expired(relay_now));
				if auth.slots.is_empty() {
					*maybe_auth = None;
					Self::authorization_removed(scope);
				}
			});
		}

		/// Insert a slot, keeping `slots` sorted by `(expiration, starts_at)`. The caps are
		/// **added** to an existing slot when the windows match exactly, or when both share
		/// `expiration` and are already active (a past `starts_at` is indistinguishable from
		/// `relay_now` for an active slot). Otherwise fails with [`Error::TooManySlots`] when
		/// the bound is reached.
		pub(crate) fn add_slot(
			scope: AuthorizationScopeFor<T>,
			transactions_allowance: u32,
			bytes_allowance: u64,
			starts_at: RelayBlockNumberFor<T>,
			expiration: RelayBlockNumberFor<T>,
		) -> DispatchResult {
			Self::prune_expired(&scope);
			let relay_now = Self::relay_now();

			Authorizations::<T>::try_mutate(&scope, |maybe_auth| -> DispatchResult {
				let was_empty = maybe_auth.is_none();
				let auth = maybe_auth.get_or_insert_with(Authorization::<T>::default);

				let new_is_active = starts_at <= relay_now;
				if let Some(existing) = auth.slots.iter_mut().find(|s| {
					s.expiration == expiration &&
						(s.starts_at == starts_at || (new_is_active && s.starts_at <= relay_now))
				}) {
					// Clamp the saturating axes to the old caps before widening them, so the
					// merged slot folds exactly like two separate slots would.
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
						bytes_allowance,
						transactions: 0,
						transactions_allowance,
						extra: Default::default(),
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

		/// Extend the latest-expiring active slot to `relay_now + DefaultAuthorizationWindow`.
		/// Sort order holds because the touched slot already had the maximum expiration.
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

		/// Remove the entry once every slot has expired. Lazy pruning already does this on
		/// mutation; this lets anyone reclaim the provider reference without one.
		fn remove_expired_authorization(scope: AuthorizationScopeFor<T>) -> DispatchResult {
			let auth = Authorizations::<T>::get(&scope).ok_or(Error::<T>::AuthorizationNotFound)?;
			ensure!(auth.all_expired(Self::relay_now()), Error::<T>::AuthorizationNotExpired);
			Authorizations::<T>::remove(&scope);
			Self::authorization_removed(&scope);
			Ok(())
		}

		/// Returns `true` iff a `store(data)` call where `data.len() == data_len`
		/// would currently pass transaction validation for `who`.
		///
		/// Mirrors the preconditions enforced by [`Self::store`] +
		/// [`Self::check_authorization`]:
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

		/// Returns `true` if `who` has a slot active at the current relay block, regardless
		/// of how much of its extent remains. Future-only and expired slots do not count.
		/// HOP promotion uses this to keep promoting blobs for an account that has spent
		/// all of its quota.
		pub fn account_has_active_authorization(who: &T::AccountId) -> bool {
			Self::get_active_authorization(&AuthorizationScope::Account(who.clone())).is_some()
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
		/// O(1): one `TransactionByContentHash` map read. The map's
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
		pub fn ensure_data_size_ok(size: usize) -> Result<(), Error<T>> {
			ensure!(Self::data_size_ok(size), Error::<T>::BadDataSize);
			Ok(())
		}

		/// Returns the [`TransactionInfo`] for the specified store/renew transaction.
		pub fn transaction_info(
			block_number: BlockNumberFor<T>,
			index: u32,
		) -> Option<TransactionInfoFor<T>> {
			let transactions = Transactions::<T>::get(block_number)?;
			transactions.into_iter().nth(index as usize)
		}

		/// Resolves a [`TransactionRef`] to its [`TransactionInfo`].
		pub fn resolve_transaction_ref(
			entry: &TransactionRef<BlockNumberFor<T>>,
		) -> Result<TransactionInfoFor<T>, Error<T>> {
			let (block, index) = match entry {
				TransactionRef::Position { block, index } => (*block, *index),
				TransactionRef::ContentHash(hash) =>
					TransactionByContentHash::<T>::get(hash).ok_or(Error::<T>::RenewedNotFound)?,
			};
			Self::transaction_info(block, index).ok_or(Error::<T>::RenewedNotFound)
		}

		/// Look up the most-recent `(block, index)` location of a content hash, if any.
		///
		/// Cross-pallet read for `pallet-bulletin-data-renewal` (registration flow:
		/// `enable_auto_renew` validates that the hash points to an extant stored
		/// transaction before inserting an `AutoRenewals` entry).
		pub fn lookup_by_content_hash(hash: ContentHash) -> Option<(BlockNumberFor<T>, u32)> {
			TransactionByContentHash::<T>::get(hash)
		}

		/// All transactions stored at the given block, in the current `TransactionInfo` layout.
		///
		/// Shape-tolerant against entries that are still in the pre-v3 layout.
		pub fn transactions_at(
			block: BlockNumberFor<T>,
		) -> Option<BoundedVec<TransactionInfoFor<T>, T::MaxBlockTransactions>> {
			let raw = sp_io::storage::get(&Transactions::<T>::hashed_key_for(block))?;

			if let Ok(txs) =
				BoundedVec::<TransactionInfoFor<T>, T::MaxBlockTransactions>::decode(&mut &raw[..])
			{
				return Some(txs);
			}

			let v2 = BoundedVec::<
				crate::migrations::v3::V2TransactionInfo,
				T::MaxBlockTransactions,
			>::decode(&mut &raw[..])
			.ok()?;

			let materialized: Vec<TransactionInfoFor<T>> = v2
				.into_iter()
				.map(|tx| TransactionInfo {
					chunk_root: tx.chunk_root,
					content_hash: tx.content_hash,
					hashing: tx.hashing,
					cid_codec: tx.cid_codec,
					size: tx.size,
					extrinsic_index: u32::MAX,
					block_chunks: tx.block_chunks,
					meta: Default::default(),
				})
				.collect();

			BoundedVec::<TransactionInfoFor<T>, T::MaxBlockTransactions>::try_from(materialized)
				.ok()
		}

		/// Returns `true` if no more store/renew transactions can be included in the current
		/// block.
		pub fn block_transactions_full() -> bool {
			Self::block_transactions_count() >= T::MaxBlockTransactions::get()
		}

		/// Check that a slot is active for `scope` and (with `consume`) charge `size` bytes and
		/// one transaction to the earliest-expiring active slot.
		///
		/// Never rejects on insufficient allowance — `bytes` and `transactions` saturate
		/// upward and the [`extension::AllowanceBasedPriority`] boost handles the overshoot
		/// (soft limit). Rejects with `InvalidTransaction::Payment` when no slot is active and
		/// with [`RELAY_CHAIN_TIME_UNAVAILABLE`] before the relay clock is set.
		pub fn check_authorization(
			scope: &AuthorizationScopeFor<T>,
			size: u32,
			consume: bool,
		) -> Result<(), TransactionValidityError> {
			let size_u64: u64 = size.into();
			let relay_now = Self::ensure_relay_now().map_err(|_| RELAY_CHAIN_TIME_UNAVAILABLE)?;
			Self::prune_expired(scope);

			let check = |maybe_auth: &mut Option<Authorization<T>>|
			 -> Result<(), TransactionValidityError> {
				let Some(auth) = maybe_auth.as_mut() else {
					return Err(InvalidTransaction::Payment.into())
				};
				let Some(slot) = auth.slots.iter_mut().find(|s| s.is_active(relay_now)) else {
					return Err(InvalidTransaction::Payment.into())
				};
				if consume {
					slot.extent.bytes = slot.extent.bytes.saturating_add(size_u64);
					slot.extent.transactions = slot.extent.transactions.saturating_add(1);
				}
				Ok(())
			};

			if consume {
				Authorizations::<T>::mutate(scope, check)
			} else {
				let mut auth = Authorizations::<T>::get(scope);
				check(&mut auth)
			}
		}

		/// Run `f` against the active slots of `scope`, earliest-expiring first, until one
		/// returns `Ok` — atomic access to the opaque [`Config::AuthorizationExtra`] payload.
		/// No active slot => `InvalidTransaction::Payment` without calling `f`; every slot
		/// rejected => the last error. Persisted iff `persist` && the result is `Ok`.
		///
		/// `f` runs in place and may be called on several slots, so it must do all of its
		/// checks before mutating: an `Err` must leave the slot untouched.
		pub fn try_mutate_active_authorization<R>(
			scope: &AuthorizationScopeFor<T>,
			persist: bool,
			mut f: impl FnMut(&mut ActiveAuthorization<'_, T>) -> Result<R, TransactionValidityError>,
		) -> Result<R, TransactionValidityError> {
			let relay_now = Self::ensure_relay_now().map_err(|_| RELAY_CHAIN_TIME_UNAVAILABLE)?;
			Self::prune_expired(scope);

			let mut run =
				|maybe_auth: &mut Option<Authorization<T>>| -> Result<R, TransactionValidityError> {
					let Some(auth) = maybe_auth.as_mut() else {
						return Err(InvalidTransaction::Payment.into())
					};
					let mut last_err: Option<TransactionValidityError> = None;
					for slot in auth.slots.iter_mut().filter(|s| s.is_active(relay_now)) {
						match f(&mut ActiveAuthorization { slot }) {
							Ok(r) => return Ok(r),
							Err(e) => last_err = Some(e),
						}
					}
					Err(last_err.unwrap_or(InvalidTransaction::Payment.into()))
				};
			if persist {
				Authorizations::<T>::try_mutate(scope, run)
			} else {
				let mut auth = Authorizations::<T>::get(scope);
				run(&mut auth)
			}
		}

		/// Read-only companion to [`Self::try_mutate_active_authorization`]: the
		/// authorization at `scope` restricted to its active slots, or `None` when none is.
		pub fn get_active_authorization(
			scope: &AuthorizationScopeFor<T>,
		) -> Option<Authorization<T>> {
			let relay_now = Self::relay_now();
			let mut auth = Authorizations::<T>::get(scope)?;
			auth.slots.retain(|s| s.is_active(relay_now));
			(!auth.slots.is_empty()).then_some(auth)
		}

		/// Check that authorization with the given scope exists in storage and every slot
		/// has expired. Mirrors the dispatch-time guard in [`remove_expired_authorization`]
		/// so that `remove_expired_*` calls are rejected at pool ingress when they cannot
		/// succeed (no pool pollution from soon-to-fail txs).
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

		fn check_store_unsigned(
			size: usize,
			content_hash: impl FnOnce() -> ContentHash,
			context: CheckContext,
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
			)?;

			Ok(context
				.want_valid_transaction()
				.then(|| T::StoreTxParams::get().provides(content_hash)))
		}

		fn check_unsigned(
			call: &Call<T>,
			context: CheckContext,
		) -> Result<Option<ValidTransaction>, TransactionValidityError> {
			match call {
				Call::<T>::store { data } => Self::check_store_unsigned(
					data.len(),
					|| sp_io::hashing::blake2_256(data),
					context,
				),
				Call::<T>::store_with_cid_config { cid, data } =>
					Self::check_store_unsigned(data.len(), || cid.hashing.hash(data), context),
				Call::<T>::remove_expired_account_authorization { who } => {
					Self::check_authorization_expired(&AuthorizationScope::Account(who.clone()))?;
					Ok(context
						.want_valid_transaction()
						.then(|| T::RemoveExpiredAccountAuthorizationTxParams::get().provides(who)))
				},
				Call::<T>::remove_expired_preimage_authorization { content_hash } => {
					Self::check_authorization_expired(&AuthorizationScope::Preimage(
						*content_hash,
					))?;
					Ok(context.want_valid_transaction().then(|| {
						T::RemoveExpiredPreimageAuthorizationTxParams::get().provides(content_hash)
					}))
				},
				Call::<T>::remove_exhausted_authorizer { who } => {
					let budget = AllowedAuthorizers::<T>::get(who).ok_or(AUTHORIZER_NOT_FOUND)?;
					ensure!(budget.is_inactive(Self::now()), AUTHORIZATION_NOT_EXHAUSTED);
					Ok(context
						.want_valid_transaction()
						.then(|| T::RemoveExhaustedAuthorizerTxParams::get().provides(who)))
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
			let (size, content_hash) = match call {
				Call::<T>::store { data } => {
					let content_hash = sp_io::hashing::blake2_256(data);
					(data.len(), content_hash)
				},
				Call::<T>::store_with_cid_config { cid, data } => {
					let content_hash = cid.hashing.hash(data);
					(data.len(), content_hash)
				},
				Call::<T>::authorize_account { .. } |
				Call::<T>::authorize_preimage { .. } |
				Call::<T>::authorize_account_window { .. } |
				Call::<T>::authorize_preimage_window { .. } |
				Call::<T>::refresh_account_authorization { .. } |
				Call::<T>::refresh_preimage_authorization { .. } => {
					// Verify that the signer satisfies the Authorizer origin. Budget
					// consumption (for `AllowedAuthorizers` signers on `authorize_*`)
					// happens inside the dispatch body, not here.
					let origin = frame_system::RawOrigin::Signed(who.clone()).into();
					T::Authorizer::ensure_origin(origin)
						.map_err(|_| InvalidTransaction::BadSigner)?;
					return Ok((
						context
							.want_valid_transaction()
							.then(|| T::AuthorizeTxParams::get().untagged()),
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
			)
			.is_ok();

			if !used_preimage_auth {
				Self::check_authorization(
					&AuthorizationScope::Account(who.clone()),
					size as u32,
					consume,
				)?;
			}

			// Only build `ValidTransaction` metadata during pool validation, not block
			// execution. The tx tag/priority differs depending on whether preimage or account
			// authorization was used.
			let (valid_tx, scope) = if context.want_valid_transaction() {
				let (valid_tx, scope) = if used_preimage_auth {
					(
						T::StoreTxParams::get().provides(content_hash),
						AuthorizationScope::Preimage(content_hash),
					)
				} else {
					(
						T::StoreTxParams::get().provides((who, content_hash)),
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
			infos: Vec<TransactionInfoFor<T>>,
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

		/// Backs `#[pallet::feeless_if]` on `authorize_account` and
		/// `refresh_account_authorization`. Goes through `T::Authorizer::ensure_origin`
		/// so Root / XCM (`Ok(None)`) are not feeless via this flag.
		///
		/// Also requires the authorizer's budget to be active (not exhausted or
		/// expired). An inactive authorizer would fail downstream anyway; gating
		/// `feeless` on it prevents spamming free, failing dispatches.
		pub(crate) fn is_feeless_authorizer(origin: &OriginFor<T>) -> bool {
			let Ok(Some(ctx)) = T::Authorizer::ensure_origin(origin.clone()) else {
				return false;
			};
			if !ctx.feeless {
				return false;
			}
			AllowedAuthorizers::<T>::get(&ctx.authorizer)
				.is_some_and(|b| !b.is_inactive(Self::now()))
		}

		/// Atomically decrement `who`'s [`AllowedAuthorizers`] budget by
		/// `transactions` / `bytes`. Returns [`Error::InsufficientAuthorizerBudget`]
		/// when either axis would go negative; on failure the budget is unchanged.
		///
		/// A missing entry is a no-op: callers reach this only via [`authorize`]
		/// once `T::Authorizer::ensure_origin` has already accepted the dispatch,
		/// so `who` is necessarily an `AllowedAuthorizers` entry — but the storage
		/// could have been removed between the origin check and here.
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

	/// This block's entry accumulator, held for the duration of
	/// [`Pallet::with_block_transactions`]. Keeps the `BlockTransactions` /
	/// `TransactionByContentHash` / host-index invariants in this pallet.
	pub struct BlockTransactionsMut<'a, T: Config> {
		entries: &'a mut BoundedVec<TransactionInfoFor<T>, T::MaxBlockTransactions>,
	}

	impl<T: Config> BlockTransactionsMut<'_, T> {
		/// Appends without indexing the data — private so every public path indexes.
		/// `entry.block_chunks` is derived here. `None` at `MaxBlockTransactions`.
		fn push(&mut self, mut entry: TransactionInfoFor<T>) -> Option<u32> {
			entry.block_chunks =
				TransactionInfo::total_chunks(self.entries).saturating_add(num_chunks(entry.size));
			let content_hash = entry.content_hash;
			let new_index = self.entries.len() as u32;
			self.entries.try_push(entry).ok()?;
			TransactionByContentHash::<T>::insert(content_hash, (Pallet::<T>::now(), new_index));
			Some(new_index)
		}

		/// Append `entry` and index the trailing `entry.size` bytes of extrinsic
		/// `entry.extrinsic_index` — see the FOOTGUN note on [`Pallet::do_store`]. The host
		/// call is not rolled back, so nothing fallible may follow it in this dispatch.
		pub(crate) fn store(&mut self, entry: TransactionInfoFor<T>) -> Option<u32> {
			let (extrinsic_index, size, content_hash) =
				(entry.extrinsic_index, entry.size, entry.content_hash);
			let new_index = self.push(entry)?;
			sp_io::transaction_index::index(extrinsic_index, size, content_hash);
			Some(new_index)
		}

		/// `Self::store` for already-stored data: appends a copy of `info` under
		/// `extrinsic_index` and `meta`, and re-indexes it with the host. Same no-rollback
		/// caveat.
		///
		/// Appends only — the caller owns the economics. This does not check that
		/// `info.content_hash` is still stored, and does not charge the renewal: the
		/// per-account `bytes_permanent` debit and the renewing pallet's chain-wide
		/// permanent-storage cap check must already have run for `info.size` (see
		/// [`Pallet::check_authorization`]). Skipping them grants permanent retention for
		/// free and leaves that counter out of step with the entries it accounts for.
		pub fn renew(
			&mut self,
			info: &TransactionInfoFor<T>,
			extrinsic_index: u32,
			meta: T::EntryMeta,
		) -> Option<u32> {
			let new_index = self.push(TransactionInfo { extrinsic_index, meta, ..info.clone() })?;
			sp_io::transaction_index::renew(extrinsic_index, info.content_hash);
			Some(new_index)
		}
	}
}

pub mod extension;

/// Block-local state for dependent pallets' tests, which cannot see `BlockTransactions`.
#[cfg(any(test, feature = "std"))]
impl<T: Config> Pallet<T> {
	/// Entries accumulated for the current block.
	pub fn block_transactions() -> BoundedVec<TransactionInfoFor<T>, T::MaxBlockTransactions> {
		<BlockTransactions<T>>::get()
	}

	/// Seal `block`'s accumulated entries exactly as `on_finalize` does.
	pub fn seal_block_transactions(block: BlockNumberFor<T>) {
		Self::seal_block(block);
	}

	/// Append `count` placeholder entries of `size` bytes, to drive the block-full paths.
	pub fn fill_block_transactions(count: u32, size: u32) {
		<BlockTransactions<T>>::mutate(|entries| {
			for _ in 0..count {
				let _ = entries.try_push(TransactionInfo {
					chunk_root: Default::default(),
					size,
					content_hash: [0u8; 32],
					hashing: HashingAlgorithm::Blake2b256,
					cid_codec: RAW_CODEC,
					extrinsic_index: 0,
					block_chunks: 0,
					meta: Default::default(),
				});
			}
		});
	}
}

#[cfg(any(test, feature = "std", feature = "try-runtime"))]
impl<T: Config> Pallet<T> {
	pub fn do_try_state(n: BlockNumberFor<T>) -> Result<(), sp_runtime::TryRuntimeError> {
		ensure!(!Self::retention_period().is_zero(), "RetentionPeriod must not be zero");
		Self::check_transactions_integrity()?;
		Self::check_no_stale_transactions(n)?;
		Self::check_authorizations_integrity()?;
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

	/// Verify slot-storage invariants: entries are non-empty, every slot has
	/// `expiration > starts_at` and a non-zero `bytes_allowance`, and slots are sorted by
	/// `(expiration, starts_at)`. The `bytes` (used) counter can exceed `bytes_allowance` —
	/// that just disables the priority boost, it does not remove the slot.
	fn check_authorizations_integrity() -> Result<(), sp_runtime::TryRuntimeError> {
		for (_, auth) in Authorizations::<T>::iter() {
			ensure!(!auth.slots.is_empty(), "Authorizations entry has no slots");
			let mut previous: Option<(RelayBlockNumberFor<T>, RelayBlockNumberFor<T>)> = None;
			for slot in auth.slots.iter() {
				ensure!(
					slot.expiration > slot.starts_at,
					"Stored slot has expiration <= starts_at"
				);
				ensure!(slot.extent.bytes_allowance > 0, "Stored slot has zero bytes_allowance");
				if let Some((previous_expiration, previous_starts_at)) = previous {
					ensure!(
						previous_expiration < slot.expiration ||
							(previous_expiration == slot.expiration &&
								previous_starts_at <= slot.starts_at),
						"Slots not sorted by (expiration, starts_at)"
					);
				}
				previous = Some((slot.expiration, slot.starts_at));
			}
		}

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
/// - `extra_mandatory`: mandatory weight other pallets add to the same block; `Weight::zero()` for
///   a store-only runtime. Only the runtime sees both pallets.
///
/// # Panics
///
/// Panics with a descriptive message if any check fails.
/// Normal-class budget left for extrinsics: `max_total` minus FRAME's `on_initialize`
/// reservation, proof size capped at the collator's PoV limit. Shared with consumer pallets.
#[cfg(any(test, feature = "std"))]
pub fn effective_normal_budget<T: Config>(
	collator_pov_percent: Option<u64>,
) -> frame_support::weights::Weight {
	use frame_support::{dispatch::DispatchClass, weights::Weight};

	let block_weights = <T as frame_system::Config>::BlockWeights::get();
	let normal = block_weights.get(DispatchClass::Normal);
	let normal_max_total = normal.max_total.expect("Normal class must have a max_total weight");
	let max_extrinsic =
		normal.max_extrinsic.expect("Normal class must have a max_extrinsic weight");
	let init_weight = normal_max_total
		.saturating_sub(max_extrinsic)
		.saturating_sub(normal.base_extrinsic);

	let after_init = normal_max_total.saturating_sub(init_weight);
	match collator_pov_percent {
		// Collators cap the PoV to reserve headroom for the relay-chain state proof.
		// Reference: cumulus/client/consensus/aura/src/collators/lookahead.rs
		Some(pov_percent) => {
			let pov_limit = block_weights.max_block.proof_size() * pov_percent / 100;
			Weight::from_parts(after_init.ref_time(), after_init.proof_size().min(pov_limit))
		},
		None => after_init,
	}
}

#[cfg(any(test, feature = "std"))]
pub fn ensure_weight_sanity<T: Config>(
	collator_pov_percent: Option<u64>,
	extra_mandatory: frame_support::weights::Weight,
) {
	use frame_support::dispatch::DispatchClass;

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

	let effective_normal = effective_normal_budget::<T>(collator_pov_percent);

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

	// 4. Renew-call ref-time fit lives with the dispatchables, in
	//    `pallet_bulletin_data_renewal::ensure_weight_sanity`.

	// 5. apply_block_inherents (DispatchClass::Mandatory, once per block) must fit
	// in max block at worst case (proof check over a full `MaxBlockTransactions` block).
	let apply_inherents_weight = T::WeightInfo::apply_block_inherents();
	assert!(
		apply_inherents_weight.all_lte(block_weights.max_block),
		"apply_block_inherents weight {apply_inherents_weight:?} exceeds max block {:?}",
		block_weights.max_block,
	);

	// 6. on_initialize at the worst-case expiry block (taking
	// `MaxBlockTransactions` items out of `Transactions[obsolete]` and looking up
	// `TransactionByContentHash` + the consumer pallet's state for each) must fit
	// alongside `apply_block_inherents` and `extra_mandatory` within `max_block`. All
	// run on that one block in the mandatory class, which cannot be refused, so their
	// sum is the floor of the mandatory budget for that block.
	let on_init_with_expiry_weight = T::WeightInfo::on_initialize_with_expiry(max_block_txs);
	let mandatory_floor = on_init_with_expiry_weight
		.saturating_add(apply_inherents_weight)
		.saturating_add(extra_mandatory);
	assert!(
		mandatory_floor.all_lte(block_weights.max_block),
		"on_initialize_with_expiry({max_block_txs}) + apply_block_inherents() \
		 + extra_mandatory {extra_mandatory:?} = {mandatory_floor:?} exceeds max block {:?}",
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
	println!("  block_weights.max_block:    {:?}", block_weights.max_block);
	println!("  apply_block_inherents wt:   {apply_inherents_weight:?}");
	println!("  on_initialize_with_expiry:  {on_init_with_expiry_weight:?}");
	println!("  extra mandatory:            {extra_mandatory:?}");
	println!("  Mandatory floor (sum):      {mandatory_floor:?}");
	println!("  Max store txs by weight:    {max_txs_by_weight}");
	println!("  Max store txs by length:    {}", normal_length / per_tx_size);
}
