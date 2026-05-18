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

//! Test environment for `pallet-storage-auto-renewal`.
//!
//! Wires together a real `pallet-bulletin-transaction-storage` plus this pallet so the
//! full expiring → enqueue → process_auto_renewals → finalize cycle can be exercised.

use crate as pallet_storage_auto_renewal;
use bulletin_pallets_common::NoCurrency;
use pallet_bulletin_transaction_storage::{
	EnsureAllowedAuthorizers, DEFAULT_MAX_BLOCK_TRANSACTIONS, DEFAULT_MAX_TRANSACTION_SIZE,
};
use polkadot_sdk_frame::{
	deps::{frame_support, frame_system},
	prelude::*,
	runtime::prelude::*,
	testing_prelude::*,
	traits::EitherOfDiverse,
};

type Block = MockBlock<Test>;

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
	pub type TransactionStorage = pallet_bulletin_transaction_storage;

	#[runtime::pallet_index(2)]
	pub type AutoRenewal = pallet_storage_auto_renewal;
}

parameter_types! {
	pub const TestDbWeight: polkadot_sdk_frame::deps::frame_support::weights::RuntimeDbWeight =
		polkadot_sdk_frame::deps::frame_support::weights::RuntimeDbWeight {
			read: 1_000_000,
			write: 5_000_000,
		};
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Nonce = u64;
	type Block = Block;
	type BlockHashCount = ConstU64<250>;
	type DbWeight = TestDbWeight;
}

parameter_types! {
	pub const AuthorizationPeriod: BlockNumberFor<Test> = 10;
	pub const StoreRenewPriority: TransactionPriority = TransactionPriority::MAX;
	pub const StoreRenewLongevity: TransactionLongevity = 10;
	pub const RemoveExpiredAuthorizationPriority: TransactionPriority = TransactionPriority::MAX;
	pub const RemoveExpiredAuthorizationLongevity: TransactionLongevity = 10;
	pub storage MaxPermanentStorageSize: u64 = u64::MAX;
}

impl pallet_bulletin_transaction_storage::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type Currency = NoCurrency<Self::AccountId, RuntimeHoldReason>;
	type RuntimeHoldReason = RuntimeHoldReason;
	type FeeDestination = ();
	type WeightInfo = ();
	type MaxBlockTransactions = ConstU32<{ DEFAULT_MAX_BLOCK_TRANSACTIONS }>;
	type MaxTransactionSize = ConstU32<{ DEFAULT_MAX_TRANSACTION_SIZE }>;
	type MaxPermanentStorageSize = MaxPermanentStorageSize;
	type AuthorizationPeriod = AuthorizationPeriod;
	type AuthorizerRegistrarOrigin = EnsureRoot<Self::AccountId>;
	type Authorizer = EitherOfDiverse<EnsureRoot<Self::AccountId>, EnsureAllowedAuthorizers<Self>>;
	type StoreRenewPriority = StoreRenewPriority;
	type StoreRenewLongevity = StoreRenewLongevity;
	type RemoveExpiredAuthorizationPriority = RemoveExpiredAuthorizationPriority;
	type RemoveExpiredAuthorizationLongevity = RemoveExpiredAuthorizationLongevity;
	type OnTransactionExpiring = AutoRenewal;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper =
		pallet_bulletin_transaction_storage::benchmarking::DefaultCheckProofHelper;
}

impl pallet_storage_auto_renewal::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = ();
	type MaxBlockTransactions = ConstU32<{ DEFAULT_MAX_BLOCK_TRANSACTIONS }>;
	type StorageRenewer = TransactionStorage;
}

pub fn new_test_ext() -> TestExternalities {
	let t = RuntimeGenesisConfig {
		system: Default::default(),
		transaction_storage: pallet_bulletin_transaction_storage::GenesisConfig::<Test> {
			retention_period: 10,
			byte_fee: 2,
			entry_fee: 200,
			account_authorizations: vec![],
			preimage_authorizations: vec![],
			allowed_authorizers: vec![],
		},
	}
	.build_storage()
	.unwrap();
	t.into()
}
