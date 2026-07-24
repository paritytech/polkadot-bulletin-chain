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

//! Custom transaction extension for the transaction storage pallet.

use crate::{
	pallet::Origin, weights::WeightInfo, AuthorizationExtent, AuthorizationScope,
	AuthorizationScopeFor, Call, Config, Pallet, LOG_TARGET,
};
use alloc::vec::Vec;
use codec::{Decode, DecodeWithMemTracking, Encode};
use core::{fmt, marker::PhantomData};
use polkadot_sdk_frame::{
	deps::*,
	prelude::*,
	traits::{AsSystemOriginSigner, Implication, OriginTrait},
};

type RuntimeCallOf<T> = <T as frame_system::Config>::RuntimeCall;

/// Maximum recursion depth for inspecting wrapper calls.
pub const MAX_WRAPPER_DEPTH: u32 = 8;

/// Tells [`ValidateAuthorizedCalls`] how to find storage calls inside wrapper
/// extrinsics (e.g. `Utility::batch`, `Sudo::sudo_as`).
///
/// The runtime implements this for its `RuntimeCall` type, allowing the pallet extension
/// to recursively inspect wrapper calls for storage-mutating operations (which are rejected)
/// and management calls (which are validated).
pub trait CallInspector<T: Config>: Clone + PartialEq + Eq + Default
where
	RuntimeCallOf<T>: IsSubType<Call<T>>,
{
	/// If `call` is a wrapper, return the inner calls to inspect for storage authorization.
	///
	/// Returns `None` for non-wrapper calls.
	fn inspect_wrapper(call: &RuntimeCallOf<T>) -> Option<Vec<&RuntimeCallOf<T>>>;

	/// Returns `true` if `call` is a storage-mutating TransactionStorage call (`store`,
	/// `store_with_cid_config`) — either directly or nested inside wrappers.
	///
	/// `force_renew` lives in `pallet-bulletin-transaction-storage-renewal`; runtime
	/// `CallInspector`s must additionally check for its call type if they care about renewal
	/// mutations.
	///
	/// Intended for use in XCM `SafeCallFilter` implementations. The runtime's
	/// [`CallInspector`] provides the wrapper-recursion logic, so this function
	/// works for any runtime without duplicating the blocked-call list.
	fn is_storage_mutating_call(call: &RuntimeCallOf<T>, depth: u32) -> bool {
		// Check direct pallet calls first — these are always identifiable regardless
		// of depth, matching the ordering in `traverse_storage_calls`.
		if let Some(inner_call) = call.is_sub_type() {
			return matches!(inner_call, Call::store { .. } | Call::store_with_cid_config { .. });
		}
		if depth >= MAX_WRAPPER_DEPTH {
			// Fail-safe: treat excessively nested wrappers as storage-mutating rather
			// than risk letting a hidden storage call bypass the filter.
			tracing::debug!(
				target: LOG_TARGET,
				"Wrapper recursion limit exceeded (depth: {depth}), treating as storage-mutating",
			);
			return true;
		}
		if let Some(inner_calls) = Self::inspect_wrapper(call) {
			return inner_calls
				.into_iter()
				.any(|inner| Self::is_storage_mutating_call(inner, depth + 1));
		}
		false
	}
}

/// No-op implementation — no wrapper inspection. Direct storage calls still work.
impl<T: Config> CallInspector<T> for ()
where
	RuntimeCallOf<T>: IsSubType<Call<T>>,
{
	fn inspect_wrapper(_: &RuntimeCallOf<T>) -> Option<Vec<&RuntimeCallOf<T>>> {
		None
	}
}

/// Result payload of [`LeafValidator::validate_leaf`]: `None` when the call is not
/// this validator's leaf; otherwise the leaf's [`ValidTransaction`] plus the
/// [`AuthorizationScope`] charged (if any).
pub type LeafValidation<T> = Option<(ValidTransaction, Option<AuthorizationScopeFor<T>>)>;

