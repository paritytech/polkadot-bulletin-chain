// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

//! Web3 Storage (`pallet-storage-provider`) configuration.

use super::{
	paseo_constants::{consensus::RELAY_CHAIN_SLOT_DURATION_MILLIS, currency::UNITS},
	AccountId, Balance, Balances, BlockNumber, Runtime, RuntimeHoldReason,
};
use frame_support::{
	parameter_types,
	traits::{fungible::HoldConsideration, ConstU32, Get, LinearStoragePrice},
	PalletId,
};
use sp_runtime::traits::AccountIdConversion;

/// The pallet's durations count relay chain blocks.
const RELAY_HOURS: BlockNumber = 60 * 60 * 1000 / RELAY_CHAIN_SLOT_DURATION_MILLIS;

parameter_types! {
	pub storage MinProviderStake: Balance = 1_000 * UNITS;
	pub storage MinStakePerByte: Balance = UNITS / 1_000_000_000;
	pub storage ChallengeTimeout: BlockNumber = 48 * RELAY_HOURS;
	pub storage ChallengeDeposit: Balance = UNITS;
	pub storage SettlementTimeout: BlockNumber = 24 * RELAY_HOURS;
	pub storage RequestTimeout: BlockNumber = 6 * RELAY_HOURS;
	pub storage DeregisterAnnouncementPeriod: BlockNumber = 54 * RELAY_HOURS;
	pub storage MaxChallengesPerDeadline: u16 = 1_000;
	pub storage StorageDepositBase: Balance = UNITS / 10;
	pub storage StorageDepositPerByte: Balance = UNITS / 1_000_000;
	pub const StorageDepositReason: RuntimeHoldReason =
		RuntimeHoldReason::StorageProvider(pallet_storage_provider::HoldReason::StorageDeposit);
	pub const AnchorBlockTimeMillis: u64 = RELAY_CHAIN_SLOT_DURATION_MILLIS as u64;
}

/// Receives slashed stake.
pub struct TreasuryAccount;
impl Get<AccountId> for TreasuryAccount {
	fn get() -> AccountId {
		PalletId(*b"py/trsry").into_account_truncating()
	}
}

impl pallet_storage_provider::Config for Runtime {
	type Currency = Balances;
	type RuntimeHoldReason = RuntimeHoldReason;
	type Treasury = TreasuryAccount;
	type StorageDeposit = HoldConsideration<
		AccountId,
		Balances,
		StorageDepositReason,
		LinearStoragePrice<StorageDepositBase, StorageDepositPerByte, Balance>,
	>;
	type MinStakePerByte = MinStakePerByte;
	type MaxMultiaddrLength = ConstU32<128>;
	type MaxMembers = ConstU32<100>;
	type MaxPrimaryProviders = ConstU32<5>;
	type MinProviderStake = MinProviderStake;
	type MaxChunkSize = ConstU32<{ storage_primitives::DEFAULT_CHUNK_SIZE }>;
	type ChallengeTimeout = ChallengeTimeout;
	type ChallengeDeposit = ChallengeDeposit;
	type SettlementTimeout = SettlementTimeout;
	type RequestTimeout = RequestTimeout;
	type MaxBucketsPerMember = ConstU32<1000>;
	type DeregisterAnnouncementPeriod = DeregisterAnnouncementPeriod;
	type MaxChallengesPerDeadline = MaxChallengesPerDeadline;
	type BlockNumberProvider = cumulus_pallet_parachain_system::RelaychainDataProvider<Runtime>;
	type AnchorBlockTimeMillis = AnchorBlockTimeMillis;
	type WeightInfo = crate::weights::pallet_storage_provider::WeightInfo<Runtime>;
}
