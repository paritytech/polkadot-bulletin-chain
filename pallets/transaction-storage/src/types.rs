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

//! Type definitions for the transaction storage pallet.

use bulletin_transaction_storage_primitives::{
	cids::{CidCodec, HashingAlgorithm},
	ContentHash,
};
use codec::{Decode, Encode, MaxEncodedLen};
use polkadot_sdk_frame::{
	deps::*,
	prelude::*,
	traits::fungible::{Credit, Inspect},
};
use sp_transaction_storage_proof::ChunkIndex;

use crate::Config;

/// A type alias for the balance type from this pallet's point of view.
pub(crate) type BalanceOf<T> =
	<<T as Config>::Currency as Inspect<<T as frame_system::Config>::AccountId>>::Balance;

pub type CreditOf<T> = Credit<<T as frame_system::Config>::AccountId, <T as Config>::Currency>;

/// Usage state of an authorization. All four counters reset to `0` when the authorization
/// is (re-)granted on the expired-but-present path, so they measure consumption **within
/// the current authorization window** — not lifetime on-chain footprint:
///
/// - `bytes` / `transactions` — soft side (priority signal). Saturate upward on every `store`;
///   never gate.
/// - `bytes_permanent` — hard side (per-window renew quota). Increments on every `renew`, gates
///   with [`crate::Error::PermanentAllowanceExceeded`] when `bytes_permanent + size >
///   bytes_allowance`. Never decrements; the chain-wide [`crate::PermanentStorageUsed`] counter is
///   the source of truth for renewed on-chain bytes.
/// - `bytes_allowance` / `transactions_allowance` — caps set at grant time. `bytes_allowance` is
///   shared between the soft and hard axes.
#[derive(
	Copy, Clone, PartialEq, Eq, Debug, Default, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen,
)]
pub struct AuthorizationExtent {
	/// Transactions consumed so far.
	pub transactions: u32,
	/// Total transaction allowance granted.
	pub transactions_allowance: u32,
	/// Bytes consumed by `store` calls (temporary storage).
	pub bytes: u64,
	/// Bytes consumed by `renew` calls (permanent storage).
	pub bytes_permanent: u64,
	/// Total byte allowance granted.
	pub bytes_allowance: u64,
}

impl AuthorizationExtent {
	/// Per-account renew quota check: `bytes_permanent + size <= bytes_allowance`.
	pub fn has_permanent_capacity(&self, size: u64) -> bool {
		self.bytes_permanent.saturating_add(size) <= self.bytes_allowance
	}
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

pub(crate) type AuthorizationScopeFor<T> =
	AuthorizationScope<<T as frame_system::Config>::AccountId>;

/// Describes the caller of a store/renew extrinsic after origin validation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AuthorizedCaller<AccountId> {
	/// A signed transaction whose origin was transformed to
	/// [`crate::pallet::Origin::Authorized`] by [`crate::extension::ValidateStorageCalls`].
	Signed { who: AccountId, scope: AuthorizationScope<AccountId> },
	/// A root call (e.g. via `sudo`).
	Root,
	/// An unsigned transaction validated by [`ValidateUnsigned`].
	/// TODO: replaced by https://github.com/paritytech/polkadot-bulletin-chain/pull/194
	Unsigned,
}

/// Convenience alias for [`AuthorizedCaller`] bound to a runtime's `AccountId`.
pub type AuthorizedCallerFor<T> = AuthorizedCaller<<T as frame_system::Config>::AccountId>;

/// An authorization to store data. Lifecycle by block number `now`:
/// - `now < expiration`: active — `store` and `renew` allowed.
/// - `expiration <= now < grace_until`: in grace — `renew` only.
/// - `now >= grace_until`: expired — both rejected; eligible for `remove_expired_*`.
#[derive(Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
pub(crate) struct Authorization<BlockNumber> {
	/// Extent of the authorization (number of transactions/bytes).
	pub(crate) extent: AuthorizationExtent,
	/// The block at which this authorization expires (start of grace).
	pub(crate) expiration: BlockNumber,
	/// The block at which the grace window ends. Always `>= expiration`.
	pub(crate) grace_until: BlockNumber,
}

impl<BlockNumber: PartialOrd + Copy> Authorization<BlockNumber> {
	/// `true` once `now` has reached `expiration`. While `expired(now)` is `true`
	/// and `past_grace(now)` is `false`, the authorization is in the grace window:
	/// `renew` is still allowed, `store` is not.
	pub(crate) fn expired(&self, now: BlockNumber) -> bool {
		now >= self.expiration
	}

	/// `true` once `now` has reached `grace_until`. Both `store` and `renew` are
	/// rejected; the entry is eligible for `remove_expired_*` and the
	/// expired-but-present branch of `authorize`.
	pub(crate) fn past_grace(&self, now: BlockNumber) -> bool {
		now >= self.grace_until
	}
}

pub(crate) type AuthorizationFor<T> = Authorization<BlockNumberFor<T>>;

/// Distinguishes a stored transaction created by `store` (temporary) from one created by
/// `renew` (permanent), so that `on_initialize`'s obsolete-block cleanup can decrement
/// `PermanentStorageUsed` only for the renewed entries.
#[derive(
	Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen,
)]
pub enum TransactionKind {
	Store,
	Renew,
}

/// State data for a stored transaction.
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq, scale_info::TypeInfo, MaxEncodedLen)]
pub struct TransactionInfo {
	/// Chunk trie root.
	pub(crate) chunk_root: <BlakeTwo256 as Hash>::Output,

	/// Plain hash of indexed data.
	pub content_hash: ContentHash,
	/// Used hashing algorithm for `content_hash`.
	pub hashing: HashingAlgorithm,
	/// Codec for CID.
	pub cid_codec: CidCodec,

	/// Size of indexed data in bytes.
	pub size: u32,
	/// Extrinsic index within the block that originally indexed this data
	/// (via `sp_io::transaction_index::index` / `renew`). For renewed entries
	/// this is the renewer's extrinsic index, not the original.
	pub extrinsic_index: u32,
	/// Total number of chunks added in the block with this transaction. This
	/// is used to find transaction info by block chunk index using binary search.
	///
	/// Cumulative value of all previous transactions in the block; the last transaction holds the
	/// total chunks.
	pub(crate) block_chunks: ChunkIndex,

	/// Whether the entry was created by a `store` (temporary) or a `renew` (permanent).
	/// Used by the obsolete-block cleanup in `on_initialize` to decrement the chain-wide
	/// `PermanentStorageUsed` counter for renewed bytes that have just aged out. Field
	/// is appended at the end of the struct so the v1→v2 translation is a tail-extend.
	pub kind: TransactionKind,
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
pub(crate) enum CheckContext {
	/// `validate_signed` or `validate_unsigned`.
	Validate,
	/// `pre_dispatch_signed` or `pre_dispatch`.
	PreDispatch,
}

impl CheckContext {
	/// Should authorization be consumed in this context? If not, we merely check that
	/// authorization exists.
	pub(crate) fn consume_authorization(self) -> bool {
		matches!(self, CheckContext::PreDispatch)
	}

	/// Should `check_signed`/`check_unsigned` return a `ValidTransaction`?
	pub(crate) fn want_valid_transaction(self) -> bool {
		matches!(self, CheckContext::Validate)
	}
}