/// One pallet's contribution to [`ValidateAuthorizedCalls`]'s shared call-tree walk.
///
/// Implemented by pallets whose calls need authorization-backed pool validation —
/// this pallet's [`StorageLeaves`] and the renewal pallet's `RenewalLeaves`. The
/// runtime wires a tuple of validators; every leaf found in the walk is offered to
/// each element in order until one claims it.
pub trait LeafValidator<T: Config> {
	/// Pool-time validation of one call-tree node. Returns `Ok(None)` when `call`
	/// is not this validator's leaf, `Ok(Some((valid, maybe_scope)))` when claimed
	/// and valid. `depth` is the wrapper depth (`0` = direct extrinsic), for
	/// direct-only rules. Must not mutate state.
	fn validate_leaf(
		who: &T::AccountId,
		call: &RuntimeCallOf<T>,
		depth: u32,
	) -> Result<LeafValidation<T>, TransactionValidityError>;

	/// Inclusion-time counterpart: consume the authorization. Returns `Ok(true)`
	/// when `call` was this validator's leaf.
	fn prepare_leaf(
		who: &T::AccountId,
		call: &RuntimeCallOf<T>,
		depth: u32,
	) -> Result<bool, TransactionValidityError>;

	/// This validator's contribution to the extension's declared weight for the
	/// top-level `call`.
	fn leaf_weight(call: &RuntimeCallOf<T>) -> Weight;
}

#[impl_trait_for_tuples::impl_for_tuples(4)]
impl<T: Config> LeafValidator<T> for Tuple {
	fn validate_leaf(
		who: &T::AccountId,
		call: &RuntimeCallOf<T>,
		depth: u32,
	) -> Result<LeafValidation<T>, TransactionValidityError> {
		for_tuples!( #(
			if let Some(result) = Tuple::validate_leaf(who, call, depth)? {
				return Ok(Some(result));
			}
		)* );
		Ok(None)
	}

	fn prepare_leaf(
		who: &T::AccountId,
		call: &RuntimeCallOf<T>,
		depth: u32,
	) -> Result<bool, TransactionValidityError> {
		for_tuples!( #(
			if Tuple::prepare_leaf(who, call, depth)? {
				return Ok(true);
			}
		)* );
		Ok(false)
	}

	fn leaf_weight(call: &RuntimeCallOf<T>) -> Weight {
		let mut weight = Weight::zero();
		for_tuples!( #( weight = weight.saturating_add(Tuple::leaf_weight(call)); )* );
		weight
	}
}

/// [`LeafValidator`] for this pallet's calls: `store` / `store_with_cid_config`
/// (direct-only, authorization-consuming) and the management calls (`authorize_*`,
/// `refresh_*`, `remove_expired_*`), which may appear inside wrappers.
pub struct StorageLeaves<T>(PhantomData<T>);

impl<T: Config> LeafValidator<T> for StorageLeaves<T>
where
	RuntimeCallOf<T>: IsSubType<Call<T>>,
{
	fn validate_leaf(
		who: &T::AccountId,
		call: &RuntimeCallOf<T>,
		depth: u32,
	) -> Result<LeafValidation<T>, TransactionValidityError> {
		let Some(inner_call) = call.is_sub_type() else { return Ok(None) };
		if depth > 0 &&
			matches!(inner_call, Call::store { .. } | Call::store_with_cid_config { .. })
		{
			// Store calls must be direct extrinsics: the node-side transaction
			// indexing assumes the data is the trailing bytes of the extrinsic.
			return Err(InvalidTransaction::Call.into());
		}
		Pallet::<T>::validate_signed(who, inner_call).map(Some)
	}

	fn prepare_leaf(
		who: &T::AccountId,
		call: &RuntimeCallOf<T>,
		depth: u32,
	) -> Result<bool, TransactionValidityError> {
		let Some(inner_call) = call.is_sub_type() else { return Ok(false) };
		if depth > 0 &&
			matches!(inner_call, Call::store { .. } | Call::store_with_cid_config { .. })
		{
			return Err(InvalidTransaction::Call.into());
		}
		Pallet::<T>::pre_dispatch_signed(who, inner_call).map(|_| true)
	}

	fn leaf_weight(call: &RuntimeCallOf<T>) -> Weight {
		match call.is_sub_type() {
			Some(Call::store { data, .. }) | Some(Call::store_with_cid_config { data, .. }) =>
				T::WeightInfo::validate_store(data.len() as u32),
			_ => Weight::zero(),
		}
	}
}

