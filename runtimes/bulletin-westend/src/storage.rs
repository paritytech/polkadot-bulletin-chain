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

use super::{Runtime, RuntimeEvent};
use frame_support::{
	parameter_types,
	traits::{EitherOfDiverse, Equals},
};
use pallet_xcm::EnsureXcm;
use sp_runtime::transaction_validity::{TransactionLongevity, TransactionPriority};
use testnet_parachains_constants::westend::locations::PeopleLocation;

parameter_types! {
	// This currently _must_ be set to DEFAULT_STORAGE_PERIOD
	pub const StoragePeriod: crate::BlockNumber = sp_transaction_storage_proof::DEFAULT_STORAGE_PERIOD;
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
	type WeightInfo = crate::weights::pallet_transaction_storage::WeightInfo<Runtime>;
	type MaxBlockTransactions = crate::ConstU32<512>;
	/// Max transaction size per block needs to be aligned with `BlockLength`.
	type MaxTransactionSize = crate::ConstU32<{ 8 * 1024 * 1024 }>;
	type StoragePeriod = StoragePeriod;
	type AuthorizationPeriod = AuthorizationPeriod;
	type Authorizer = EitherOfDiverse<
		// Root can do whatever.
		crate::EnsureRoot<Self::AccountId>,
		// People chain can also handle authorizations.
		EnsureXcm<Equals<PeopleLocation>>,
	>;
	type StoreRenewPriority = StoreRenewPriority;
	type StoreRenewLongevity = StoreRenewLongevity;
	type RemoveExpiredAuthorizationPriority = RemoveExpiredAuthorizationPriority;
	type RemoveExpiredAuthorizationLongevity = RemoveExpiredAuthorizationLongevity;
}
