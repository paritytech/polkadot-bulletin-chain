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

//! Test environment for the bulletin-utility pallet.

use crate as pallet_bulletin_utility;
use polkadot_sdk_frame::{
	deps::{frame_support, frame_system},
	prelude::*,
	runtime::prelude::*,
	testing_prelude::*,
};

type Block = MockBlock<Test>;

/// Minimal pallet providing one feeless call and one fee-charging call, so tests can build
/// batches whose feeless status varies.
#[frame_support::pallet]
pub mod dummy {
	use super::*;

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config {}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::call_index(0)]
		#[pallet::weight(Weight::zero())]
		#[pallet::feeless_if(|_origin: &OriginFor<T>| -> bool { true })]
		pub fn feeless_noop(origin: OriginFor<T>) -> DispatchResult {
			ensure_signed(origin)?;
			Ok(())
		}

		#[pallet::call_index(1)]
		#[pallet::weight(Weight::zero())]
		pub fn paid_noop(origin: OriginFor<T>) -> DispatchResult {
			ensure_signed(origin)?;
			Ok(())
		}
	}
}

#[frame_support::runtime]
mod runtime {
	#[runtime::runtime]
	#[runtime::derive(
		RuntimeCall,
		RuntimeEvent,
		RuntimeError,
		RuntimeOrigin,
		RuntimeTask,
		RuntimeFreezeReason,
		RuntimeHoldReason,
		RuntimeSlashReason,
		RuntimeLockId,
		RuntimeViewFunction
	)]
	pub struct Test;

	#[runtime::pallet_index(0)]
	pub type System = frame_system;

	#[runtime::pallet_index(1)]
	pub type Utility = pallet_utility;

	#[runtime::pallet_index(2)]
	pub type BulletinUtility = pallet_bulletin_utility;

	#[runtime::pallet_index(3)]
	pub type Dummy = dummy;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
}

impl pallet_utility::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type PalletsOrigin = OriginCaller;
	type WeightInfo = ();
}

impl pallet_bulletin_utility::Config for Test {}

impl dummy::Config for Test {}

pub fn new_test_ext() -> TestExternalities {
	frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}