/// Transaction extension that validates authorization-gated Bulletin calls in a
/// single walk over the call tree.
///
/// `I` supplies the wrapper inspector ([`CallInspector`], e.g. `Utility::batch`
/// unwrap); `L` is a [`LeafValidator`] tuple — one element per participating pallet.
/// Each leaf is offered to the validators in order; the first claimant handles it.
/// `validate()` checks without consuming and combines the leaves'
/// [`ValidTransaction`]s; `prepare()` repeats the walk consuming the authorization.
/// The origin is rewritten once, after the walk, to [`Origin::Authorized`] with the
/// last claimed scope.
///
/// A renewal-free runtime wires `L = (StorageLeaves<Runtime>,)`; runtimes with
/// `pallet-bulletin-transaction-storage-renewal` add its `RenewalLeaves`.
#[derive(Encode, Decode, DecodeWithMemTracking, scale_info::TypeInfo)]
#[codec(encode_bound())]
#[codec(decode_bound())]
#[codec(mel_bound())]
#[scale_info(skip_type_params(T, I, L))]
pub struct ValidateAuthorizedCalls<T, I = (), L = (StorageLeaves<T>,)>(PhantomData<(T, I, L)>);

impl<T, I, L> Default for ValidateAuthorizedCalls<T, I, L> {
	fn default() -> Self {
		Self(PhantomData)
	}
}

// Manual impls: derives would demand `I: Clone + Eq` / `L: Clone + Eq`, which the
// marker parameters don't need — the struct is a unit.
impl<T, I, L> Clone for ValidateAuthorizedCalls<T, I, L> {
	fn clone(&self) -> Self {
		Self(PhantomData)
	}
}

impl<T, I, L> PartialEq for ValidateAuthorizedCalls<T, I, L> {
	fn eq(&self, _: &Self) -> bool {
		true
	}
}

impl<T, I, L> Eq for ValidateAuthorizedCalls<T, I, L> {}

impl<T: Config + Send + Sync, I, L> fmt::Debug for ValidateAuthorizedCalls<T, I, L> {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "ValidateAuthorizedCalls")
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
		Ok(())
	}
}

impl<T, I, L> ValidateAuthorizedCalls<T, I, L>
where
	T: Config,
	I: CallInspector<T>,
	RuntimeCallOf<T>: IsSubType<Call<T>>,
{
	/// Offer each node to `visitor` (returning whether it was claimed); descend
	/// into unclaimed wrappers via [`CallInspector::inspect_wrapper`]. Unclaimed
	/// non-wrapper nodes are ignored. Fail-safe: an unclaimed node at
	/// [`MAX_WRAPPER_DEPTH`] is rejected rather than risking a hidden direct-only
	/// call slipping through.
	fn walk<F>(
		call: &RuntimeCallOf<T>,
		depth: u32,
		visitor: &mut F,
	) -> Result<(), TransactionValidityError>
	where
		F: FnMut(&RuntimeCallOf<T>, u32) -> Result<bool, TransactionValidityError>,
	{
		if visitor(call, depth)? {
			return Ok(());
		}
		if depth >= MAX_WRAPPER_DEPTH {
			return Err(InvalidTransaction::Call.into());
		}
		if let Some(inner_calls) = I::inspect_wrapper(call) {
			for inner in inner_calls {
				Self::walk(inner, depth + 1, visitor)?;
			}
		}
		Ok(())
	}
}

