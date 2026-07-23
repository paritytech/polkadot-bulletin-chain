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

pub use bulletin_transaction_storage_primitives::TransactionRef;
use bulletin_transaction_storage_primitives::{
	cids::{CidCodec, HashingAlgorithm},
	ContentHash,
};
use codec::{Decode, Encode, MaxEncodedLen};
#[cfg(feature = "runtime-benchmarks")]
use polkadot_sdk_frame::deps::frame_benchmarking;
use polkadot_sdk_frame::{
	deps::*,
	prelude::*,
	traits::fungible::{Credit, Inspect},
};
use sp_transaction_storage_proof::ChunkIndex;

use crate::{AllowedAuthorizers, Config, Pallet};

/// A type alias for the balance type from this pallet's point of view.
pub(crate) type BalanceOf<T> =
	<<T as Config>::Currency as Inspect<<T as frame_system::Config>::AccountId>>::Balance;

pub type CreditOf<T> = Credit<<T as frame_system::Config>::AccountId, <T as Config>::Currency>;

/// Usage state of an authorization. All counters (including the opaque `extra`) reset
/// when the authorization is (re-)granted on the expired-but-present path, so they
/// measure consumption **within the current authorization window** — not lifetime
/// on-chain footprint:
///
/// - `bytes` / `transactions` — soft side (priority signal). Saturate upward on every `store`;
///   never gate.
/// - `bytes_allowance` / `transactions_allowance` — caps set at grant time. `bytes_allowance` is
///   also the cap consumer pallets gate their `extra` accounting against (the renewal pallet's
///   per-window renew quota draws on the same allowance).
/// - `extra` — opaque per-window consumption state owned by a consumer pallet
///   ([`crate::Config::AuthorizationExtra`]); this pallet only stores and resets it.
#[derive(
	Copy, Clone, PartialEq, Eq, Debug, Default, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen,
)]
pub struct AuthorizationExtent<Extra> {
	/// Transactions consumed so far.
	pub transactions: u32,
	/// Total transaction allowance granted.
	pub transactions_allowance: u32,
	/// Bytes consumed by `store` calls (temporary storage).
	pub bytes: u64,
	/// Total byte allowance granted.
	pub bytes_allowance: u64,
	/// Opaque consumer-pallet consumption state (e.g. the renewal pallet's
	/// `PermanentExtent`). Mutated only through
	/// [`crate::Pallet::try_mutate_active_authorization`].
	pub extra: Extra,
}

/// [`AuthorizationExtent`] bound to a runtime's [`crate::Config::AuthorizationExtra`].
pub type AuthorizationExtentFor<T> = AuthorizationExtent<<T as Config>::AuthorizationExtra>;

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

pub type AuthorizationScopeFor<T> = AuthorizationScope<<T as frame_system::Config>::AccountId>;

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

/// An authorization to store data.
///
/// The value shape (the `extra` field era) is tracked by
/// `pallet-bulletin-transaction-storage-renewal`'s storage version from its v2 on, not
/// by this pallet's.
#[derive(Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
pub struct Authorization<BlockNumber, Extra> {
	/// Extent of the authorization (number of transactions/bytes).
	pub extent: AuthorizationExtent<Extra>,
	/// The block at which this authorization expires.
	pub expiration: BlockNumber,
}

impl<BlockNumber: PartialOrd + Copy, Extra> Authorization<BlockNumber, Extra> {
	/// `true` once `now` has reached `expiration`; the authorization no longer
	/// permits `store`/`renew` and is eligible for `remove_expired_*`.
	pub fn expired(&self, now: BlockNumber) -> bool {
		now >= self.expiration
	}
}

pub type AuthorizationFor<T> = Authorization<BlockNumberFor<T>, <T as Config>::AuthorizationExtra>;

/// Invariant-preserving mutable view over one unexpired authorization, passed to the
/// closure of [`crate::Pallet::try_mutate_active_authorization`]. Consumer pallets get
/// full mutable access to their own opaque payload plus the one native counter a
/// consuming operation legitimately spends (a transaction slot) — never to allowances
/// or expiry.
pub struct ActiveAuthorization<'a, T: Config> {
	pub(crate) authorization: &'a mut AuthorizationFor<T>,
}

impl<T: Config> ActiveAuthorization<'_, T> {
	/// Read-only view of the extent (counters, allowances, and the opaque extra).
	pub fn extent(&self) -> &AuthorizationExtentFor<T> {
		&self.authorization.extent
	}

	/// Mutable access to the opaque consumer-pallet payload.
	pub fn extra_mut(&mut self) -> &mut T::AuthorizationExtra {
		&mut self.authorization.extent.extra
	}

	/// Consume one native transaction slot (saturating).
	pub fn note_transaction(&mut self) {
		self.authorization.extent.transactions =
			self.authorization.extent.transactions.saturating_add(1);
	}
}

