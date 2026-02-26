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

//! Storage Provider pallet configuration.

use super::*;
use frame_support::{parameter_types, PalletId};
use sp_runtime::traits::AccountIdConversion;

parameter_types! {
	pub const MinProviderStake: Balance = 1_000 * UNITS;
	pub const ChallengeTimeout: BlockNumber = 48 * HOURS;
	pub const SettlementTimeout: BlockNumber = 24 * HOURS;
	pub const RequestTimeout: BlockNumber = 6 * HOURS;
	pub const MinStakePerByte: Balance = 1_000;
	pub const DefaultCheckpointInterval: BlockNumber = 100;
	pub const DefaultCheckpointGrace: BlockNumber = 20;
	pub const CheckpointReward: Balance = 1_000_000_000_000; // 1 token
	pub const CheckpointMissPenalty: Balance = 500_000_000_000; // 0.5 token
}

pub struct StorageProviderTreasury;
impl frame_support::traits::Get<AccountId> for StorageProviderTreasury {
	fn get() -> AccountId {
		PalletId(*b"w3s/trsy").into_account_truncating()
	}
}

impl pallet_storage_provider::Config for Runtime {
	type Currency = Balances;
	type Treasury = StorageProviderTreasury;
	type MinStakePerByte = MinStakePerByte;
	type MaxMultiaddrLength = ConstU32<128>;
	type MaxMembers = ConstU32<100>;
	type MaxPrimaryProviders = ConstU32<5>;
	type MinProviderStake = MinProviderStake;
	type MaxChunkSize = ConstU32<262144>; // 256 KiB
	type ChallengeTimeout = ChallengeTimeout;
	type SettlementTimeout = SettlementTimeout;
	type RequestTimeout = RequestTimeout;
	type DefaultCheckpointInterval = DefaultCheckpointInterval;
	type DefaultCheckpointGrace = DefaultCheckpointGrace;
	type CheckpointReward = CheckpointReward;
	type CheckpointMissPenalty = CheckpointMissPenalty;
	type WeightInfo = pallet_storage_provider::weights::SubstrateWeight<Runtime>;
}
