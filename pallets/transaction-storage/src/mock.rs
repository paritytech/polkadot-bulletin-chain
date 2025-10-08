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

//! Test environment for transaction-storage pallet.

use crate::{
	self as pallet_transaction_storage, TransactionStorageProof, DEFAULT_MAX_BLOCK_TRANSACTIONS,
	DEFAULT_MAX_TRANSACTION_SIZE,
};
use polkadot_sdk_frame::{prelude::*, runtime::prelude::*, testing_prelude::*};

type Block = MockBlock<Test>;

// Configure a mock runtime to test the pallet.
construct_runtime!(
	pub enum Test
	{
		System: frame_system,
		TransactionStorage: pallet_transaction_storage,
	}
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Nonce = u64;
	type Block = Block;
	type BlockHashCount = ConstU64<250>;
}

parameter_types! {
	pub const StoragePeriod: BlockNumberFor<Test> = 10;
	pub const AuthorizationPeriod: BlockNumberFor<Test> = 10;
	pub const StoreRenewPriority: TransactionPriority = TransactionPriority::MAX;
	pub const StoreRenewLongevity: TransactionLongevity = 10;
	pub const RemoveExpiredAuthorizationPriority: TransactionPriority = TransactionPriority::MAX;
	pub const RemoveExpiredAuthorizationLongevity: TransactionLongevity = 10;
}

impl pallet_transaction_storage::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = ();
	type MaxBlockTransactions = ConstU32<{ DEFAULT_MAX_BLOCK_TRANSACTIONS }>;
	type MaxTransactionSize = ConstU32<{ DEFAULT_MAX_TRANSACTION_SIZE }>;
	type StoragePeriod = StoragePeriod;
	type AuthorizationPeriod = AuthorizationPeriod;
	type Authorizer = EnsureRoot<Self::AccountId>;
	type StoreRenewPriority = StoreRenewPriority;
	type StoreRenewLongevity = StoreRenewLongevity;
	type RemoveExpiredAuthorizationPriority = RemoveExpiredAuthorizationPriority;
	type RemoveExpiredAuthorizationLongevity = RemoveExpiredAuthorizationLongevity;
}

pub fn new_test_ext() -> TestExternalities {
	let t = RuntimeGenesisConfig { system: Default::default() }.build_storage().unwrap();
	t.into()
}

pub fn run_to_block(n: u64, f: impl Fn() -> Option<TransactionStorageProof>) {
	while System::block_number() < n {
		if let Some(proof) = f() {
			TransactionStorage::check_proof(RuntimeOrigin::none(), proof).unwrap();
		}
		TransactionStorage::on_finalize(System::block_number());
		System::on_finalize(System::block_number());
		System::set_block_number(System::block_number() + 1);
		System::on_initialize(System::block_number());
		TransactionStorage::on_initialize(System::block_number());
	}
}
