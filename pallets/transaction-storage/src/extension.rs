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

use crate::{pallet::Origin, weights::WeightInfo, Call, Config, Pallet};
use codec::{Decode, DecodeWithMemTracking, Encode};
use core::{fmt, marker::PhantomData};
use polkadot_sdk_frame::{
	deps::*,
	prelude::*,
	traits::{AsSystemOriginSigner, Implication, OriginTrait},
};

type RuntimeCallOf<T> = <T as frame_system::Config>::RuntimeCall;

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
///
/// All other calls and unsigned transactions are passed through unchanged.
#[derive(Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, scale_info::TypeInfo)]
#[scale_info(skip_type_params(T))]
pub struct ValidateStorageCalls<T>(PhantomData<T>);

impl<T> Default for ValidateStorageCalls<T> {
	fn default() -> Self {
		Self(PhantomData)
	}
}

impl<T: Config + Send + Sync> fmt::Debug for ValidateStorageCalls<T> {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "ValidateStorageCalls")
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
		Ok(())
	}
}

impl<T: Config + Send + Sync> TransactionExtension<RuntimeCallOf<T>> for ValidateStorageCalls<T>
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

	/// The signer for store/renew calls, passed from `validate()` to `prepare()`.
	///
	/// For store/renew calls, `validate()` transforms the origin to [`Origin::Authorized`],
	/// so `origin.as_system_origin_signer()` is no longer available in `prepare()`. The signer
	/// is preserved here instead. `None` for all other calls.
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
		// Only handle TransactionStorage calls; pass through others
		let Some(inner_call) = call.is_sub_type() else {
			return Ok((ValidTransaction::default(), None, origin));
		};

		// Get the signer from the origin
		let who = match origin.as_system_origin_signer() {
			Some(who) => who.clone(),
			None => return Ok((ValidTransaction::default(), None, origin)),
		};

		// Validate the call
		let (valid_tx, maybe_scope) = Pallet::<T>::validate_signed(&who, inner_call)?;

		// Transform origin only for store/renew calls (when scope is Some)
		let val = maybe_scope.map(|scope| {
			origin.set_caller_from(Origin::<T>::Authorized { who: who.clone(), scope });
			who
		});

		Ok((valid_tx, val, origin))
	}

	fn prepare(
		self,
		val: Self::Val,
		origin: &T::RuntimeOrigin,
		call: &RuntimeCallOf<T>,
		_info: &DispatchInfoOf<RuntimeCallOf<T>>,
		_len: usize,
	) -> Result<Self::Pre, TransactionValidityError> {
		let Some(inner_call) = call.is_sub_type() else {
			return Ok(());
		};

		// For store/renew: origin was transformed to Authorized, so get `who` from val.
		// For other calls: origin is still the system signer.
		let who = val.as_ref().or_else(|| origin.as_system_origin_signer());
		if let Some(who) = who {
			Pallet::<T>::pre_dispatch_signed(who, inner_call)?;
		}
		Ok(())
	}
}