impl<T, I, L> TransactionExtension<RuntimeCallOf<T>> for ValidateAuthorizedCalls<T, I, L>
where
	T: Config + Send + Sync,
	I: CallInspector<T> + Send + Sync + 'static,
	L: LeafValidator<T> + Send + Sync + 'static,
	RuntimeCallOf<T>: IsSubType<Call<T>>,
	T::RuntimeOrigin: OriginTrait + AsSystemOriginSigner<T::AccountId> + From<Origin<T>>,
	<T::RuntimeOrigin as OriginTrait>::PalletsOrigin: From<Origin<T>> + TryInto<Origin<T>>,
{
	const IDENTIFIER: &'static str = "ValidateAuthorizedCalls";

	type Implicit = ();
	fn implicit(&self) -> Result<Self::Implicit, TransactionValidityError> {
		Ok(())
	}

	/// Signer (when the transaction is signed and any leaf was claimed); drives
	/// `prepare`'s second walk.
	type Val = Option<T::AccountId>;
	type Pre = ();

	fn weight(&self, call: &RuntimeCallOf<T>) -> Weight {
		L::leaf_weight(call)
	}

	fn validate(
		&self,
		mut origin: T::RuntimeOrigin,
		call: &RuntimeCallOf<T>,
		_info: &DispatchInfoOf<RuntimeCallOf<T>>,
		_len: usize,
		_self_implicit: Self::Implicit,
		_inherited_implication: &impl Implication,
		_source: TransactionSource,
	) -> ValidateResult<Self::Val, RuntimeCallOf<T>> {
		// Unsigned + non-system origins pass through.
		let who = match origin.as_system_origin_signer() {
			Some(w) => w.clone(),
			None => return Ok((ValidTransaction::default(), None, origin)),
		};

		let mut combined = ValidTransaction::default();
		let mut last_scope: Option<AuthorizationScopeFor<T>> = None;
		let mut visited_any = false;

		Self::walk(call, 0, &mut |leaf, depth| {
			let Some((valid_tx, maybe_scope)) = L::validate_leaf(&who, leaf, depth)? else {
				return Ok(false);
			};
			combined = core::mem::take(&mut combined).combine_with(valid_tx);
			if let Some(scope) = maybe_scope {
				last_scope = Some(scope);
			}
			visited_any = true;
			Ok(true)
		})?;

		if let Some(scope) = last_scope {
			origin.set_caller_from(Origin::<T>::Authorized { who: who.clone(), scope });
		}
		Ok((combined, visited_any.then_some(who), origin))
	}

	fn prepare(
		self,
		val: Self::Val,
		_origin: &T::RuntimeOrigin,
		call: &RuntimeCallOf<T>,
		_info: &DispatchInfoOf<RuntimeCallOf<T>>,
		_len: usize,
	) -> Result<Self::Pre, TransactionValidityError> {
		let Some(who) = val else { return Ok(()) };
		Self::walk(call, 0, &mut |leaf, depth| L::prepare_leaf(&who, leaf, depth))
	}
}

/// Priority bonus given to an in-budget signer. Over-budget signers get `0`.
pub const ALLOWANCE_PRIORITY_BOOST: TransactionPriority = 1_000_000_000;

/// Maps the prospective post-this-tx [`AuthorizationExtent`] to the priority bonus
/// added by [`AllowanceBasedPriority`]. Pick a concrete impl in the runtime's
/// `TxExtension` tuple.
///
/// Callers must pre-apply the call's effect to `extent` before calling `boost`:
/// `extent.bytes += size` and `extent.transactions += 1`. The boost decision
/// then reduces to "would this leave the holder in-budget on both axes?".
pub trait BoostStrategy: Clone + PartialEq + Eq {
	fn boost<Extra>(extent: AuthorizationExtent<Extra>) -> TransactionPriority;
}

/// Returns whether `extent` (already post-this-tx) is in-budget on both the byte
/// counter and the transaction counter. The `bytes_allowance == 0` guard catches the
/// "missing or empty grant" case.
fn in_budget<Extra>(extent: &AuthorizationExtent<Extra>) -> bool {
	if extent.bytes_allowance == 0 {
		return false;
	}
	extent.bytes <= extent.bytes_allowance && extent.transactions <= extent.transactions_allowance
}

