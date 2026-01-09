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

//! Relayer set pallet. Maintains a set of relayers which can be added to or removed from by a
//! privileged origin.

#![cfg_attr(not(feature = "std"), no_std)]

mod benchmarking;
mod mock;
mod tests;
pub mod weights;

use codec::{Decode, Encode, MaxEncodedLen};
use polkadot_sdk_frame::{deps::sp_core::sp_std::prelude::*, prelude::*};

// Re-export pallet items so that they can be accessed from the crate namespace.
pub use pallet::*;
pub use weights::WeightInfo;

const LOG_TARGET: &str = "runtime::relayer-set";

/// Per-relayer data stored by this pallet.
#[derive(Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
struct Relayer<BlockNumber> {
	/// The relayer may not submit any bridge transactions before this block.
	min_bridge_tx_block: BlockNumber,
}

#[polkadot_sdk_frame::pallet]
pub mod pallet {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// The overarching event type.
		#[allow(deprecated)]
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// Weight information for extrinsics in this pallet.
		type WeightInfo: WeightInfo;
		/// Origin for adding or removing a relayer.
		type AddRemoveOrigin: EnsureOrigin<Self::RuntimeOrigin>;
		/// Number of cooldown blocks after a bad bridge transaction signed by a relayer. The
		/// relayer is blocked from submitting bridge transactions during the cooldown period.
		#[pallet::constant]
		type BridgeTxFailCooldownBlocks: Get<BlockNumberFor<Self>>;
	}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	/// Relayer set. Unlike eg the validator set, changes to this take effect immediately.
	#[pallet::storage]
	pub(super) type Relayers<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, Relayer<BlockNumberFor<T>>, OptionQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// New relayer added.
		RelayerAdded(T::AccountId),
		/// Relayer removed.
		RelayerRemoved(T::AccountId),
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Relayer is already in the relayer set.
		Duplicate,
		/// Relayer is not in the relayer set.
		NotARelayer,
	}

	#[pallet::genesis_config]
	#[derive(DefaultNoBound)]
	pub struct GenesisConfig<T: Config> {
		pub initial_relayers: Vec<T::AccountId>,
	}

	#[pallet::genesis_build]
	impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
		fn build(&self) {
			assert!(Relayers::<T>::iter().next().is_none(), "Relayers are already initialized");
			for who in &self.initial_relayers {
				assert!(Pallet::<T>::do_add_relayer(who).is_ok());
			}
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Add a new relayer.
		///
		/// The origin for this call must be the pallet's `AddRemoveOrigin`. Emits
		/// [`RelayerAdded`](Event::RelayerAdded) when successful.
		#[pallet::call_index(0)]
		#[pallet::weight(<T as Config>::WeightInfo::add_relayer())]
		pub fn add_relayer(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AddRemoveOrigin::ensure_origin(origin)?;
			Self::do_add_relayer(&who)?;
			Self::deposit_event(Event::RelayerAdded(who));
			Ok(())
		}

		/// Remove a relayer.
		///
		/// The origin for this call must be the pallet's `AddRemoveOrigin`. Emits
		/// [`RelayerRemoved`](Event::RelayerRemoved) when successful.
		#[pallet::call_index(1)]
		#[pallet::weight(<T as Config>::WeightInfo::remove_relayer())]
		pub fn remove_relayer(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AddRemoveOrigin::ensure_origin(origin)?;
			ensure!(Self::do_remove_relayer(&who), Error::<T>::NotARelayer);
			Self::deposit_event(Event::RelayerRemoved(who));
			Ok(())
		}
	}
}

impl<T: Config> Pallet<T> {
	pub fn relayers() -> Vec<T::AccountId> {
		Relayers::<T>::iter_keys().collect()
	}

	fn do_add_relayer(who: &T::AccountId) -> DispatchResult {
		Relayers::<T>::mutate(who, |relayer| {
			if !relayer.is_none() {
				return Err(Error::<T>::Duplicate)
			}
			*relayer = Some(Relayer { min_bridge_tx_block: Zero::zero() });
			Ok(())
		})?;

		frame_system::Pallet::<T>::inc_providers(who);

		Ok(())
	}

	/// Returns `false` if `who` is not a relayer.
	fn do_remove_relayer(who: &T::AccountId) -> bool {
		if Relayers::<T>::take(who).is_none() {
			return false
		}

		// Decrement who's provider reference count
		if let Err(err) = frame_system::Pallet::<T>::dec_providers(who) {
			log::warn!(
				target: LOG_TARGET,
				"Failed to decrement provider reference count for relayer {who:?}, \
				leaking reference: {err:?}"
			);
		}

		true
	}

	/// Check the validity of a bridge transaction signed by `who`.
	pub fn validate_bridge_tx(who: &T::AccountId) -> Result<(), TransactionValidityError> {
		match Relayers::<T>::get(who) {
			Some(relayer) =>
				if frame_system::Pallet::<T>::block_number() < relayer.min_bridge_tx_block {
					Err(InvalidTransaction::Future.into())
				} else {
					Ok(())
				},
			None => Err(InvalidTransaction::BadSigner.into()),
		}
	}

	/// Call after a failed bridge transaction signed by `who`.
	pub fn post_dispatch_failed_bridge_tx(who: &T::AccountId) {
		Relayers::<T>::mutate(who, |relayer| match relayer {
			Some(relayer) =>
				relayer.min_bridge_tx_block = frame_system::Pallet::<T>::block_number()
					.saturating_add(T::BridgeTxFailCooldownBlocks::get()),
			None => log::warn!(
				target: LOG_TARGET,
				"Could not find signer {who:?} of failed bridge transaction in relayer set"
			),
		});
	}
}
