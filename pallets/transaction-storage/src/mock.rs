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
	self as pallet_bulletin_transaction_storage, TransactionStorageProof,
	DEFAULT_MAX_BLOCK_TRANSACTIONS, DEFAULT_MAX_TRANSACTION_SIZE,
};
use bulletin_pallets_common::NoCurrency;
use polkadot_sdk_frame::{prelude::*, runtime::prelude::*, testing_prelude::*};

type Block = MockBlock<Test>;

// Configure a mock runtime to test the pallet.
construct_runtime!(
	pub enum Test
	{
		System: frame_system,
		TransactionStorage: pallet_bulletin_transaction_storage,
	}
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Nonce = u64;
	type Block = Block;
	type BlockHashCount = ConstU64<250>;
}

/// One period is 10 seconds in tests; small enough that period rollover can be
/// driven by a single `set_period` jump in any test that cares.
pub const PERIOD_DURATION: u64 = 10;

std::thread_local! {
	/// Thread-local mock clock, in seconds since the unix epoch. Tests advance it via
	/// [`set_period`]; `MockUnixTime` reads it to satisfy the pallet's `TimeProvider`.
	/// Defaults to 0 so a test that never sets a period still sees `current_period() == 0`.
	pub(crate) static MOCK_NOW_SECS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// `UnixTime` impl backed by [`MOCK_NOW_SECS`]. Avoids pulling in `pallet_timestamp`,
/// which would require driving the timestamp extrinsic from `run_to_block`.
pub struct MockUnixTime;
impl polkadot_sdk_frame::traits::UnixTime for MockUnixTime {
	fn now() -> core::time::Duration {
		core::time::Duration::from_secs(MOCK_NOW_SECS.with(|c| c.get()))
	}
}

parameter_types! {
	pub const StoreRenewPriority: TransactionPriority = TransactionPriority::MAX;
	pub const StoreRenewLongevity: TransactionLongevity = 10;
	pub const RemoveExpiredAuthorizationPriority: TransactionPriority = TransactionPriority::MAX;
	pub const RemoveExpiredAuthorizationLongevity: TransactionLongevity = 10;
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
	type MaxPermanentStorageSize = ConstU64<{ u64::MAX }>;
	type TimeProvider = MockUnixTime;
	type PeriodDuration = ConstU64<PERIOD_DURATION>;
	type Authorizer = EnsureRoot<Self::AccountId>;
	type StoreRenewPriority = StoreRenewPriority;
	type StoreRenewLongevity = StoreRenewLongevity;
	type RemoveExpiredAuthorizationPriority = RemoveExpiredAuthorizationPriority;
	type RemoveExpiredAuthorizationLongevity = RemoveExpiredAuthorizationLongevity;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = crate::benchmarking::DefaultCheckProofHelper;
}

/// Set the mock unix-time clock to the start of `period` (i.e., `period * PERIOD_DURATION`
/// seconds). Tests use this to drive period rollover without churning blocks.
pub fn set_period(period: u32) {
	MOCK_NOW_SECS.with(|c| c.set((period as u64) * PERIOD_DURATION));
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
		},
	}
	.build_storage()
	.unwrap();
	t.into()
}

pub fn run_to_block(n: u64, f: impl Fn() -> Option<TransactionStorageProof> + 'static) {
	System::run_to_block_with::<AllPalletsWithSystem>(
		n,
		RunToBlockHooks::default().before_finalize(|_| {
			if let Some(proof) = f() {
				TransactionStorage::check_proof(RuntimeOrigin::none(), proof).unwrap();
			}
		}),
	);
}