/// Boost scales linearly with the tighter of the byte-budget and tx-budget remainders.
/// Fresh grant yields the full boost; at-cap on either axis yields zero.
#[derive(Clone, PartialEq, Eq)]
pub struct ProportionalBoost;
impl BoostStrategy for ProportionalBoost {
	fn boost<Extra>(extent: AuthorizationExtent<Extra>) -> TransactionPriority {
		if !in_budget(&extent) {
			return 0;
		}
		// Byte remainder: `bytes_allowance` is non-zero by `in_budget`.
		let bytes_rem = extent.bytes_allowance.saturating_sub(extent.bytes);
		let bytes_share =
			(ALLOWANCE_PRIORITY_BOOST as u128 * bytes_rem as u128) / extent.bytes_allowance as u128;
		// Tx remainder: when `transactions_allowance == 0`, treat as no boost.
		let tx_share = if extent.transactions_allowance == 0 {
			0
		} else {
			let tx_rem = extent.transactions_allowance.saturating_sub(extent.transactions);
			(ALLOWANCE_PRIORITY_BOOST as u128 * tx_rem as u128) /
				extent.transactions_allowance as u128
		};
		bytes_share.min(tx_share) as u64
	}
}

/// Flat boost while in-budget on both byte and tx axes, `0` otherwise.
#[derive(Clone, PartialEq, Eq)]
pub struct FlatBoost;
impl BoostStrategy for FlatBoost {
	fn boost<Extra>(extent: AuthorizationExtent<Extra>) -> TransactionPriority {
		if in_budget(&extent) {
			ALLOWANCE_PRIORITY_BOOST
		} else {
			0
		}
	}
}

/// Boosts signed `store` / `store_with_cid_config` priority via a runtime-selected
/// [`BoostStrategy`]. Over-allowance txs still validate; they just don't get the boost.
#[derive(Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, scale_info::TypeInfo)]
#[codec(encode_bound())]
#[codec(decode_bound())]
#[codec(mel_bound())]
#[scale_info(skip_type_params(T, B))]
pub struct AllowanceBasedPriority<T, B = FlatBoost>(PhantomData<(T, B)>);

impl<T, B> Default for AllowanceBasedPriority<T, B> {
	fn default() -> Self {
		Self(PhantomData)
	}
}

impl<T: Config + Send + Sync, B: BoostStrategy> fmt::Debug for AllowanceBasedPriority<T, B> {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "AllowanceBasedPriority")
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
		Ok(())
	}
}

impl<T: Config + Send + Sync, B: BoostStrategy + Send + Sync + 'static>
	TransactionExtension<RuntimeCallOf<T>> for AllowanceBasedPriority<T, B>
where
	RuntimeCallOf<T>: IsSubType<Call<T>>,
	T::RuntimeOrigin: OriginTrait,
	<T::RuntimeOrigin as OriginTrait>::PalletsOrigin: Clone + TryInto<Origin<T>>,
{
	const IDENTIFIER: &'static str = "AllowanceBasedPriority";

	type Implicit = ();
	fn implicit(&self) -> Result<Self::Implicit, TransactionValidityError> {
		Ok(())
	}

	type Val = ();
	type Pre = ();

	fn weight(&self, _call: &RuntimeCallOf<T>) -> Weight {
		<T as frame_system::Config>::DbWeight::get().reads(1)
	}

	fn validate(
		&self,
		origin: T::RuntimeOrigin,
		call: &RuntimeCallOf<T>,
		_info: &DispatchInfoOf<RuntimeCallOf<T>>,
		_len: usize,
		_self_implicit: Self::Implicit,
		_inherited_implication: &impl Implication,
		_source: TransactionSource,
	) -> ValidateResult<Self::Val, RuntimeCallOf<T>> {
		let Some(inner_call) = call.is_sub_type() else {
			return Ok((ValidTransaction::default(), (), origin));
		};
		// Only `store` / `store_with_cid_config` get the boost. `renew` also carries
		// `Origin::Authorized` and does consume allowance, but it operates on
		// already-stored data and shouldn't compete for the same priority slots as
		// new submissions.
		let this_tx_bytes = match inner_call {
			Call::store { data } | Call::store_with_cid_config { data, .. } => data.len() as u64,
			_ => return Ok((ValidTransaction::default(), (), origin)),
		};

		// `ValidateStorageCalls` earlier in the pipeline rewrites the origin to
		// `Origin::Authorized`; only the account-scoped variant consumes the caller's allowance.
		// Boost against the post-this-tx state so a single oversized tx is demoted on entry.
		let priority = match origin.caller().clone().try_into() {
			Ok(Origin::<T>::Authorized { who, scope: AuthorizationScope::Account(_) }) => {
				let mut extent = Pallet::<T>::account_authorization_extent(who);
				extent.bytes = extent.bytes.saturating_add(this_tx_bytes);
				extent.transactions = extent.transactions.saturating_add(1);
				B::boost(extent)
			},
			_ => 0,
		};

		Ok((ValidTransaction { priority, ..Default::default() }, (), origin))
	}

	fn prepare(
		self,
		_val: Self::Val,
		_origin: &T::RuntimeOrigin,
		_call: &RuntimeCallOf<T>,
		_info: &DispatchInfoOf<RuntimeCallOf<T>>,
		_len: usize,
	) -> Result<Self::Pre, TransactionValidityError> {
		Ok(())
	}
}

