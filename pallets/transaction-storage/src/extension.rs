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

use crate::{pallet::Origin, weights::WeightInfo, AuthorizationScope, Call, Config, Pallet};
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

/// Tells [`ValidateStorageCalls`] how to find storage calls inside wrapper
/// extrinsics (e.g. `Utility::batch`, `Sudo::sudo_as`).
///
/// The runtime implements this for its `RuntimeCall` type, allowing the pallet extension
/// to recursively validate and consume storage authorization in wrapped calls, and to
/// transform the origin to [`Origin::Authorized`] for origin-preserving wrappers.
pub trait CallInspector<Call>: Clone + PartialEq + Eq + Default {
	/// If `call` is a wrapper, return:
	/// - The inner calls to inspect for storage authorization
	/// - `true` if the wrapper passes origin through to inner calls (e.g. batch), `false` if it
	///   changes the origin (e.g. sudo_as)
	///
	/// Returns `None` for non-wrapper calls.
	fn inspect_wrapper(call: &Call) -> Option<(Vec<&Call>, bool)>;
}

/// No-op implementation — no wrapper inspection. Direct storage calls still work.
impl<Call> CallInspector<Call> for () {
	fn inspect_wrapper(_: &Call) -> Option<(Vec<&Call>, bool)> {
		None
	}
}

/// Transaction extension that validates signed TransactionStorage calls.
///
/// This extension handles **signed TransactionStorage transactions** via
/// [`Pallet::validate_signed`]:
/// - **Store/renew calls**: Validates authorization in `validate()` and transforms the origin to
///   [`Origin::Authorized`] to carry authorization info. Then in `prepare()`, it consumes the
///   authorization extent (decrements remaining transactions/bytes) before the extrinsic executes.
///   This early consumption prevents large invalid store transactions from propagating through
///   mempools and the network — authorization is checked and spent at the extension level rather
///   than during dispatch.
/// - **Authorization management calls** (authorize_*, refresh_*, remove_expired_*): Validates that
///   the signer satisfies the [`Config::Authorizer`] origin requirement.
/// - **Wrapper calls** (e.g. `Utility::batch`, `Sudo::sudo`): Uses `I: CallInspector` to
///   recursively find and validate/consume storage authorization for inner storage calls. For
///   origin-preserving wrappers (batch), the origin is transformed to [`Origin::Authorized`] so
///   that inner `store`/`renew` dispatches pass [`Pallet::ensure_authorized`].
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

impl<T: Config + Send + Sync, I: CallInspector<RuntimeCallOf<T>>> ValidateStorageCalls<T, I>
where
	RuntimeCallOf<T>: IsSubType<Call<T>>,
{
	/// Recursively traverse a call tree, applying `visitor` to each storage call found.
	/// Returns `true` if any storage calls were visited.
	fn traverse_storage_calls(
		call: &RuntimeCallOf<T>,
		depth: u32,
		visitor: &mut impl FnMut(&Call<T>) -> Result<(), TransactionValidityError>,
	) -> Result<bool, TransactionValidityError> {
		if let Some(inner_call) = call.is_sub_type() {
			visitor(inner_call)?;
			return Ok(true);
		}
		if let Some((inner_calls, _)) = I::inspect_wrapper(call) {
			if depth >= MAX_WRAPPER_DEPTH {
				return Err(InvalidTransaction::ExhaustsResources.into());
			}
			let mut found = false;
			for inner in inner_calls {
				if Self::traverse_storage_calls(inner, depth + 1, visitor)? {
					found = true;
				}
			}
			return Ok(found);
		}
		Ok(false)
	}
}

impl<T: Config + Send + Sync, I: CallInspector<RuntimeCallOf<T>> + Send + Sync + 'static>
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
			if let Some(ref scope) = maybe_scope {
				origin.set_caller_from(Origin::<T>::Authorized {
					who: who.clone(),
					scope: scope.clone(),
				});
			}
			return Ok((valid_tx, Some(who), origin));
		}

		// Wrapper call — validate storage authorization for inner calls
		if let Some((_, preserves_origin)) = I::inspect_wrapper(call) {
			let has_storage = Self::traverse_storage_calls(call, 0, &mut |inner_call| {
				Pallet::<T>::validate_signed(&who, inner_call).map(|_| ())
			})?;
			if has_storage {
				if preserves_origin {
					// Transform origin so inner storage dispatches see Authorized.
					origin.set_caller_from(Origin::<T>::Authorized {
						who: who.clone(),
						scope: AuthorizationScope::Account(who.clone()),
					});
				}
				return Ok((ValidTransaction::default(), Some(who), origin));
			}
		}

		// Not a storage-related call
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

		// Direct storage call
		if let Some(inner_call) = call.is_sub_type() {
			Pallet::<T>::pre_dispatch_signed(&who, inner_call)?;
			return Ok(());
		}

		// Wrapper call — consume authorization for inner storage calls
		Self::traverse_storage_calls(call, 0, &mut |inner_call| {
			Pallet::<T>::pre_dispatch_signed(&who, inner_call)
		})?;

		Ok(())
	}
}