/// Callback fired by [`crate::Pallet::on_initialize`] when transactions age out of the
/// `RetentionPeriod` window.
///
/// The storage pallet eagerly takes `Transactions[obsolete]`, computes `is_latest` for each
/// entry (whether [`crate::TransactionByContentHash`] still points at this `(block, index)`),
/// removes the `TransactionByContentHash` mapping for `is_latest` entries, and then hands the
/// resulting slice to this trait. Implementers — typically
/// `pallet-bulletin-transaction-storage-renewal` — inspect each entry's opaque `meta` to
/// maintain chain-wide renewed-byte counters and queue auto-renewals.
///
/// Wiring `()` (the default impl) makes the obsolete sweep a pure storage-pallet concern,
/// suitable for runtimes that omit the renewal pallet entirely.
pub trait OnObsoleteTransactions<BlockNumber, Meta> {
	/// `obsolete` is the block whose transactions just aged out. `items` are the entries
	/// taken from `Transactions[obsolete]` paired with their `is_latest` flag. The order
	/// preserves the original `BoundedVec` order.
	fn handle_obsolete(obsolete: BlockNumber, items: &[(TransactionInfo<Meta>, bool)]);
}

impl<BlockNumber, Meta> OnObsoleteTransactions<BlockNumber, Meta> for () {
	fn handle_obsolete(_: BlockNumber, _: &[(TransactionInfo<Meta>, bool)]) {}
}

/// State data for a stored transaction.
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq, scale_info::TypeInfo, MaxEncodedLen)]
pub struct TransactionInfo<Meta> {
	/// Chunk trie root.
	pub chunk_root: <BlakeTwo256 as Hash>::Output,

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
	pub block_chunks: ChunkIndex,

	/// Opaque per-entry payload ([`crate::Config::EntryMeta`]). This pallet stores it
	/// verbatim (entries it creates carry `Default::default()`) and hands it back through
	/// [`OnObsoleteTransactions::handle_obsolete`] at expiry. Field is appended at the end
	/// of the struct; the wired type must keep the retired `TransactionKind`'s 1-byte
	/// encoding so pre-existing entries decode unchanged.
	pub meta: Meta,
}

/// [`TransactionInfo`] bound to a runtime's [`crate::Config::EntryMeta`].
pub type TransactionInfoFor<T> = TransactionInfo<<T as Config>::EntryMeta>;

impl<Meta> TransactionInfo<Meta> {
	/// Get the number of total chunks.
	///
	/// See the `block_chunks` field of [`TransactionInfo`] for details.
	pub fn total_chunks(txs: &[TransactionInfo<Meta>]) -> ChunkIndex {
		txs.last().map_or(0, |t| t.block_chunks)
	}
}

/// Context of a `check_signed`/`check_unsigned` call.
#[derive(Clone, Copy)]
pub enum CheckContext {
	/// `validate_signed` or `validate_unsigned`.
	Validate,
	/// `pre_dispatch_signed` or `pre_dispatch`.
	PreDispatch,
}

impl CheckContext {
	/// Should authorization be consumed in this context? If not, we merely check that
	/// authorization exists.
	pub fn consume_authorization(self) -> bool {
		matches!(self, CheckContext::PreDispatch)
	}

	/// Should `check_signed`/`check_unsigned` return a `ValidTransaction`?
	pub fn want_valid_transaction(self) -> bool {
		matches!(self, CheckContext::Validate)
	}
}

/// A registered authorizer's budget.
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
pub struct AuthorizerBudget<BlockNumber> {
	/// `None` is unlimited; `Some(_)` decrements both axes per dispatch.
	pub quota: Option<Quota>,
	/// Optional expiration block. While `Some(t)`, this authorizer can authorize only
	/// while `now < t`; once `now >= t`, [`EnsureAllowedAuthorizers`] rejects them and
	/// [`Pallet::remove_exhausted_authorizer`] becomes callable on this entry.
	/// Additionally, authorizations granted by this authorizer have their expiration
	/// clamped to `t` — a grant cannot outlive the authorizer that issued it.
	pub valid_until: Option<BlockNumber>,
	/// When `true`, this authorizer's `authorize_account` / `refresh_account_authorization`
	/// dispatches are fee-exempt.
	pub feeless: bool,
}

/// Paired transaction / byte quota for an authorizer.
#[derive(
	Copy,
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
pub struct Quota {
	pub transactions: u32,
	pub bytes: u64,
}

impl<BlockNumber> AuthorizerBudget<BlockNumber> {
	/// `quota = Some` with either axis at zero. `quota = None` is never exhausted.
	pub fn is_exhausted(&self) -> bool {
		self.quota.is_some_and(|q| q.transactions == 0 || q.bytes == 0)
	}

	/// Decrement both quota axes by `(transactions, bytes)`. `Some(())` on success and
	/// also when `quota = None` (unlimited — no-op). `None` on underflow of either
	/// axis; the budget is left unchanged in that case.
	pub fn try_consume(&mut self, transactions: u32, bytes: u64) -> Option<()> {
		let Some(q) = self.quota.as_mut() else { return Some(()) };
		let new_tx = q.transactions.checked_sub(transactions)?;
		let new_bytes = q.bytes.checked_sub(bytes)?;
		q.transactions = new_tx;
		q.bytes = new_bytes;
		Some(())
	}
}

