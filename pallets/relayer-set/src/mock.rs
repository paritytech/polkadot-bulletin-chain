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

//! Test environment for relayer set pallet.

#![cfg(test)]

use crate as pallet_relayer_set;
use polkadot_sdk_frame::{prelude::*, runtime::prelude::*, testing_prelude::*};

pub type AccountId = u64;
type Block = MockBlock<Test>;

construct_runtime!(
	pub enum Test
	{
		System: frame_system,
		RelayerSet: pallet_relayer_set,
	}
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Nonce = u64;
	type Block = Block;
	type BlockHashCount = ConstU64<250>;
}

parameter_types! {
	pub const BridgeTxFailCooldownBlocks: BlockNumberFor<Test> = 2;
}

impl pallet_relayer_set::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = ();
	type AddRemoveOrigin = EnsureRoot<AccountId>;
	type BridgeTxFailCooldownBlocks = BridgeTxFailCooldownBlocks;
}

pub fn new_test_ext() -> TestExternalities {
	let t = RuntimeGenesisConfig {
		system: Default::default(),
		relayer_set: RelayerSetConfig { initial_relayers: vec![1, 2, 3] },
	}
	.build_storage()
	.unwrap();
	t.into()
}

pub fn next_block() {
	System::on_finalize(System::block_number());
	System::set_block_number(System::block_number() + 1);
	System::on_initialize(System::block_number());
}
