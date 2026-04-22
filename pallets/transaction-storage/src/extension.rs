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

//! Custom transaction extension for the transaction storage pallet.

use crate::{
	pallet::Origin, weights::WeightInfo, AuthorizationScope, Call, Config, Pallet, LOG_TARGET,
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

/// Result of [`CallInspector::traverse_storage_calls`]: whether any TransactionStorage
/// pallet calls (management calls like authorize_*, refresh_*, remove_expired_*) were found.
#[derive(Default)]
pub struct TraverseResult {
	pub found_storage: bool,
}

/// Maximum recursion depth for inspecting wrapper calls.
pub const MAX_WRAPPER_DEPTH: u32 = 8;

/// Tells [`ValidateStorageCalls`] how to find storage calls inside wrapper
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

	/// Returns `true` if `call` is a storage-mutating TransactionStorage call (store,
	/// store_with_cid_config, renew) — either directly or nested inside wrappers.
	///
	/// Intended for use in XCM `SafeCallFilter` implementations. The runtime's
	/// [`CallInspector`] provides the wrapper-recursion logic, so this function
	/// works for any runtime without duplicating the blocked-call list.
	fn is_storage_mutating_call(call: &RuntimeCallOf<T>, depth: u32) -> bool {
		// Check direct pallet calls first — these are always identifiable regardless
		// of depth, matching the ordering in `traverse_storage_calls`.
		if let Some(inner_call) = call.is_sub_type() {
			return matches!(
				inner_call,
				Call::store { .. } | Call::store_with_cid_config { .. } | Call::renew { .. }
			);
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

	/// Recursively traverse a call tree, applying `visitor` to each
	/// TransactionStorage pallet call found.
	///
	/// Returns [`TraverseResult`] with `found_storage` set if any pallet calls were visited.
	/// Callers should use [`Self::is_storage_mutating_call`] first to reject wrappers
	/// containing store/renew before calling this.
	fn traverse_storage_calls(
		call: &RuntimeCallOf<T>,
		depth: u32,
		visitor: &mut impl FnMut(&Call<T>) -> Result<(), TransactionValidityError>,
	) -> Result<TraverseResult, TransactionValidityError> {
		if let Some(inner_call) = call.is_sub_type() {
			visitor(inner_call)?;
			return Ok(TraverseResult { found_storage: true });
		}
		if let Some(inner_calls) = Self::inspect_wrapper(call) {
			if depth >= MAX_WRAPPER_DEPTH {
				tracing::debug!(
					target: LOG_TARGET,
					"Wrapper recursion limit exceeded (depth: {depth}), rejecting call",
				);
				return Err(InvalidTransaction::ExhaustsResources.into());
			}
			let mut found_storage = false;
			for inner in inner_calls {
				found_storage |=
					Self::traverse_storage_calls(inner, depth + 1, visitor)?.found_storage;
			}
			return Ok(TraverseResult { found_storage });
		}
		// Not a storage call and not a wrapper — ignore.
		Ok(TraverseResult::default())
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

/// Transaction extension that validates signed TransactionStorage calls.
///
/// This extension handles **signed TransactionStorage transactions** via
/// [`Pallet::validate_signed`]:
/// - **Store/renew calls**: Must be submitted as **direct extrinsics** (not wrapped). Validates
///   authorization in `validate()` and transforms the origin to [`Origin::Authorized`] to carry
///   authorization info. Then in `prepare()`, it consumes the authorization extent (decrements
///   remaining transactions/bytes) before the extrinsic executes. This early consumption prevents
///   large invalid store transactions from propagating through mempools and the network —
///   authorization is checked and spent at the extension level rather than during dispatch.
/// - **Authorization management calls** (authorize_*, refresh_*, remove_expired_*): Validates that
///   the signer satisfies the [`Config::Authorizer`] origin requirement. These calls **can** be
///   wrapped (e.g. in `Utility::batch`).
/// - **Wrapper calls** (e.g. `Utility::batch`, `Sudo::sudo`): Uses `I: CallInspector` to
///   recursively inspect inner calls. Rejects any wrapper containing store/renew calls. Allows
///   wrappers containing only management calls.
///
/// The `I` type parameter controls wrapper inspection. Use `()` (the default) for no wrapper
/// support, or provide a runtime-specific [`CallInspector`] implementation to enable recursive
/// validation inside batch, sudo, proxy, etc.
///
/// All other calls and unsigned transactions are passed through unchanged.
#[derive(Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, scale_info::TypeInfo)]
#[codec(encode_bound())]
#[codec(decode_bound())]
#[codec(mel_bound())]
#[scale_info(skip_type_params(T, I))]
pub struct ValidateStorageCalls<T, I = ()>(PhantomData<(T, I)>);

impl<T, I> Default for ValidateStorageCalls<T, I> {
	fn default() -> Self {
		Self(PhantomData)
	}
}

impl<T: Config + Send + Sync, I> fmt::Debug for ValidateStorageCalls<T, I> {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "ValidateStorageCalls")
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
		Ok(())
	}
}

impl<T: Config + Send + Sync, I: CallInspector<T> + Send + Sync + 'static>
	TransactionExtension<RuntimeCallOf<T>> for ValidateStorageCalls<T, I>
where
	RuntimeCallOf<T>: IsSubType<Call<T>>,
	T::RuntimeOrigin: OriginTrait + AsSystemOriginSigner<T::AccountId> + From<Origin<T>>,
	<T::RuntimeOrigin as OriginTrait>::PalletsOrigin: From<Origin<T>> + TryInto<Origin<T>>,
{
	const IDENTIFIER: &'static str = "ValidateStorageCalls";

	type Implicit = ();
	fn implicit(&self) -> Result<Self::Implicit, TransactionValidityError> {
		Ok(())
	}

	/// `Some(who)` when this extension handled storage-related calls (direct or wrapped).
	/// The signer is saved because the origin may be transformed to `Authorized`.
	type Val = Option<T::AccountId>;
	type Pre = ();

	fn weight(&self, call: &RuntimeCallOf<T>) -> Weight {
		let Some(inner_call) = call.is_sub_type() else {
			return Weight::zero();
		};
		match inner_call {
			Call::store { data, .. } | Call::store_with_cid_config { data, .. } =>
				T::WeightInfo::validate_store(data.len() as u32),
			Call::renew { .. } => T::WeightInfo::validate_renew(),
			_ => Weight::zero(),
		}
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
		// Only handle signed transactions
		let who = match origin.as_system_origin_signer() {
			Some(who) => who.clone(),
			None => return Ok((ValidTransaction::default(), None, origin)),
		};

		// Direct storage call
		if let Some(inner_call) = call.is_sub_type() {
			let (valid_tx, maybe_scope) = Pallet::<T>::validate_signed(&who, inner_call)?;
			if let Some(scope) = maybe_scope {
				origin.set_caller_from(Origin::<T>::Authorized { who: who.clone(), scope });
			}
			return Ok((valid_tx, Some(who), origin));
		}

		// Wrapper call — reject if it contains store/renew (must be direct extrinsics),
		// then validate any management calls (authorize_*, refresh_*, remove_expired_*).
		if I::is_storage_mutating_call(call, 0) {
			return Err(InvalidTransaction::Call.into());
		}
		let mut combined_valid = ValidTransaction::default();
		let result = I::traverse_storage_calls(call, 0, &mut |inner_call| {
			let (valid_tx, _scope) = Pallet::<T>::validate_signed(&who, inner_call)?;
			combined_valid = core::mem::take(&mut combined_valid).combine_with(valid_tx);
			Ok(())
		})?;
		if result.found_storage {
			return Ok((combined_valid, Some(who), origin));
		}

		// No TransactionStorage calls found in wrapper.
		Ok((ValidTransaction::default(), None, origin))
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

		// traverse_storage_calls handles both direct pallet calls (via is_sub_type)
		// and wrapper calls (via inspect_wrapper), consuming authorization for each.
		I::traverse_storage_calls(call, 0, &mut |inner_call| {
			Pallet::<T>::pre_dispatch_signed(&who, inner_call)
		})?;

		Ok(())
	}
}

/// Maximum priority boost added to a signed `store` / `store_with_cid_config` transaction.
///
/// The actual boost scales linearly with the signer's unused allowance:
/// `BOOST * (bytes_allowance - bytes) / bytes_allowance`. Fresh grant yields the full boost;
/// at-cap or over-cap yields zero.
pub const ALLOWANCE_PRIORITY_BOOST: TransactionPriority = 1_000_000_000;

/// Computes `ALLOWANCE_PRIORITY_BOOST * (bytes_allowance - bytes) / bytes_allowance`.
///
/// Returns `0` when `bytes_allowance == 0` (missing / expired auth) or when `bytes >=
/// bytes_allowance` (over cap). Uses `u128` intermediate because
/// `ALLOWANCE_PRIORITY_BOOST (1e9) * u64_max_plausible_bytes (~1e12)` exceeds `u64::MAX`.
fn proportional_boost(bytes: u64, bytes_allowance: u64) -> TransactionPriority {
	if bytes_allowance == 0 {
		return 0;
	}
	let remaining = bytes_allowance.saturating_sub(bytes);
	((ALLOWANCE_PRIORITY_BOOST as u128 * remaining as u128) / bytes_allowance as u128) as u64
}

/// Boosts the priority of signed `store` / `store_with_cid_config` calls proportionally to
/// the signer's remaining byte allowance. Transactions over allowance still validate
/// (consumption happens in [`ValidateStorageCalls`]), they just don't get any boost.
#[derive(Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, scale_info::TypeInfo)]
#[codec(encode_bound())]
#[codec(decode_bound())]
#[codec(mel_bound())]
#[scale_info(skip_type_params(T))]
pub struct AllowanceBasedPriority<T>(PhantomData<T>);

impl<T> Default for AllowanceBasedPriority<T> {
	fn default() -> Self {
		Self(PhantomData)
	}
}

impl<T: Config + Send + Sync> fmt::Debug for AllowanceBasedPriority<T> {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "AllowanceBasedPriority")
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
		Ok(())
	}
}

impl<T: Config + Send + Sync> TransactionExtension<RuntimeCallOf<T>> for AllowanceBasedPriority<T>
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
		// Exclude `renew`: it also carries `Origin::Authorized` but doesn't consume allowance.
		if !matches!(inner_call, Call::store { .. } | Call::store_with_cid_config { .. }) {
			return Ok((ValidTransaction::default(), (), origin));
		}

		// `ValidateStorageCalls` earlier in the pipeline rewrites the origin to
		// `Origin::Authorized`; only the account-scoped variant consumes the caller's allowance.
		let priority = match origin.caller().clone().try_into() {
			Ok(Origin::<T>::Authorized { who, scope: AuthorizationScope::Account(_) }) => {
				let extent = Pallet::<T>::account_authorization_extent(who);
				proportional_boost(extent.bytes, extent.bytes_allowance)
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
