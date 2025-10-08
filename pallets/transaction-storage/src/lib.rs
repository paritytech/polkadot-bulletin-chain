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
//! `SignedExtension` implementation that calls the [`validate_signed`](Pallet::validate_signed)
//! and [`pre_dispatch_signed`](Pallet::pre_dispatch_signed) functions.

// Ensure we're `no_std` when compiling for Wasm.
#![cfg_attr(not(feature = "std"), no_std)]

mod benchmarking;
pub mod weights;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use codec::{Decode, Encode, MaxEncodedLen};
use polkadot_sdk_frame::{
	deps::{sp_core::sp_std::prelude::*, *},
	prelude::*,
};
use sp_transaction_storage_proof::{
	encode_index, random_chunk, InherentError, TransactionStorageProof, CHUNK_SIZE,
	INHERENT_IDENTIFIER,
};

// Re-export pallet items so that they can be accessed from the crate namespace.
pub use pallet::*;
pub use weights::WeightInfo;

const LOG_TARGET: &str = "runtime::transaction-storage";

/// Maximum bytes that can be stored in one transaction.
// TODO: find out what is "allocator" and "allocator limit"
// Setting higher limit also requires raising the allocator limit.
// TODO: not used, can we remove or use?
pub const DEFAULT_MAX_TRANSACTION_SIZE: u32 = 8 * 1024 * 1024;
// TODO: not used, can we remove or use?
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

/// Number of transactions and bytes covered by an authorization.
#[derive(PartialEq, Eq, RuntimeDebug, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
pub struct AuthorizationExtent {
	/// Number of transactions.
	pub transactions: u32,
	/// Number of bytes.
	pub bytes: u64,
}

/// Hash of a stored blob of data.
type ContentHash = [u8; 32];

/// The scope of an authorization.
#[derive(Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
enum AuthorizationScope<AccountId> {
	/// Authorization for the given account to store arbitrary data.
	Account(AccountId),
	/// Authorization for anyone to store data with a specific hash.
	Preimage(ContentHash),
}

type AuthorizationScopeFor<T> = AuthorizationScope<<T as frame_system::Config>::AccountId>;

/// An authorization to store data.
#[derive(Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
struct Authorization<BlockNumber> {
	/// Extent of the authorization (number of transactions/bytes).
	extent: AuthorizationExtent,
	/// The block at which this authorization expires.
	expiration: BlockNumber,
}

type AuthorizationFor<T> = Authorization<BlockNumberFor<T>>;