impl<BlockNumber: PartialOrd + Copy> AuthorizerBudget<BlockNumber> {
	/// `true` iff `valid_until` is set and `now` has reached or passed it. Authorizers
	/// with `valid_until = None` never expire. Single source of truth for the
	/// `[registration_block, valid_until)` window used by `add_authorizer` validation,
	/// [`EnsureAllowedAuthorizers`], and [`Pallet::remove_exhausted_authorizer`].
	pub fn is_expired(&self, now: BlockNumber) -> bool {
		self.valid_until.is_some_and(|t| now >= t)
	}

	/// `true` when this budget can no longer back new authorizations —
	/// either exhausted on at least one axis, or past its `valid_until`.
	pub fn is_inactive(&self, now: BlockNumber) -> bool {
		self.is_exhausted() || self.is_expired(now)
	}
}

pub(crate) type AuthorizerBudgetFor<T> = AuthorizerBudget<BlockNumberFor<T>>;

/// Per-dispatch context returned by [`Config::Authorizer`] when the dispatcher is
/// an [`AllowedAuthorizers`] entry. Carries everything `authorize_*` needs from
/// the authorizer:
///
/// - `authorizer`: the account whose [`AllowedAuthorizers`] budget will be charged.
/// - `valid_until`: the authorizer's expiry block. Authorizations granted through this dispatch
///   have their expiration clamped to `valid_until` — a grant cannot outlive the authorizer that
///   issued it.
///
/// `None` (as the full [`EnsureOrigin::Success`]) means the dispatcher is a
/// non-account authorizer (Root / XCM / signed-by list) — no budget to charge
/// and no clamping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationOrigin<AccountId, BlockNumber> {
	pub authorizer: AccountId,
	pub valid_until: Option<BlockNumber>,
	/// Mirrors [`AuthorizerBudget::feeless`]. Threaded here so `#[pallet::feeless_if]`
	/// doesn't have to re-read the budget.
	pub feeless: bool,
}

pub(crate) type AuthorizationOriginFor<T> =
	AuthorizationOrigin<<T as frame_system::Config>::AccountId, BlockNumberFor<T>>;

/// `EnsureOrigin` adapter that accepts a `Signed(account)` origin iff the signing
/// account is registered in [`AllowedAuthorizers`]. Used to plug the runtime-mutable
/// authorizer list into the pallet's `Authorizer` chain.
pub struct EnsureAllowedAuthorizers<T>(core::marker::PhantomData<T>);

impl<T: Config> EnsureOrigin<T::RuntimeOrigin> for EnsureAllowedAuthorizers<T>
where
	T::RuntimeOrigin: From<frame_system::RawOrigin<T::AccountId>>
		+ Into<Result<frame_system::RawOrigin<T::AccountId>, T::RuntimeOrigin>>,
{
	type Success = Option<AuthorizationOriginFor<T>>;

	fn try_origin(o: T::RuntimeOrigin) -> Result<Self::Success, T::RuntimeOrigin> {
		o.into().and_then(|raw| match raw {
			frame_system::RawOrigin::Signed(who) => match AllowedAuthorizers::<T>::get(&who) {
				Some(b) if !b.is_expired(Pallet::<T>::now()) => Ok(Some(AuthorizationOrigin {
					authorizer: who,
					valid_until: b.valid_until,
					feeless: b.feeless,
				})),
				_ => Err(T::RuntimeOrigin::from(frame_system::RawOrigin::Signed(who))),
			},
			other => Err(T::RuntimeOrigin::from(other)),
		})
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn try_successful_origin() -> Result<T::RuntimeOrigin, ()> {
		let who = match AllowedAuthorizers::<T>::iter_keys().next() {
			Some(existing) => existing,
			None => {
				let new: T::AccountId = frame_benchmarking::account("allowed_authorizer", 0, 0);
				AllowedAuthorizers::<T>::insert(
					&new,
					AuthorizerBudget {
						quota: Some(Quota { transactions: 10_000, bytes: 100_000 }),
						valid_until: None,
						feeless: false,
					},
				);
				new
			},
		};
		Ok(frame_system::RawOrigin::Signed(who).into())
	}
}

/// `EnsureOrigin` adapter that wraps an inner origin and projects its `Success` to
/// `None: Option<AuthorizationOrigin<AccountId, BlockNumber>>`. Used to lift
/// non-budgeted authorizers (Root, XCM, signed-by lists) into the
/// `Option<AuthorizationOrigin<_, _>>` `Success` shape produced by
/// [`EnsureAllowedAuthorizers`], so both kinds compose in the
/// [`Config::Authorizer`] chain.
pub struct AsAuthorizer<E, AccountId, BlockNumber>(
	core::marker::PhantomData<(E, AccountId, BlockNumber)>,
);

impl<O, AccountId, BlockNumber, E: EnsureOrigin<O>> EnsureOrigin<O>
	for AsAuthorizer<E, AccountId, BlockNumber>
{
	type Success = Option<AuthorizationOrigin<AccountId, BlockNumber>>;

	fn try_origin(o: O) -> Result<Self::Success, O> {
		E::try_origin(o).map(|_| None)
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn try_successful_origin() -> Result<O, ()> {
		E::try_successful_origin()
	}
}
