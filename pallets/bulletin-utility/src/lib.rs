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

//! Utility wrapper pallet.
//!
//! Re-exposes `pallet-utility`'s `batch`, `batch_all` and `force_batch` and delegates execution to
//! it. The only addition is a `feeless_if` on each call that holds when the batch is non-empty and
//! every inner call is itself feeless, so a batch of feeless calls is not charged a fee.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use alloc::vec::Vec;
use pallet_utility::WeightInfo;
use polkadot_sdk_frame::{
	deps::frame_support::{
		dispatch::CheckIfFeeless,
		traits::{IsSubType, OriginTrait},
	},
	prelude::*,
};

pub use pallet::*;

type UtilityCallOf<T> = <T as pallet_utility::Config>::RuntimeCall;

/// Accumulated call weight and dispatch class of `calls`, mirroring `pallet_utility`'s own
/// `weight_and_dispatch_class` (which is private).
fn aggregate_dispatch<T: Config>(calls: &[UtilityCallOf<T>]) -> (Weight, DispatchClass) {
	calls.iter().map(|call| call.get_dispatch_info()).fold(
		(Weight::zero(), DispatchClass::Operational),
		|(total_weight, dispatch_class), di| {
			(
				total_weight.saturating_add(di.call_weight),
				if di.class == DispatchClass::Normal { di.class } else { dispatch_class },
			)
		},
	)
}

#[polkadot_sdk_frame::pallet]
pub mod pallet {
	use super::*;

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config:
		frame_system::Config
		+ pallet_utility::Config<
			RuntimeCall: CheckIfFeeless<Origin = OriginFor<Self>> + IsSubType<Call<Self>>,
		>
	{
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Feeless-aware [`pallet_utility::Pallet::batch`].
		#[pallet::call_index(0)]
		#[pallet::weight({
			let (weight, class) = aggregate_dispatch::<T>(calls);
			(weight.saturating_add(<T as pallet_utility::Config>::WeightInfo::batch(calls.len() as u32)), class)
		})]
		#[pallet::feeless_if(|origin: &OriginFor<T>, calls: &Vec<UtilityCallOf<T>>| -> bool {
			!calls.is_empty() && calls.iter().all(|call| call.is_feeless(origin))
		})]
		pub fn batch(
			origin: OriginFor<T>,
			calls: Vec<UtilityCallOf<T>>,
		) -> DispatchResultWithPostInfo {
			pallet_utility::Pallet::<T>::batch(origin, calls)
		}

		/// Feeless-aware [`pallet_utility::Pallet::batch_all`].
		#[pallet::call_index(1)]
		#[pallet::weight({
			let (weight, class) = aggregate_dispatch::<T>(calls);
			(weight.saturating_add(<T as pallet_utility::Config>::WeightInfo::batch_all(calls.len() as u32)), class)
		})]
		#[pallet::feeless_if(|origin: &OriginFor<T>, calls: &Vec<UtilityCallOf<T>>| -> bool {
			!calls.is_empty() && calls.iter().all(|call| call.is_feeless(origin))
		})]
		pub fn batch_all(
			mut origin: OriginFor<T>,
			calls: Vec<UtilityCallOf<T>>,
		) -> DispatchResultWithPostInfo {
			// `pallet_utility::batch_all` blocks nested `batch_all`, but its filter only matches
			// `pallet_utility::Call::batch_all`. Block this pallet's variant too, so the
			// no-nested-atomic-batch guarantee holds through the wrapper.
			origin.add_filter(|call: &<T as frame_system::Config>::RuntimeCall| {
				!matches!(
					UtilityCallOf::<T>::from_ref(call).is_sub_type(),
					Some(Call::batch_all { .. })
				)
			});
			pallet_utility::Pallet::<T>::batch_all(origin, calls)
		}

		/// Feeless-aware [`pallet_utility::Pallet::force_batch`].
		#[pallet::call_index(2)]
		#[pallet::weight({
			let (weight, class) = aggregate_dispatch::<T>(calls);
			(weight.saturating_add(<T as pallet_utility::Config>::WeightInfo::force_batch(calls.len() as u32)), class)
		})]
		#[pallet::feeless_if(|origin: &OriginFor<T>, calls: &Vec<UtilityCallOf<T>>| -> bool {
			!calls.is_empty() && calls.iter().all(|call| call.is_feeless(origin))
		})]
		pub fn force_batch(
			origin: OriginFor<T>,
			calls: Vec<UtilityCallOf<T>>,
		) -> DispatchResultWithPostInfo {
			pallet_utility::Pallet::<T>::force_batch(origin, calls)
		}
	}
}