/// State data for a stored transaction.
#[derive(
	Encode, Decode, Clone, RuntimeDebug, PartialEq, Eq, scale_info::TypeInfo, MaxEncodedLen,
)]
pub struct TransactionInfo {
	/// Chunk trie root.
	chunk_root: <BlakeTwo256 as Hash>::Output,
	/// Plain hash of indexed data.
	content_hash: <BlakeTwo256 as Hash>::Output,
	/// Size of indexed data in bytes.
	size: u32,
	/// Total number of chunks added in the block with this transaction. This
	/// is used find transaction info by block chunk index using binary search.
	block_chunks: u32,
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

fn num_chunks(bytes: u32) -> u32 {
	(bytes as u64).div_ceil(CHUNK_SIZE as u64) as u32
}

#[polkadot_sdk_frame::pallet]
pub mod pallet {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// The overarching event type.
		#[allow(deprecated)]
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// Weight information for extrinsics in this pallet.
		type WeightInfo: WeightInfo;
		/// Maximum number of indexed transactions in a block.
		#[pallet::constant]
		type MaxBlockTransactions: Get<u32>;
		/// Maximum data set in a single transaction in bytes.
		#[pallet::constant]
		type MaxTransactionSize: Get<u32>;
		/// Storage period for data in blocks. Should match
		/// [`DEFAULT_STORAGE_PERIOD`](sp_transaction_storage_proof::DEFAULT_STORAGE_PERIOD) for
		/// block authoring.
		#[pallet::constant]
		type StoragePeriod: Get<BlockNumberFor<Self>>;
		/// Authorizations expire after this many blocks.
		#[pallet::constant]
		type AuthorizationPeriod: Get<BlockNumberFor<Self>>;
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
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Attempted to call `store`/`renew` outside of block execution.
		BadContext,
		/// Data size is not in the allowed range.
		BadDataSize,
		/// Too many transactions in the block.
		TooManyTransactions,
		/// Renewed extrinsic not found.
		RenewedNotFound,
		/// Proof was not expected in this block.
		UnexpectedProof,
		/// Proof failed verification.
		InvalidProof,
		/// Unable to verify proof becasue state data is missing.
		MissingStateData,
		/// Double proof check in the block.
		DoubleCheck,
		/// Authorization was not found.
		AuthorizationNotFound,
		/// Authorization has not expired.
		AuthorizationNotExpired,
	}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(n: BlockNumberFor<T>) -> Weight {
			let mut weight = Weight::zero();
			let db_weight = T::DbWeight::get();

			// Drop obsolete roots. The proof for `obsolete` will be checked later
			// in this block, so we drop `obsolete` - 1.
			weight.saturating_accrue(db_weight.reads(1));
			let period = T::StoragePeriod::get();
			let obsolete = n.saturating_sub(period.saturating_add(One::one()));
			if obsolete > Zero::zero() {
				weight.saturating_accrue(db_weight.writes(2));
				<Transactions<T>>::remove(obsolete);
				<ChunkCount<T>>::remove(obsolete);
			}

			// For `on_finalize`
			weight.saturating_accrue(db_weight.reads_writes(2, 2));

			weight
		}

		fn on_finalize(n: BlockNumberFor<T>) {
			assert!(
				<ProofChecked<T>>::take() || {
					// Proof is not required for early or empty blocks.
					let number = <frame_system::Pallet<T>>::block_number();
					let period = T::StoragePeriod::get();
					let target_number = number.saturating_sub(period);
					target_number.is_zero() || <ChunkCount<T>>::get(target_number) == 0
				},
				"Storage proof must be checked once in the block"
			);

			// Insert new transactions
			let transactions = <BlockTransactions<T>>::take();
			let total_chunks = transactions.last().map_or(0, |t| t.block_chunks);
			if total_chunks != 0 {
				<ChunkCount<T>>::insert(n, total_chunks);
				<Transactions<T>>::insert(n, transactions);
			}
		}

