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

use alloc::vec::Vec;
use bulletin_transaction_storage_primitives::{
	cids::{calculate_cid, Cid, CidCodec, CidConfig, HashingAlgorithm, RAW_CODEC},
	ContentHash,
};
use codec::{Decode, Encode, MaxEncodedLen};
use core::fmt::Debug;
use polkadot_sdk_frame::{
	deps::*,
	prelude::*,
	traits::{
		fungible::{hold::Balanced, Credit, Inspect, Mutate, MutateHold},
		parameter_types, OriginTrait, UnixTime,
	},
};
use sp_transaction_storage_proof::{
	encode_index, num_chunks, random_chunk, ChunkIndex, InherentError, TransactionStorageProof,
	CHUNK_SIZE, INHERENT_IDENTIFIER,
};

/// A type alias for the balance type from this pallet's point of view.
type BalanceOf<T> =
	<<T as Config>::Currency as Inspect<<T as frame_system::Config>::AccountId>>::Balance;
pub type CreditOf<T> = Credit<<T as frame_system::Config>::AccountId, <T as Config>::Currency>;

// Re-export pallet items so that they can be accessed from the crate namespace.
pub use pallet::*;
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

/// Per-period usage and budget of an authorization grant. All fields reset when a new
/// grant is created (e.g. period rollover). `bytes` accumulates as data is stored via
/// `store` (soft cap on bytes — overage demotes priority, never rejects). `bytes_permanent`
/// accumulates as data is renewed via `renew` (hard cap on bytes — combined with `bytes`
/// against `bytes_allowance`, overage rejects). `transactions_used` counts boost-tier
/// calls only (in-budget store + renew); over-budget store calls do NOT increment it.
/// `transactions_allowance` is the per-period budget of "guaranteed-priority" calls and
/// acts as a DoS shield against 1-byte-store spam.
#[derive(
	Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen, Default,
)]
pub struct AuthorizationExtent {
	/// Bytes consumed by `store` calls this period (soft cap on bytes).
	pub bytes: u64,
	/// Bytes consumed by `renew` calls this period (hard cap on bytes, combined with `bytes`).
	pub bytes_permanent: u64,
	/// Per-period byte budget shared by `store` and `renew`.
	pub bytes_allowance: u64,
	/// Boost-tier call count this period (in-budget store + renew).
	pub transactions_used: u32,
	/// Per-period boost-tier transaction budget.
	pub transactions_allowance: u32,
}

/// The scope of an authorization.
///
/// This type is used both for storage keys and to indicate which authorization
/// was consumed for a store/renew transaction (passed via custom origin).
#[derive(
	Clone,
	PartialEq,
	Eq,
	Debug,
	Encode,
	Decode,
	codec::DecodeWithMemTracking,
	scale_info::TypeInfo,
	MaxEncodedLen,
)]
pub enum AuthorizationScope<AccountId> {
	/// Authorization for the given account to store arbitrary data.
	Account(AccountId),
	/// Authorization for anyone to store data with a specific hash.
	Preimage(ContentHash),
}

type AuthorizationScopeFor<T> = AuthorizationScope<<T as frame_system::Config>::AccountId>;

/// Describes the caller of a store/renew extrinsic after origin validation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AuthorizedCaller<AccountId> {
	/// A signed transaction whose origin was transformed to
	/// [`pallet::Origin::Authorized`] by [`extension::ValidateStorageCalls`].
	Signed { who: AccountId, scope: AuthorizationScope<AccountId> },
	/// A root call (e.g. via `sudo`).
	Root,
	/// An unsigned transaction validated by [`ValidateUnsigned`].
	/// TODO: replaced by https://github.com/paritytech/polkadot-bulletin-chain/pull/194
	Unsigned,
}

/// Convenience alias for [`AuthorizedCaller`] bound to a runtime's `AccountId`.
pub type AuthorizedCallerFor<T> = AuthorizedCaller<<T as frame_system::Config>::AccountId>;

pub use extension::{CallInspector, MAX_WRAPPER_DEPTH};

