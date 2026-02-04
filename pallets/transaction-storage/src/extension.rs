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

use crate::{cids::CidConfig, Call, CidConfigForStore, Config};
use codec::{Decode, DecodeWithMemTracking, Encode};
use core::{fmt, marker::PhantomData};
use polkadot_sdk_frame::{
	deps::{sp_core::sp_std::prelude::*, *},
	prelude::*,
	traits::{Implication, PostDispatchInfoOf},
};

type RuntimeCallOf<T> = <T as frame_system::Config>::RuntimeCall;

/// `TransactionExtension` implementation that provides optional `CidConfig` for the `store`
/// extrinsic.
#[derive(Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, scale_info::TypeInfo)]
#[scale_info(skip_type_params(T))]
pub struct ProvideCidConfig<T>(pub Option<CidConfig>, PhantomData<T>);

impl<T> ProvideCidConfig<T> {
	/// Create a new `ProvideCidConfig` instance.
	pub fn new(config: Option<CidConfig>) -> Self {
		Self(config, Default::default())
	}
}

impl<T: Config + Send + Sync> fmt::Debug for ProvideCidConfig<T> {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "ProvideCidConfig({:?})", self.0)
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
		Ok(())
	}
}

impl<T: Config + Send + Sync> TransactionExtension<RuntimeCallOf<T>> for ProvideCidConfig<T>
where
	RuntimeCallOf<T>: IsSubType<Call<T>>,
{
	const IDENTIFIER: &'static str = "ProvideCidConfig";

	type Implicit = ();
	fn implicit(&self) -> Result<Self::Implicit, TransactionValidityError> {
		Ok(())
	}

	type Val = Option<CidConfig>;
	type Pre = bool;

	fn weight(&self, _call: &RuntimeCallOf<T>) -> Weight {
		Weight::zero()
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
		match (self.0.as_ref(), call.is_sub_type()) {
			(Some(cid_config), Some(Call::store { .. })) =>
				Ok((Default::default(), Some(cid_config.clone()), origin)),
			(Some(_), _) => {
				// All other calls are invalid with cid_codec.
				Err(InvalidTransaction::Call.into())
			},
			_ => Ok((Default::default(), None, origin)),
		}
	}

	fn prepare(
		self,
		val: Self::Val,
		_: &T::RuntimeOrigin,
		_: &RuntimeCallOf<T>,
		_info: &DispatchInfoOf<RuntimeCallOf<T>>,
		_len: usize,
	) -> Result<Self::Pre, TransactionValidityError> {
		if let Some(cid_config) = val {
			// Let's store the codec in the intermediary storage, which will be cleared by the store
			// extrinsic.
			CidConfigForStore::<T>::set(Some(cid_config));
			Ok(true)
		} else {
			Ok(false)
		}
	}

	fn post_dispatch_details(
		pre: Self::Pre,
		_info: &DispatchInfoOf<RuntimeCallOf<T>>,
		_post_info: &PostDispatchInfoOf<RuntimeCallOf<T>>,
		_len: usize,
		_result: &DispatchResult,
	) -> Result<Weight, TransactionValidityError> {
		if pre {
			// Let's clean up after the dispatch.
			CidConfigForStore::<T>::kill();
		}
		Ok(Weight::zero())
	}
}
