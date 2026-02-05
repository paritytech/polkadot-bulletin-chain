// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Cumulus.
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

//! Storage-specific configurations.

use super::{AccountId, Runtime, RuntimeCall, RuntimeEvent, RuntimeHoldReason};
use frame_support::{
	parameter_types,
	traits::{EitherOfDiverse, EnsureOrigin, Equals},
};
use pallet_xcm::EnsureXcm;
use pallets_common::NoCurrency;
use sp_runtime::transaction_validity::{TransactionLongevity, TransactionPriority};
use testnet_parachains_constants::westend::locations::PeopleLocation;

/// Alice's well-known account ID bytes (Sr25519).
/// SS58: 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY
const ALICE_ACCOUNT_ID: [u8; 32] = [
	0xd4, 0x35, 0x93, 0xc7, 0x15, 0xfd, 0xd3, 0x1c, 0x61, 0x14, 0x1a, 0xbd, 0x04, 0xa9, 0x9f, 0xd6,
	0x82, 0x2c, 0x85, 0x58, 0x85, 0x4c, 0xcd, 0xe3, 0x9a, 0x56, 0x84, 0xe7, 0xa5, 0x6d, 0xa2, 0x7d,
];

/// Ensures that the origin is the well-known Alice test account.
pub struct EnsureAlice;
impl<OuterOrigin> EnsureOrigin<OuterOrigin> for EnsureAlice
where
	OuterOrigin: Into<Result<frame_system::RawOrigin<AccountId>, OuterOrigin>> + Clone,
{
	type Success = ();

	fn try_origin(o: OuterOrigin) -> Result<Self::Success, OuterOrigin> {
		let alice_account = AccountId::from(ALICE_ACCOUNT_ID);
		o.clone().into().and_then(|raw_origin| match raw_origin {
			frame_system::RawOrigin::Signed(who) if who == alice_account => Ok(()),
			_ => Err(o),
		})
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn try_successful_origin() -> Result<OuterOrigin, ()> {
		Err(())
	}
}

parameter_types! {
	pub const AuthorizationPeriod: crate::BlockNumber = 7 * crate::DAYS;
	// Priorities and longevities used by the transaction storage pallet extrinsics.
	pub const SudoPriority: TransactionPriority = TransactionPriority::MAX;
	pub const SetPurgeKeysPriority: TransactionPriority = SudoPriority::get() - 1;
	pub const SetPurgeKeysLongevity: TransactionLongevity = crate::HOURS as TransactionLongevity;
	pub const RemoveExpiredAuthorizationPriority: TransactionPriority = SetPurgeKeysPriority::get() - 1;
	pub const RemoveExpiredAuthorizationLongevity: TransactionLongevity = crate::DAYS as TransactionLongevity;
	pub const StoreRenewPriority: TransactionPriority = RemoveExpiredAuthorizationPriority::get() - 1;
	pub const StoreRenewLongevity: TransactionLongevity = crate::DAYS as TransactionLongevity;
}

/// The main business of the Bulletin chain.
impl pallet_transaction_storage::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type Currency = NoCurrency<Self::AccountId, RuntimeHoldReason>;
	type RuntimeHoldReason = RuntimeHoldReason;
	type FeeDestination = ();
	type WeightInfo = crate::weights::pallet_transaction_storage::WeightInfo<Runtime>;
	type MaxBlockTransactions = crate::ConstU32<512>;
	/// Max transaction size per block needs to be aligned with `BlockLength`.
	type MaxTransactionSize = crate::ConstU32<{ 8 * 1024 * 1024 }>;
	type AuthorizationPeriod = AuthorizationPeriod;
	type Authorizer = EitherOfDiverse<
		EitherOfDiverse<
			// Root can do whatever.
			crate::EnsureRoot<Self::AccountId>,
			// People chain can also handle authorizations.
			EnsureXcm<Equals<PeopleLocation>>,
		>,
		// Alice can also authorize for testing purposes.
		EnsureAlice,
	>;
	type StoreRenewPriority = StoreRenewPriority;
	type StoreRenewLongevity = StoreRenewLongevity;
	type RemoveExpiredAuthorizationPriority = RemoveExpiredAuthorizationPriority;
	type RemoveExpiredAuthorizationLongevity = RemoveExpiredAuthorizationLongevity;
}