		fn integrity_test() {
			assert!(
				!T::MaxBlockTransactions::get().is_zero(),
				"Not useful if data cannot be stored"
			);
			assert!(!T::MaxTransactionSize::get().is_zero(), "Not useful if data cannot be stored");
			assert!(!T::StoragePeriod::get().is_zero(), "Not useful if data is not stored");
			assert!(
				!T::AuthorizationPeriod::get().is_zero(),
				"Not useful if authorizations are never valid"
			);
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Index and store data off chain. Minimum data size is 1 bytes, maximum is
		/// `MaxTransactionSize`. Data will be removed after `StoragePeriod` blocks, unless `renew`
		/// is called.
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
		pub fn store(_origin: OriginFor<T>, data: Vec<u8>) -> DispatchResult {
			// In the case of a regular unsigned transaction, this should have been checked by
			// pre_dispatch. In the case of a regular signed transaction, this should have been
			// checked by pre_dispatch_signed.
			ensure!(Self::data_size_ok(data.len()), Error::<T>::BadDataSize);

			// Chunk data and compute storage root
			let chunks: Vec<_> = data.chunks(CHUNK_SIZE).map(|c| c.to_vec()).collect();
			let chunk_count = chunks.len();
			debug_assert_eq!(chunk_count, num_chunks(data.len() as u32) as usize);
			let root = sp_io::trie::blake2_256_ordered_root(chunks, sp_runtime::StateVersion::V1);

			let extrinsic_index =
				<frame_system::Pallet<T>>::extrinsic_index().ok_or(Error::<T>::BadContext)?;
			let content_hash = sp_io::hashing::blake2_256(&data);
			sp_io::transaction_index::index(extrinsic_index, data.len() as u32, content_hash);

			let mut index = 0;
			<BlockTransactions<T>>::mutate(|transactions| {
				let total_chunks =
					transactions.last().map_or(0, |t| t.block_chunks) + (chunk_count as u32);
				index = transactions.len() as u32;
				transactions
					.try_push(TransactionInfo {
						chunk_root: root,
						size: data.len() as u32,
						content_hash: content_hash.into(),
						block_chunks: total_chunks,
					})
					.map_err(|_| Error::<T>::TooManyTransactions)
			})?;
			Self::deposit_event(Event::Stored { index });
			Ok(())
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
			_origin: OriginFor<T>,
			block: BlockNumberFor<T>,
			index: u32,
		) -> DispatchResultWithPostInfo {
			let info = Self::transaction_info(block, index).ok_or(Error::<T>::RenewedNotFound)?;

			// In the case of a regular unsigned transaction, this should have been checked by
			// pre_dispatch. In the case of a regular signed transaction, this should have been
			// checked by pre_dispatch_signed.
			ensure!(Self::data_size_ok(info.size as usize), Error::<T>::BadDataSize);

			let extrinsic_index =
				<frame_system::Pallet<T>>::extrinsic_index().ok_or(Error::<T>::BadContext)?;
			sp_io::transaction_index::renew(extrinsic_index, info.content_hash.into());

			let mut index = 0;
			<BlockTransactions<T>>::mutate(|transactions| {
				let chunks = num_chunks(info.size);
				let total_chunks = transactions.last().map_or(0, |t| t.block_chunks) + chunks;
				index = transactions.len() as u32;
				transactions
					.try_push(TransactionInfo {
						chunk_root: info.chunk_root,
						size: info.size,
						content_hash: info.content_hash,
						block_chunks: total_chunks,
					})
					.map_err(|_| Error::<T>::TooManyTransactions)
			})?;
			Self::deposit_event(Event::Renewed { index });
			Ok(().into())
		}

		/// Check storage proof for block number `block_number() - StoragePeriod`. If such a block
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
			let number = <frame_system::Pallet<T>>::block_number();
			let period = T::StoragePeriod::get();
			let target_number = number.saturating_sub(period);
			ensure!(!target_number.is_zero(), Error::<T>::UnexpectedProof);
			let total_chunks = <ChunkCount<T>>::get(target_number);
			ensure!(total_chunks != 0, Error::<T>::UnexpectedProof);
			let parent_hash = <frame_system::Pallet<T>>::parent_hash();
			let selected_chunk_index = random_chunk(parent_hash.as_ref(), total_chunks);
			let (info, chunk_index) = match <Transactions<T>>::get(target_number) {
				Some(infos) => {
					let index = match infos
						.binary_search_by_key(&selected_chunk_index, |info| info.block_chunks)
					{
						Ok(index) => index,
						Err(index) => index,
					};
					let info = infos.get(index).ok_or(Error::<T>::MissingStateData)?.clone();
					let chunks = num_chunks(info.size);
					let prev_chunks = info.block_chunks - chunks;
					(info, selected_chunk_index - prev_chunks)
				},
				None => return Err(Error::<T>::MissingStateData.into()),
			};
			ensure!(
				sp_io::trie::blake2_256_verify_proof(
					info.chunk_root,
					&proof.proof,
					&encode_index(chunk_index),
					&proof.chunk,
					sp_runtime::StateVersion::V1,
				),
				Error::<T>::InvalidProof
			);
			ProofChecked::<T>::put(true);
			Self::deposit_event(Event::ProofChecked);
			Ok(().into())
		}