/// A per-period grant of authorization: the extent available during `period`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
pub struct PeriodGrant {
	/// Period this grant is valid for.
	pub period: u32,
	/// Per-period usage and budget for this grant.
	pub extent: AuthorizationExtent,
}

/// An authorization to store data, modelled as up to two per-period grants.
///
/// - `current` holds the grant usable *now* (i.e., for the current period).
/// - `next` holds a pre-booked grant for the next period.
///
/// On any access, expired slots are pruned and `next` is shifted into `current` if
/// it matches the current period.
#[derive(Clone, PartialEq, Eq, Debug, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
pub struct Authorization {
	/// The grant usable in the current period (if any).
	pub current: Option<PeriodGrant>,
	/// A pre-booked grant that activates when its period becomes the current one.
	pub next: Option<PeriodGrant>,
}

impl Authorization {
	/// Whether this authorization has no live or future grants left.
	pub fn is_empty(&self) -> bool {
		self.current.is_none() && self.next.is_none()
	}
}


/// State data for a stored transaction.
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq, scale_info::TypeInfo, MaxEncodedLen)]
pub struct TransactionInfo {
	/// Chunk trie root.
	chunk_root: <BlakeTwo256 as Hash>::Output,

	/// Plain hash of indexed data.
	pub content_hash: ContentHash,
	/// Used hashing algorithm for `content_hash`.
	pub hashing: HashingAlgorithm,
	/// Codec for CID.
	pub cid_codec: CidCodec,

	/// Size of indexed data in bytes.
	size: u32,
	/// Total number of chunks added in the block with this transaction. This
	/// is used to find transaction info by block chunk index using binary search.
	///
	/// Cumulative value of all previous transactions in the block; the last transaction holds the
	/// total chunks.
	block_chunks: ChunkIndex,
}

impl TransactionInfo {
	/// Get the number of total chunks.
	///
	/// See the `block_chunks` field of [`TransactionInfo`] for details.
	pub fn total_chunks(txs: &[TransactionInfo]) -> ChunkIndex {
		txs.last().map_or(0, |t| t.block_chunks)
	}
}

/// Context of a `check_signed`/`check_unsigned` call.
#[derive(Clone, Copy)]
enum CheckContext {
	/// `validate_signed` or `validate_unsigned`.
	Validate,
	/// `pre_dispatch_signed` or `pre_dispatch`.
	PreDispatch,
}

impl CheckContext {
	/// Should authorization be consumed in this context? If not, we merely check that
	/// authorization exists.
	fn consume_authorization(self) -> bool {
		matches!(self, CheckContext::PreDispatch)
	}