#[cfg(test)]
mod boost_tests {
	use super::*;

	/// Build a post-this-tx extent on the byte axis. The tx counter is parked at
	/// `(0, u32::MAX)` so the tx axis is never the binding constraint in byte-focused
	/// tests; tx-axis behaviour is covered by the dedicated test below.
	fn extent(bytes: u64, allowance: u64) -> AuthorizationExtent<()> {
		AuthorizationExtent {
			bytes,
			extra: (),
			bytes_allowance: allowance,
			transactions: 0,
			transactions_allowance: u32::MAX,
		}
	}

	const A: u64 = 1_000;
	const BOOST: u64 = ALLOWANCE_PRIORITY_BOOST;

	#[test]
	fn proportional_scales_with_remaining_allowance() {
		assert_eq!(ProportionalBoost::boost(extent(0, 0)), 0); // no auth
		assert_eq!(ProportionalBoost::boost(extent(A, A)), 0); // at cap (post-tx)
		assert_eq!(ProportionalBoost::boost(extent(A + 1, A)), 0); // over cap
		assert_eq!(ProportionalBoost::boost(extent(0, A)), BOOST); // unused
		assert_eq!(ProportionalBoost::boost(extent(A / 2, A)), BOOST / 2); // half
		assert_eq!(ProportionalBoost::boost(extent(A * 3 / 4, A)), BOOST / 4); // three-quarters
	}

	#[test]
	fn flat_is_constant_while_in_budget() {
		assert_eq!(FlatBoost::boost(extent(0, 0)), 0); // no auth
		assert_eq!(FlatBoost::boost(extent(A, A)), BOOST); // at cap (still in-budget)
		assert_eq!(FlatBoost::boost(extent(A + 1, A)), 0); // over cap
		assert_eq!(FlatBoost::boost(extent(0, A)), BOOST); // unused
		assert_eq!(FlatBoost::boost(extent(A / 2, A)), BOOST); // half
		assert_eq!(FlatBoost::boost(extent(A - 1, A)), BOOST); // just below cap
	}

	#[test]
	fn flat_does_not_let_fresh_outrank_partly_used() {
		let fresh = extent(0, A);
		let partly_used = extent(A / 2, A);
		assert!(ProportionalBoost::boost(fresh) > ProportionalBoost::boost(partly_used));
		assert_eq!(FlatBoost::boost(fresh), FlatBoost::boost(partly_used));
	}

	#[test]
	fn tx_axis_gates_boost_independently() {
		// In-budget on bytes, over on transactions → no boost.
		let over_tx = AuthorizationExtent {
			bytes: 0,
			extra: (),
			bytes_allowance: A,
			transactions: 11,
			transactions_allowance: 10,
		};
		assert_eq!(FlatBoost::boost(over_tx), 0);
		assert_eq!(ProportionalBoost::boost(over_tx), 0);

		// In-budget on both axes; the tighter remainder caps the proportional share.
		let tight_tx = AuthorizationExtent {
			bytes: 0,
			extra: (),
			bytes_allowance: A,
			transactions: 9,
			transactions_allowance: 10,
		};
		assert_eq!(FlatBoost::boost(tight_tx), BOOST);
		assert_eq!(ProportionalBoost::boost(tight_tx), BOOST / 10);
	}
}
