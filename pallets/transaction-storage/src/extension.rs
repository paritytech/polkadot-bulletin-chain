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

use crate::{Call, CidCodecForStore, Config, LOG_TARGET};
use codec::{Decode, DecodeWithMemTracking, Encode};
use core::{fmt, marker::PhantomData};
use polkadot_sdk_frame::{
	deps::{sp_core::sp_std::prelude::*, *},
	prelude::*,
	traits::Implication,
};

/// Type alias representing a CID codec.
pub type CidCodec = u64;

/// Temporarily tracks provided optional CID codec.
#[derive(Default)]
pub struct CidCodecContext {
	pub codec: Option<CidCodec>,
}

/// `TransactionExtension` implementation that provides optional `CidCodec` for the `store`
/// extrinsic.
#[derive(Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, scale_info::TypeInfo)]
#[scale_info(skip_type_params(T))]
pub struct ProvideCidCodec<T>(pub Option<CidCodec>, PhantomData<T>);

impl<T> ProvideCidCodec<T> {
	/// Create a new `ProvideCidCodec` instance.
	pub fn new(cid_codec: Option<CidCodec>) -> Self {
		Self(cid_codec, Default::default())
	}
}

impl<T: Config + Send + Sync> fmt::Debug for ProvideCidCodec<T> {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "ProvideCidCodec({:?})", self.0)
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
		Ok(())
	}
}

impl<T: Config + Send + Sync> TransactionExtension<T::RuntimeCall> for ProvideCidCodec<T>
where
	<T as frame_system::Config>::RuntimeCall: IsSubType<Call<T>>,
{
	const IDENTIFIER: &'static str = "ProvideCidCodec";

	type Implicit = ();
	fn implicit(&self) -> Result<Self::Implicit, TransactionValidityError> {
		Ok(())
	}

	type Val = Option<CidCodec>;
	type Pre = ();

	fn weight(&self, _call: &T::RuntimeCall) -> Weight {
		Weight::zero()
	}

	fn validate(
		&self,
		origin: T::RuntimeOrigin,
		call: &T::RuntimeCall,
		_info: &DispatchInfoOf<T::RuntimeCall>,
		_len: usize,
		_self_implicit: Self::Implicit,
		_inherited_implication: &impl Implication,
		_source: TransactionSource,
	) -> ValidateResult<Self::Val, T::RuntimeCall> {
		match (self.0, call.is_sub_type()) {
			(Some(cid_codec), Some(Call::store { .. })) =>
				Ok((Default::default(), Some(cid_codec), origin)),
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
		_: &T::RuntimeCall,
		_info: &DispatchInfoOf<T::RuntimeCall>,
		_len: usize,
	) -> Result<Self::Pre, TransactionValidityError> {
		log::error!(target: LOG_TARGET, "prepare: {val:?}");
		if let Some(cid_codec) = val {
			CidCodecForStore::<T>::set(Some(cid_codec));

			// TODO: just attempt, not working, will remove.
			// Put cid codec to the dispatch context
			// with_context::<CidCodecContext, _>(|v| {
			// 	let context = v.or_default();
			// 	context.codec = Some(cid_codec);
			// 	log::error!(target: LOG_TARGET, "prepare - setting: {cid_codec:?}!");
			// });
		}
		Ok(())
	}
}
