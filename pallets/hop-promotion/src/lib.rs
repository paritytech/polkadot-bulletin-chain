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

//! # HOP Promotion Pallet
//!
//! Promotes near-expiry HOP pool data to permanent chain storage via
//! `pallet-transaction-storage`. Uses general transactions with
//! `#[pallet::authorize]` — no signature, no fees, priority 0.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {
	use alloc::vec::Vec;
	use bulletin_transaction_storage_primitives::cids::{HashingAlgorithm, RAW_CODEC};
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use pallet_bulletin_transaction_storage::WeightInfo as _;

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_bulletin_transaction_storage::Config {}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::call_index(0)]
		#[pallet::weight(
			<T as pallet_bulletin_transaction_storage::Config>::WeightInfo::store(data.len() as u32)
		)]
		#[pallet::authorize(|source, data: &Vec<u8>| {
			if matches!(source, TransactionSource::External) {
				return Err(InvalidTransaction::Call.into());
			}
			if data.is_empty() ||
				data.len() >
					<T as pallet_bulletin_transaction_storage::Config>::MaxTransactionSize::get() as usize
			{
				return Err(InvalidTransaction::Custom(0).into());
			}
			Ok((
				ValidTransaction::with_tag_prefix("HopPromotion")
					.priority(0)
					.longevity(5)
					.propagate(false)
					.and_provides(sp_io::hashing::blake2_256(data))
					.build()
					.expect("builder always succeeds; qed"),
				Weight::zero(),
			))
		})]
		#[pallet::weight_of_authorize(Weight::zero())]
		pub fn promote(origin: OriginFor<T>, data: Vec<u8>) -> DispatchResult {
			ensure_authorized(origin)?;
			pallet_bulletin_transaction_storage::Pallet::<T>::do_store(
				data,
				HashingAlgorithm::Blake2b256,
				RAW_CODEC,
			)
		}
	}
}