		/// Authorize an account to store up to a given amount of arbitrary data. The authorization
		/// will expire after a configured number of blocks.
		///
		/// If the account is already authorized to store data, this will increase the amount of
		/// data the account is authorized to store (and the number of transactions the account may
		/// submit to supply the data), and push back the expiration block.
		///
		/// Parameters:
		///
		/// - `who`: The account to be credited with an authorization to store data.
		/// - `transactions`: The number of transactions that `who` may submit to supply that data.
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
			T::Authorizer::ensure_origin(origin)?;
			Self::authorize(AuthorizationScope::Account(who.clone()), transactions, bytes);
			Self::deposit_event(Event::AccountAuthorized { who, transactions, bytes });
			Ok(())
		}

		/// Authorize anyone to store a preimage of the given BLAKE2b hash. The authorization will
		/// expire after a configured number of blocks.
		///
		/// If authorization already exists for a preimage of the given hash to be stored, the
		/// maximum size of the preimage will be increased to `max_size`, and the expiration block
		/// will be pushed back.
		///
		/// Parameters:
		///
		/// - `hash`: The BLAKE2b hash of the data to be submitted.
		/// - `max_size`: The maximum size, in bytes, of the preimage.
		///
		/// The origin for this call must be the pallet's `Authorizer`. Emits
		/// [`PreimageAuthorized`](Event::PreimageAuthorized) when successful.
		#[pallet::call_index(4)]
		#[pallet::weight(T::WeightInfo::authorize_preimage())]
		pub fn authorize_preimage(
			origin: OriginFor<T>,
			hash: ContentHash,
			max_size: u64,
		) -> DispatchResult {
			T::Authorizer::ensure_origin(origin)?;
			Self::authorize(AuthorizationScope::Preimage(hash), 1, max_size);
			Self::deposit_event(Event::PreimageAuthorized { hash, max_size });
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
		/// - `hash`: The BLAKE2b hash that was authorized.
		///
		/// Emits
		/// [`ExpiredPreimageAuthorizationRemoved`](Event::ExpiredPreimageAuthorizationRemoved)
		/// when successful.
		#[pallet::call_index(6)]
		#[pallet::weight(T::WeightInfo::remove_expired_preimage_authorization())]
		pub fn remove_expired_preimage_authorization(
			_origin: OriginFor<T>,
			hash: ContentHash,
		) -> DispatchResult {
			Self::remove_expired_authorization(AuthorizationScope::Preimage(hash))?;
			Self::deposit_event(Event::ExpiredPreimageAuthorizationRemoved { hash });
			Ok(())
		}

		/// Refresh the expiration of an existing authorization for an account.
		///
		/// If the account does not have an authorization, the call will fail.
		///
		/// Parameters:
		///
		/// - `who`: The account to be credited with an authorization to store data.
		///
		/// The origin for this call must be the pallet's `Authorizer`. Emits
		/// [`AccountAuthorizationRefreshed`](Event::AccountAuthorizationRefreshed) when successful.
		#[pallet::call_index(7)]
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

		/// Refresh the expiration of an existing authorization for a preimage of a BLAKE2b hash.
		///
		/// If the preimage does not have an authorization, the call will fail.
		///
		/// Parameters:
		///
		/// - `hash`: The BLAKE2b hash of the data to be submitted.
		///
		/// The origin for this call must be the pallet's `Authorizer`. Emits
		/// [`PreimageAuthorizationRefreshed`](Event::PreimageAuthorizationRefreshed) when
		/// successful.
		#[pallet::call_index(8)]
		#[pallet::weight(T::WeightInfo::refresh_preimage_authorization())]
		pub fn refresh_preimage_authorization(
			origin: OriginFor<T>,
			hash: ContentHash,
		) -> DispatchResult {
			T::Authorizer::ensure_origin(origin)?;
			Self::refresh_authorization(AuthorizationScope::Preimage(hash))?;
			Self::deposit_event(Event::PreimageAuthorizationRefreshed { hash });
			Ok(())
		}
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Stored data under specified index.
		Stored { index: u32 },
		/// Renewed data under specified index.
		Renewed { index: u32 },
		/// Storage proof was successfully checked.
		ProofChecked,
		/// An account `who` was authorized to store `bytes` bytes in `transactions` transactions.
		AccountAuthorized { who: T::AccountId, transactions: u32, bytes: u64 },
		/// An authorization for account `who` was refreshed.
		AccountAuthorizationRefreshed { who: T::AccountId },
		/// Authorization was given for a preimage of `hash` (not exceeding `max_size`) to be
		/// stored by anyone.
		PreimageAuthorized { hash: ContentHash, max_size: u64 },
		/// An authorization for a preimage of `hash` was refreshed.
		PreimageAuthorizationRefreshed { hash: ContentHash },
		/// An expired account authorization was removed.
		ExpiredAccountAuthorizationRemoved { who: T::AccountId },
		/// An expired preimage authorization was removed.
		ExpiredPreimageAuthorizationRemoved { hash: ContentHash },
	}

	/// Authorizations, keyed by scope.
	#[pallet::storage]
	pub(super) type Authorizations<T: Config> =
		StorageMap<_, Blake2_128Concat, AuthorizationScopeFor<T>, AuthorizationFor<T>, OptionQuery>;

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

	/// Count indexed chunks for each block.
	#[pallet::storage]
	pub(super) type ChunkCount<T: Config> =
		StorageMap<_, Blake2_128Concat, BlockNumberFor<T>, u32, ValueQuery>;

	// Intermediates
	#[pallet::storage]
	pub(super) type BlockTransactions<T: Config> =
		StorageValue<_, BoundedVec<TransactionInfo, T::MaxBlockTransactions>, ValueQuery>;

	/// Was the proof checked in this block?
	#[pallet::storage]
	pub(super) type ProofChecked<T: Config> = StorageValue<_, bool, ValueQuery>;

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
			Self::check_unsigned(call, CheckContext::PreDispatch).map(|_| ())
		}
	}

	impl<T: Config> Pallet<T> {
		/// Returns `true` if the system is beyond the given expiration point.
		fn expired(expiration: BlockNumberFor<T>) -> bool {
			let now = frame_system::Pallet::<T>::block_number();
			now >= expiration
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
						log::warn!(
							target: LOG_TARGET,
							"Failed to decrement provider reference count for authorized account {who:?}, \
							leaking reference: {err:?}"
						);
					}
				},
				AuthorizationScope::Preimage(_) => (),
			}
		}

		/// Authorize data storage.
		fn authorize(scope: AuthorizationScopeFor<T>, transactions: u32, bytes: u64) {
			let expiration = frame_system::Pallet::<T>::block_number()
				.saturating_add(T::AuthorizationPeriod::get());

			Authorizations::<T>::mutate(&scope, |maybe_authorization| {
				if let Some(authorization) = maybe_authorization {
					if Self::expired(authorization.expiration) {
						// Previous authorization expired. Overwrite it.
						*authorization = Authorization {
							extent: AuthorizationExtent { transactions, bytes },
							expiration,
						};
					} else {
						// An unexpired authorization already exists. Extend it.
						match scope {
							AuthorizationScope::Account(_) => {
								// Add
								authorization.extent.transactions =
									authorization.extent.transactions.saturating_add(transactions);
								authorization.extent.bytes =
									authorization.extent.bytes.saturating_add(bytes);
							},
							AuthorizationScope::Preimage(_) => {
								// Max
								authorization.extent.transactions =
									authorization.extent.transactions.max(transactions);
								authorization.extent.bytes = authorization.extent.bytes.max(bytes);
							},
						}
						authorization.expiration = expiration;
					}
				} else {
					// No previous authorization. Create a fresh one.
					*maybe_authorization = Some(Authorization {
						extent: AuthorizationExtent { transactions, bytes },
						expiration,
					});
					Self::authorization_added(&scope);
				}
			});
		}

		/// Authorize data storage.
		fn refresh_authorization(scope: AuthorizationScopeFor<T>) -> DispatchResult {
			let expiration = frame_system::Pallet::<T>::block_number()
				.saturating_add(T::AuthorizationPeriod::get());

			Authorizations::<T>::mutate(&scope, |maybe_authorization| {
				if let Some(authorization) = maybe_authorization {
					authorization.expiration = expiration;
					Ok(())
				} else {
					// No previous authorization to refresh.
					Err(Error::<T>::AuthorizationNotFound.into())
				}
			})
		}

		/// Remove an expired authorization.
		fn remove_expired_authorization(scope: AuthorizationScopeFor<T>) -> DispatchResult {
			// In the case of a regular unsigned transaction, pre_dispatch should have checked that
			// the authorization exists and has expired
			let Some(authorization) = Authorizations::<T>::take(&scope) else {
				return Err(Error::<T>::AuthorizationNotFound.into())
			};
			ensure!(Self::expired(authorization.expiration), Error::<T>::AuthorizationNotExpired);
			Self::authorization_removed(&scope);
			Ok(())
		}

		fn authorization_extent(scope: AuthorizationScopeFor<T>) -> AuthorizationExtent {
			let Some(authorization) = Authorizations::<T>::get(&scope) else {
				return AuthorizationExtent { transactions: 0, bytes: 0 }
			};
			if Self::expired(authorization.expiration) {
				AuthorizationExtent { transactions: 0, bytes: 0 }
			} else {
				authorization.extent
			}
		}

		/// Returns the (unused and unexpired) authorization extent for the given account.
		pub fn account_authorization_extent(who: T::AccountId) -> AuthorizationExtent {
			Self::authorization_extent(AuthorizationScope::Account(who))
		}

		/// Returns the (unused and unexpired) authorization extent for the given content hash.
		pub fn preimage_authorization_extent(hash: ContentHash) -> AuthorizationExtent {
			Self::authorization_extent(AuthorizationScope::Preimage(hash))
		}

		/// Returns the validity of the given call, signed by the given account.
		///
		/// This is equivalent to `validate_unsigned` but for signed transactions. It should be
		/// called from a `SignedExtension` implementation.
		pub fn validate_signed(who: &T::AccountId, call: &Call<T>) -> TransactionValidity {
			Self::check_signed(who, call, CheckContext::Validate)?.ok_or(IMPOSSIBLE.into())
		}

		/// Check the validity of the given call, signed by the given account, and consume
		/// authorization for it.
		///
		/// This is equivalent to `pre_dispatch` but for signed transactions. It should be called
		/// from a `SignedExtension` implementation.
		pub fn pre_dispatch_signed(
			who: &T::AccountId,
			call: &Call<T>,
		) -> Result<(), TransactionValidityError> {
			Self::check_signed(who, call, CheckContext::PreDispatch).map(|_| ())
		}

		/// Returns `true` if a blob of the given size can be stored.
		fn data_size_ok(size: usize) -> bool {
			(size > 0) && (size <= T::MaxTransactionSize::get() as usize)
		}

		/// Returns the [`TransactionInfo`] for the specified store/renew transaction.
		fn transaction_info(
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

		/// Check that authorization exists for data of the given size to be stored in a single
		/// transaction. If `consume` is `true`, the authorization is consumed.
		fn check_authorization(
			scope: AuthorizationScopeFor<T>,
			size: u32,
			consume: bool,
		) -> Result<(), TransactionValidityError> {
			// Returns true if authorization was removed
			let consume_authorization = |maybe_authorization: &mut Option<Authorization<_>>| -> Result<bool, TransactionValidityError> {
				let Some(authorization) = maybe_authorization else {
					return Err(InvalidTransaction::Payment.into())
				};
				if Self::expired(authorization.expiration) {
					return Err(InvalidTransaction::Payment.into())
				}

				let transactions = authorization
					.extent
					.transactions
					.checked_sub(1)
					.ok_or(InvalidTransaction::Payment)?;
				let bytes = authorization
					.extent
					.bytes
					.checked_sub(size.into())
					.ok_or(InvalidTransaction::Payment)?;

				// Authorization is sufficient. Remove if _either_ no transactions left or no bytes
				// left.
				if transactions == 0 || bytes == 0 {
					*maybe_authorization = None;
					Ok(true)
				} else {
					authorization.extent.transactions = transactions;
					authorization.extent.bytes = bytes;
					Ok(false)
				}
			};

			if consume {
				if Authorizations::<T>::mutate(&scope, consume_authorization)? {
					Self::authorization_removed(&scope);
				}
			} else {
				// Note we call consume_authorization on a temporary; the authorization in storage
				// is untouched and doesn't actually get consumed
				let mut authorization = Authorizations::<T>::get(&scope);
				consume_authorization(&mut authorization)?;
			}

			Ok(())
		}

		/// Check that authorization with the given scope exists in storage but has expired.
		fn check_authorization_expired(
			scope: AuthorizationScopeFor<T>,
		) -> Result<(), TransactionValidityError> {
			let Some(authorization) = Authorizations::<T>::get(&scope) else {
				return Err(AUTHORIZATION_NOT_FOUND.into())
			};
			if Self::expired(authorization.expiration) {
				Ok(())
			} else {
				Err(AUTHORIZATION_NOT_EXPIRED.into())
			}
		}

		fn check_store_renew_unsigned(
			size: usize,
			hash: impl FnOnce() -> ContentHash,
			context: CheckContext,
		) -> Result<Option<ValidTransaction>, TransactionValidityError> {
			if !Self::data_size_ok(size) {
				return Err(BAD_DATA_SIZE.into())
			}

			if Self::block_transactions_full() {
				return Err(InvalidTransaction::ExhaustsResources.into())
			}

			let hash = hash();

			Self::check_authorization(
				AuthorizationScope::Preimage(hash),
				size as u32,
				context.consume_authorization(),
			)?;

			Ok(context.want_valid_transaction().then(|| {
				ValidTransaction::with_tag_prefix("TransactionStorageStoreRenew")
					.and_provides(hash)
					.priority(T::StoreRenewPriority::get())
					.longevity(T::StoreRenewLongevity::get())
					.into()
			}))
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
				),
				Call::<T>::renew { block, index } => {
					let info = Self::transaction_info(*block, *index).ok_or(RENEWED_NOT_FOUND)?;
					Self::check_store_renew_unsigned(
						info.size as usize,
						|| info.content_hash.into(),
						context,
					)
				},
				Call::<T>::remove_expired_account_authorization { who } => {
					Self::check_authorization_expired(AuthorizationScope::Account(who.clone()))?;
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
				Call::<T>::remove_expired_preimage_authorization { hash } => {
					Self::check_authorization_expired(AuthorizationScope::Preimage(*hash))?;
					Ok(context.want_valid_transaction().then(|| {
						ValidTransaction::with_tag_prefix(
							"TransactionStorageRemoveExpiredPreimageAuthorization",
						)
						.and_provides(hash)
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
		) -> Result<Option<ValidTransaction>, TransactionValidityError> {
			let size = match call {
				Call::<T>::store { data } => data.len(),
				Call::<T>::renew { block, index } => {
					let info = Self::transaction_info(*block, *index).ok_or(RENEWED_NOT_FOUND)?;
					info.size as usize
				},
				_ => return Err(InvalidTransaction::Call.into()),
			};

			if !Self::data_size_ok(size) {
				return Err(BAD_DATA_SIZE.into())
			}

			if Self::block_transactions_full() {
				return Err(InvalidTransaction::ExhaustsResources.into())
			}

			Self::check_authorization(
				AuthorizationScope::Account(who.clone()),
				size as u32,
				context.consume_authorization(),
			)?;

			Ok(context.want_valid_transaction().then(|| ValidTransaction {
				priority: T::StoreRenewPriority::get(),
				longevity: T::StoreRenewLongevity::get(),
				..Default::default()
			}))
		}
	}
}
