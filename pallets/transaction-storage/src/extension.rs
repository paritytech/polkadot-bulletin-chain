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

use crate::{pallet::Origin, AuthorizationScopeFor, Call, Config, Pallet};
use codec::{Decode, DecodeWithMemTracking, Encode};
use core::{fmt, marker::PhantomData};
use polkadot_sdk_frame::{
	deps::*,
	prelude::*,
	traits::{AsSystemOriginSigner, Implication, OriginTrait},
};

type RuntimeCallOf<T> = <T as frame_system::Config>::RuntimeCall;

/// Transaction extension that authorizes signed TransactionStorage calls.
///
/// This extension handles **signed TransactionStorage transactions** via
/// [`Pallet::validate_signed`]:
/// - **Store/renew calls**: Validates authorization and transforms the origin to
///   [`Origin::Authorized`] to carry authorization info.
/// - **Authorizer calls** (authorize_*, refresh_*): Validates that the signer satisfies the
///   [`Config::Authorizer`] origin requirement.
///
/// All other calls and unsigned transactions are passed through unchanged.
#[derive(Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, scale_info::TypeInfo)]
#[scale_info(skip_type_params(T))]
pub struct AuthorizeStorageSigned<T>(PhantomData<T>);

impl<T> Default for AuthorizeStorageSigned<T> {
	fn default() -> Self {
		Self(PhantomData)
	}
}

impl<T: Config + Send + Sync> fmt::Debug for AuthorizeStorageSigned<T> {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "AuthorizeStorageSigned")
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
		Ok(())
	}
}

impl<T: Config + Send + Sync> TransactionExtension<RuntimeCallOf<T>> for AuthorizeStorageSigned<T>
where
	RuntimeCallOf<T>: IsSubType<Call<T>>,
	T::RuntimeOrigin: OriginTrait + AsSystemOriginSigner<T::AccountId> + From<Origin<T>>,
	<T::RuntimeOrigin as OriginTrait>::PalletsOrigin: From<Origin<T>> + TryInto<Origin<T>>,
{
	const IDENTIFIER: &'static str = "AuthorizeStorageSigned";

	type Implicit = ();
	fn implicit(&self) -> Result<Self::Implicit, TransactionValidityError> {
		Ok(())
	}

	/// `Some(AuthorizationScope)` for store/renew calls, `None` for passthrough.
	type Val = Option<AuthorizationScopeFor<T>>;
	type Pre = ();

	fn weight(&self, _call: &RuntimeCallOf<T>) -> Weight {
		Weight::zero()
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
		if let Some(ref scope) = maybe_scope {
			origin.set_caller_from(Origin::<T>::Authorized { who, scope: scope.clone() });
		}

		Ok((valid_tx, maybe_scope, origin))
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

		// For store/renew calls (val is Some), the origin was transformed in validate()
		// to Origin::Authorized. Extract the account from the transformed origin.
		if val.is_some() {
			let who = match origin.clone().into_caller().try_into() {
				Ok(Origin::<T>::Authorized { who, .. }) => who,
				Err(_) => return Err(InvalidTransaction::BadSigner.into()),
			};
			Pallet::<T>::pre_dispatch_signed(&who, inner_call)?;
			return Ok(());
		}

		// For other TransactionStorage calls (authorizer calls), the origin is unchanged.
		if let Some(who) = origin.as_system_origin_signer() {
			Pallet::<T>::pre_dispatch_signed(who, inner_call)?;
		}
		Ok(())
	}
}