	/// Should `check_signed`/`check_unsigned` return a `ValidTransaction`?
	fn want_valid_transaction(self) -> bool {
		matches!(self, CheckContext::Validate)
	}
}

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
		/// Policy ceiling, in bytes, on the sum of `bytes_allowance` across all live
		/// authorizations.
		#[pallet::constant]
		type MaxPermanentStorageSize: Get<u64>;
		/// Unix-time source for computing the current period. Must agree with the people
		/// chain (e.g., `pallet_timestamp::Pallet<Runtime>`).
		type TimeProvider: UnixTime;
		/// Length of an authorization period, in seconds. Must match the people chain's
		/// `LongTermStoragePeriodDuration`.
		#[pallet::constant]
		type PeriodDuration: Get<u64>;
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
		/// Content hash was not calculated.
		InvalidContentHash,
		/// Authorization was requested for a period that is neither the current period
		/// nor the next period.
		InvalidPeriod,
	}

	const STORAGE_VERSION: StorageVersion = StorageVersion::new(2);

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

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
		fn on_initialize(n: BlockNumberFor<T>) -> Weight {
			// TODO: https://github.com/paritytech/polkadot-sdk/issues/10203 - Replace this with benchmarked weights.
			let mut weight = Weight::zero();
			let db_weight = T::DbWeight::get();

			// Run v0→v1 migration if it hasn't been applied yet.
			// This handles the case where `codeSubstitutes` loaded the fix runtime
			// without triggering `on_runtime_upgrade` (spec_version unchanged).
			// Safe alongside the regular `MigrateV0ToV1` wired in Executive: both
			// check `on_chain_storage_version() < 1`, so whichever runs first bumps
			// the version and the other becomes a no-op.
			// TODO: Remove once all chains have been migrated past v1 — after that
			// this is just a redundant storage read per block.
			weight.saturating_accrue(migrations::v1::maybe_migrate_v0_to_v1::<T>());

			// Drop obsolete roots. The proof for `obsolete` will be checked later
			// in this block, so we drop `obsolete` - 1.
			weight.saturating_accrue(db_weight.reads(1));
			let period = Self::retention_period();
			let obsolete = n.saturating_sub(period.saturating_add(One::one()));
			if obsolete > Zero::zero() {
				weight.saturating_accrue(db_weight.writes(1));
				<Transactions<T>>::remove(obsolete);
			}

			// For `on_finalize`
			weight.saturating_accrue(db_weight.reads_writes(2, 2));

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
				T::PeriodDuration::get() > 0,
				"PeriodDuration must be greater than zero"
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

		/// Renew previously stored data. Parameters are the block number that contains previous
		/// `store` or `renew` call and transaction index within that block. Transaction index is
		/// emitted in the `Stored` or `Renewed` event.
		///
		/// As with [`store`](Self::store), authorization is required to renew data using regular
		/// signed/unsigned transactions.
		///
		/// Emits [`Renewed`](Event::Renewed) when successful.
		///
		/// ## Complexity
		///
		/// O(1).
		#[pallet::call_index(1)]
		#[pallet::weight(T::WeightInfo::renew())]
		pub fn renew(
			origin: OriginFor<T>,
			block: BlockNumberFor<T>,
			index: u32,
		) -> DispatchResultWithPostInfo {
			let _caller = Self::ensure_authorized(origin)?;
			let info = Self::transaction_info(block, index).ok_or(Error::<T>::RenewedNotFound)?;

			// In the case of a regular unsigned transaction, this should have been checked by
			// pre_dispatch. In the case of a regular signed transaction, this should have been
			// checked by pre_dispatch_signed.
			Self::ensure_data_size_ok(info.size as usize)?;

			let extrinsic_index =
				<frame_system::Pallet<T>>::extrinsic_index().ok_or(Error::<T>::BadContext)?;
			let content_hash = info.content_hash;
			sp_io::transaction_index::renew(extrinsic_index, content_hash);

			let mut index = 0;
			<BlockTransactions<T>>::mutate(|transactions| {
				if transactions.len() + 1 > T::MaxBlockTransactions::get() as usize {
					return Err(Error::<T>::TooManyTransactions);
				}
				let chunks = num_chunks(info.size);
				let total_chunks = TransactionInfo::total_chunks(transactions) + chunks;
				index = transactions.len() as u32;
				transactions
					.try_push(TransactionInfo {
						chunk_root: info.chunk_root,
						size: info.size,
						content_hash: info.content_hash,
						hashing: info.hashing,
						cid_codec: info.cid_codec,
						block_chunks: total_chunks,
					})
					.map_err(|_| Error::<T>::TooManyTransactions)
			})?;
			Self::deposit_event(Event::Renewed { index, content_hash });
			Ok(().into())
		}

		/// Check storage proof for block number `block_number() - RetentionPeriod`. If such a block
		/// does not exist, the proof is expected to be `None`.
		///
		/// ## Complexity
		///
		/// Linear w.r.t the number of indexed transactions in the proved block for random probing.
		/// There's a DB read for each transaction.
		#[pallet::call_index(2)]
		#[pallet::weight((T::WeightInfo::check_proof(), DispatchClass::Mandatory))]
		pub fn check_proof(
			origin: OriginFor<T>,
			proof: TransactionStorageProof,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			ensure!(!ProofChecked::<T>::get(), Error::<T>::DoubleCheck);

			// Get the target block metadata.
			let number = <frame_system::Pallet<T>>::block_number();
			let period = Self::retention_period();
			let target_number = number.saturating_sub(period);
			ensure!(!target_number.is_zero(), Error::<T>::UnexpectedProof);
			let transactions =
				Transactions::<T>::get(target_number).ok_or(Error::<T>::MissingStateData)?;

			// Verify the proof with a "random" chunk (randomness is based on the parent hash).
			let parent_hash = frame_system::Pallet::<T>::parent_hash();
			Self::verify_chunk_proof(proof, parent_hash.as_ref(), transactions.to_vec())?;
			ProofChecked::<T>::put(true);
			Self::deposit_event(Event::ProofChecked);
			Ok(().into())
		}

		/// Authorize an account to store up to `bytes` bytes via at most `transactions`
		/// boost-tier `store`/`renew` calls during `for_period`.
		///
		/// If the account already has a grant for the same slot (current or next), the
		/// new allowance is **added** to the existing one — `bytes_allowance += bytes`,
		/// `transactions_allowance += transactions`. Used counters are preserved.
		///
		/// `for_period` must be the current period or the next one; any other value is
		/// rejected with [`Error::InvalidPeriod`].
		///
		/// Parameters:
		///
		/// - `who`: The account to be credited.
		/// - `transactions`: Per-period budget of boost-tier `store`/`renew` calls.
		/// - `bytes`: Per-period byte budget shared between `store` and `renew`.
		/// - `for_period`: The period this authorization is valid in.
		///
		/// The origin for this call must be the pallet's `Authorizer`. Emits
		/// [`AccountAuthorized`](Event::AccountAuthorized) when successful.
		#[pallet::call_index(3)]
		#[pallet::weight(T::WeightInfo::authorize_account())]
		#[pallet::feeless_if(|origin: &OriginFor<T>, _who: &T::AccountId, _transactions: &u32, _bytes: &u64, _for_period: &u32| -> bool {
			T::Authorizer::try_origin(origin.clone()).is_ok()
		})]
		pub fn authorize_account(
			origin: OriginFor<T>,
			who: T::AccountId,
			transactions: u32,
			bytes: u64,
			for_period: u32,
		) -> DispatchResult {
			T::Authorizer::ensure_origin(origin)?;
			ensure!(transactions > 0 && bytes > 0, Error::<T>::BadDataSize);
			Self::authorize(
				AuthorizationScope::Account(who.clone()),
				transactions,
				bytes,
				for_period,
			)?;
			Self::deposit_event(Event::AccountAuthorized {
				who,
				transactions,
				bytes,
				period: for_period,
			});
			Ok(())
		}

		/// Authorize anyone to store a preimage of the given content hash, valid for
		/// `for_period`. Each preimage can be stored at most once per grant
		/// (`transactions_allowance = 1`).
		///
		/// If a grant already exists for the same slot, the new `max_size` is added to
		/// the existing `bytes_allowance` and the transaction budget is incremented by 1.
		///
		/// `for_period` must be the current period or the next one; any other value is
		/// rejected with [`Error::InvalidPeriod`].
		///
		/// Parameters:
		///
		/// - `content_hash`: The hash of the data to be submitted. For [`store`](Self::store) this
		///   is the BLAKE2b-256 hash; for [`store_with_cid_config`](Self::store_with_cid_config)
		///   this is the hash produced by the CID config's hashing algorithm.
		/// - `max_size`: The maximum size, in bytes, of the preimage.
		/// - `for_period`: The period this authorization is valid in.
		///
		/// The origin for this call must be the pallet's `Authorizer`. Emits
		/// [`PreimageAuthorized`](Event::PreimageAuthorized) when successful.
		#[pallet::call_index(4)]
		#[pallet::weight(T::WeightInfo::authorize_preimage())]
		#[pallet::feeless_if(|origin: &OriginFor<T>, _content_hash: &ContentHash, _max_size: &u64, _for_period: &u32| -> bool {
			T::Authorizer::try_origin(origin.clone()).is_ok()
		})]
		pub fn authorize_preimage(
			origin: OriginFor<T>,
			content_hash: ContentHash,
			max_size: u64,
			for_period: u32,
		) -> DispatchResult {
			T::Authorizer::ensure_origin(origin)?;
			ensure!(max_size > 0, Error::<T>::BadDataSize);
			Self::authorize(AuthorizationScope::Preimage(content_hash), 1, max_size, for_period)?;
			Self::deposit_event(Event::PreimageAuthorized {
				content_hash,
				max_size,
				period: for_period,
			});
			Ok(())
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

		// `refresh_account_authorization` (old call_index 7) and
		// `refresh_preimage_authorization` (old call_index 8) were removed. Refreshing
		// is expressed by calling `authorize_account` / `authorize_preimage` again with
		// the desired `for_period`.
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
		/// An account `who` was authorized to store `bytes` bytes in at most
		/// `transactions` boost-tier transactions, valid for the given `period`.
		AccountAuthorized {
			who: T::AccountId,
			transactions: u32,
			bytes: u64,
			period: u32,
		},
		/// Authorization was given for a preimage of `content_hash` (not exceeding `max_size`) to
		/// be stored by anyone, valid for the given `period`.
		PreimageAuthorized { content_hash: ContentHash, max_size: u64, period: u32 },
		/// An expired account authorization was removed.
		ExpiredAccountAuthorizationRemoved { who: T::AccountId },
		/// An expired preimage authorization was removed.
		ExpiredPreimageAuthorizationRemoved { content_hash: ContentHash },
	}

	/// Authorizations, keyed by scope.
	#[pallet::storage]
	pub(super) type Authorizations<T: Config> =
		StorageMap<_, Blake2_128Concat, AuthorizationScopeFor<T>, Authorization, OptionQuery>;

	/// Collection of transaction metadata by block number.
	#[pallet::storage]
	#[pallet::getter(fn transaction_roots)]
	pub(super) type Transactions<T: Config> = StorageMap<
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

	/// Was the proof checked in this block?
	#[pallet::storage]
	pub(super) type ProofChecked<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		pub byte_fee: BalanceOf<T>,
		pub entry_fee: BalanceOf<T>,
		pub retention_period: BlockNumberFor<T>,
		/// Initial account authorizations as (account, transactions_allowance, bytes_allowance) tuples.
		pub account_authorizations: Vec<(T::AccountId, u32, u64)>,
		/// Initial preimage authorizations as (content_hash, max_size) tuples.
		/// Each preimage gets `transactions_allowance = 1`.
		pub preimage_authorizations: Vec<(ContentHash, u64)>,
	}

	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> Self {
			Self {
				byte_fee: 10u32.into(),
				entry_fee: 1000u32.into(),
				retention_period: DEFAULT_RETENTION_PERIOD.into(),
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
			let period = Pallet::<T>::current_period();
			for (who, transactions, bytes) in &self.account_authorizations {
				let scope = AuthorizationScope::Account(who.clone());
				Authorizations::<T>::insert(
					&scope,
					Authorization {
						current: Some(PeriodGrant {
							period,
							extent: AuthorizationExtent {
								bytes: 0,
								bytes_permanent: 0,
								bytes_allowance: *bytes,
								transactions_used: 0,
								transactions_allowance: *transactions,
							},
						}),
						next: None,
					},
				);
				Pallet::<T>::authorization_added(&scope);
			}
			for (content_hash, max_size) in &self.preimage_authorizations {
				let scope = AuthorizationScope::Preimage(*content_hash);
				Authorizations::<T>::insert(
					&scope,
					Authorization {
						current: Some(PeriodGrant {
							period,
							extent: AuthorizationExtent {
								bytes: 0,
								bytes_permanent: 0,
								bytes_allowance: *max_size,
								transactions_used: 0,
								transactions_allowance: 1,
							},
						}),
						next: None,
					},
				);
				Pallet::<T>::authorization_added(&scope);
			}
		}
	}

	#[pallet::inherent]
	impl<T: Config> ProvideInherent for Pallet<T> {
		type Call = Call<T>;
		type Error = InherentError;
		const INHERENT_IDENTIFIER: InherentIdentifier = INHERENT_IDENTIFIER;

		fn create_inherent(data: &InherentData) -> Option<Self::Call> {
			let proof = data
				.get_data::<TransactionStorageProof>(&Self::INHERENT_IDENTIFIER)
				.unwrap_or(None);
			proof.map(|proof| Call::check_proof { proof })
		}

		fn check_inherent(_call: &Self::Call, _data: &InherentData) -> Result<(), Self::Error> {
			Ok(())
		}

		fn is_inherent(call: &Self::Call) -> bool {
			matches!(call, Call::check_proof { .. })
		}
	}

	#[pallet::validate_unsigned]
	impl<T: Config> ValidateUnsigned for Pallet<T> {
		type Call = Call<T>;

		fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
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
		fn do_store(
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
			sp_io::transaction_index::index(extrinsic_index, data_len, cid.content_hash);

			let mut index = 0;
			<BlockTransactions<T>>::mutate(|transactions| {
				if transactions.len() + 1 > T::MaxBlockTransactions::get() as usize {
					return Err(Error::<T>::TooManyTransactions);
				}
				let total_chunks = TransactionInfo::total_chunks(transactions) + chunk_count;
				index = transactions.len() as u32;
				transactions
					.try_push(TransactionInfo {
						chunk_root: root,
						size: data_len,
						content_hash: cid.content_hash,
						hashing,
						cid_codec,
						block_chunks: total_chunks,
					})
					.map_err(|_| Error::<T>::TooManyTransactions)
			})?;

			Self::deposit_event(Event::Stored {
				index,
				content_hash: cid.content_hash,
				cid: cid.to_bytes(),
			});

			Ok(())
		}

		/// Returns the current period, derived from the shared unix-time clock.
		pub fn current_period() -> u32 {
			let now_secs = T::TimeProvider::now().as_secs();
			let period_duration = T::PeriodDuration::get();
			// PeriodDuration is guaranteed non-zero by `integrity_test`.
			(now_secs / period_duration) as u32
		}

		/// Prune expired slots and shift `next` into `current` when `next.period`
		/// matches the current period.
		///
		/// Returns `true` if the authorization should be removed entirely (both slots
		/// are empty after pruning). Caller is responsible for running the removal and
		/// calling [`authorization_removed`].
		fn prune_and_shift(auth: &mut Authorization, current: u32) -> bool {
			if let Some(g) = &auth.current {
				if g.period < current {
					auth.current = None;
				}
			}
			if let Some(g) = &auth.next {
				if g.period < current {
					auth.next = None;
				}
			}
			if auth.current.is_none() {
				if let Some(g) = &auth.next {
					if g.period == current {
						auth.current = auth.next.take();
					}
				}
			}
			auth.is_empty()
		}

		fn authorization_added(scope: &AuthorizationScopeFor<T>) {
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

		/// Authorize data storage for a specific period.
		///
		/// `for_period` must be the current period or the next one; any other value is
		/// rejected with [`Error::InvalidPeriod`]. After pruning/shifting, the grant is
		/// routed to the corresponding slot. Repeated authorizations for the same slot
		/// (e.g. multiple People-Chain claims for the same period from different aliases)
		/// **add** to the existing allowances; used counters are preserved.
		fn authorize(
			scope: AuthorizationScopeFor<T>,
			transactions: u32,
			bytes: u64,
			for_period: u32,
		) -> DispatchResult {
			let current = Self::current_period();
			let is_current_slot = match for_period.checked_sub(current) {
				Some(0) => true,
				Some(1) => false,
				_ => return Err(Error::<T>::InvalidPeriod.into()),
			};

			let newly_added = Authorizations::<T>::mutate(&scope, |maybe_authorization| {
				let is_new = maybe_authorization.is_none();
				let auth = maybe_authorization
					.get_or_insert_with(|| Authorization { current: None, next: None });
				Self::prune_and_shift(auth, current);
				let slot = if is_current_slot { &mut auth.current } else { &mut auth.next };
				match slot {
					Some(existing) => {
						debug_assert_eq!(existing.period, for_period);
						existing.extent.bytes_allowance = existing
							.extent
							.bytes_allowance
							.saturating_add(bytes);
						existing.extent.transactions_allowance = existing
							.extent
							.transactions_allowance
							.saturating_add(transactions);
					},
					None =>
						*slot = Some(PeriodGrant {
							period: for_period,
							extent: AuthorizationExtent {
								bytes: 0,
								bytes_permanent: 0,
								bytes_allowance: bytes,
								transactions_used: 0,
								transactions_allowance: transactions,
							},
						}),
				}
				is_new
			});
			if newly_added {
				Self::authorization_added(&scope);
			}
			Ok(())
		}

		/// Remove an authorization that has no live or future grants left.
		fn remove_expired_authorization(scope: AuthorizationScopeFor<T>) -> DispatchResult {
			let current = Self::current_period();
			let remove = Authorizations::<T>::try_mutate(
				&scope,
				|maybe_authorization| -> Result<bool, DispatchError> {
					let auth = maybe_authorization
						.as_mut()
						.ok_or(Error::<T>::AuthorizationNotFound)?;
					Self::prune_and_shift(auth, current);
					if auth.is_empty() {
						*maybe_authorization = None;
						Ok(true)
					} else {
						Err(Error::<T>::AuthorizationNotExpired.into())
					}
				},
			)?;
			if remove {
				Self::authorization_removed(&scope);
			}
			Ok(())
		}

		/// Returns the effective extent of the current period's grant for `scope`.
		///
		/// Reads the persisted [`Authorization`] without mutating it, then resolves which
		/// slot is "live" for `current_period()`: prefer `current` if its period matches,
		/// otherwise fall back to `next` (which would be promoted on a real access).
		/// Any other state — both slots empty, or neither slot's period matches —
		/// returns a zero extent.
		fn authorization_extent(scope: AuthorizationScopeFor<T>) -> AuthorizationExtent {
			let Some(authorization) = Authorizations::<T>::get(&scope) else {
				return AuthorizationExtent::default();
			};
			let current = Self::current_period();
			authorization
				.current
				.filter(|g| g.period == current)
				.or_else(|| authorization.next.filter(|g| g.period == current))
				.map(|g| g.extent)
				.unwrap_or_default()
		}

		/// Returns the (unused and unexpired) authorization extent for the given account.
		pub fn account_authorization_extent(who: T::AccountId) -> AuthorizationExtent {
			Self::authorization_extent(AuthorizationScope::Account(who))
		}

		/// Returns the (unused and unexpired) authorization extent for the given content hash.
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

		/// Returns `true` if a blob of the given size can be stored.
		fn data_size_ok(size: usize) -> bool {
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

		/// Returns `true` if no more store/renew transactions can be included in the current
		/// block.
		fn block_transactions_full() -> bool {
			BlockTransactions::<T>::decode_len()
				.is_some_and(|len| len >= T::MaxBlockTransactions::get() as usize)
		}

		/// Check that the authorization for `scope` permits a `store`/`renew` of `size`
		/// bytes in the current period.
		///
		/// Always runs prune-and-shift first. Rejects if there's no live grant for the
		/// current period. For **renew** (`is_renew = true`), hard-rejects when the call
		/// would push `bytes + bytes_permanent` past `bytes_allowance` or when
		/// `transactions_used + 1` would exceed `transactions_allowance`. For **store**
		/// (`is_renew = false`), never rejects on those caps — `bytes` is `saturating_add`-ed
		/// and `transactions_used` is bumped only when the post-tx state is in-budget on
		/// both axes.
		///
		/// When `consume = false` (validate phase), the same rejection rules apply but
		/// no counters are mutated.
		fn check_authorization(
			scope: &AuthorizationScopeFor<T>,
			size: u32,
			consume: bool,
			is_renew: bool,
		) -> Result<(), TransactionValidityError> {
			let current = Self::current_period();
			let check = |maybe_authorization: &mut Option<Authorization>|
			 -> Result<(), TransactionValidityError> {
				let Some(authorization) = maybe_authorization.as_mut() else {
					return Err(InvalidTransaction::Payment.into())
				};
				Self::prune_and_shift(authorization, current);
				let Some(grant) = authorization.current.as_mut() else {
					return Err(InvalidTransaction::Payment.into())
				};
				let extent = &mut grant.extent;
				// Prospective post-tx state, identical for store and renew because both
				// consume from the shared per-period byte budget.
				let combined_after = extent
					.bytes
					.saturating_add(extent.bytes_permanent)
					.saturating_add(size.into());
				let tx_after = extent.transactions_used.saturating_add(1);
				let in_budget = combined_after <= extent.bytes_allowance
					&& tx_after <= extent.transactions_allowance;

				if is_renew && !in_budget {
					// Hard cap on renew: combined bytes or tx-count would exceed allowance.
					return Err(InvalidTransaction::Payment.into());
				}

				if consume {
					if is_renew {
						extent.bytes_permanent = extent.bytes_permanent.saturating_add(size.into());
						extent.transactions_used = tx_after;
					} else {
						// Soft cap: store always saturates into `bytes`. Only count toward
						// `transactions_used` when the post-tx state is in-budget on both axes.
						extent.bytes = extent.bytes.saturating_add(size.into());
						if in_budget {
							extent.transactions_used = tx_after;
						}
					}
				}
				Ok(())
			};

			if consume {
				Authorizations::<T>::mutate(scope, check)
			} else {
				let mut authorization = Authorizations::<T>::get(scope);
				check(&mut authorization)
			}
		}

		/// Check that an authorization exists for `scope` but has no live or future
		/// grants left (after prune-and-shift), and is therefore safe to remove.
		fn check_authorization_expired(
			scope: &AuthorizationScopeFor<T>,
		) -> Result<(), TransactionValidityError> {
			let Some(mut authorization) = Authorizations::<T>::get(scope) else {
				return Err(AUTHORIZATION_NOT_FOUND.into());
			};
			let current = Self::current_period();
			Self::prune_and_shift(&mut authorization, current);
			if authorization.is_empty() {
				Ok(())
			} else {
				Err(AUTHORIZATION_NOT_EXPIRED.into())
			}
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
				Call::<T>::renew { block, index } => {
					let info = Self::transaction_info(*block, *index).ok_or(RENEWED_NOT_FOUND)?;
					Self::check_store_renew_unsigned(
						info.size as usize,
						|| info.content_hash,
						context,
						true,
					)
				},
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
				Call::<T>::renew { block, index } => {
					let info = Self::transaction_info(*block, *index).ok_or(RENEWED_NOT_FOUND)?;
					(info.size as usize, info.content_hash, true)
				},
				Call::<T>::authorize_account { .. } |
				Call::<T>::authorize_preimage { .. } => {
					// Verify that the signer satisfies the Authorizer origin.
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
					(
						ValidTransaction::with_tag_prefix("TransactionStorageCheckedSigned")
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

	/// Verify that every stored authorization has at least one populated slot, and that
	/// every populated slot has a non-zero `bytes_allowance` and `transactions_allowance`.
	/// The `bytes` (used) counter can exceed `bytes_allowance` — that just disables the
	/// priority boost — but the cap and tx-budget themselves must always be positive.
	fn check_authorizations_integrity() -> Result<(), sp_runtime::TryRuntimeError> {
		for (_, authorization) in Authorizations::<T>::iter() {
			ensure!(!authorization.is_empty(), "Stored authorization has no live or pending grant");
			for grant in [&authorization.current, &authorization.next].iter().filter_map(|s| s.as_ref()) {
				ensure!(
					grant.extent.bytes_allowance > 0,
					"Stored authorization grant has zero bytes_allowance"
				);
				ensure!(
					grant.extent.transactions_allowance > 0,
					"Stored authorization grant has zero transactions_allowance"
				);
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

	// 5. check_proof (DispatchClass::Mandatory, once per block) must fit in max block.
	let check_proof_weight = T::WeightInfo::check_proof();
	assert!(
		check_proof_weight.all_lte(block_weights.max_block),
		"check_proof weight {check_proof_weight:?} exceeds max block {:?}",
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
	println!("  check_proof weight:         {check_proof_weight:?}");
	println!("  Max store txs by weight:    {max_txs_by_weight}");
	println!("  Max store txs by length:    {}", normal_length / per_tx_size);
}
