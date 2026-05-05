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

use super::{
	xcm_config::IsSiblingParachain, AccountId, Runtime, RuntimeCall, RuntimeEvent,
	RuntimeHoldReason,
};
use alloc::vec::Vec;
use bulletin_pallets_common::{inspect_utility_wrapper, NoCurrency};
use frame_support::{
	parameter_types,
	traits::{ConstU64, Contains, EitherOfDiverse, SortedMembers},
};
use frame_system::EnsureSignedBy;
use pallet_bulletin_transaction_storage::{
	CallInspector, DEFAULT_MAX_BLOCK_TRANSACTIONS, DEFAULT_MAX_TRANSACTION_SIZE,
};
use pallet_xcm::EnsureXcm;
use sp_keyring::Sr25519Keyring;
use sp_runtime::transaction_validity::{TransactionLongevity, TransactionPriority};

/// Cap on the total bytes committed to permanent storage (via `renew`) across all
/// authorizations on this chain. We decided to go with 1.7 TiB.
pub const MAX_PERMANENT_STORAGE_SIZE: u64 = 17 * 1024 * 1024 * 1024 * 1024 / 10;

/// Provides test accounts for use with `EnsureSignedBy`.
pub struct TestAccounts;
impl SortedMembers<AccountId> for TestAccounts {
	fn sorted_members() -> Vec<AccountId> {
		let mut members = alloc::vec![
			Sr25519Keyring::Alice.to_account_id(),
			// 5GBhBA9H49M24LaZXaQopm3MzHtBT9i4mbQZbMSn5FcJNRb9
			AccountId::new([
				0xb6, 0x45, 0x5b, 0xc5, 0x38, 0x36, 0x5d, 0x32, 0xd3, 0x29, 0x67, 0xb6, 0xf2, 0x1a,
				0x0c, 0x9b, 0x07, 0x15, 0x65, 0xe8, 0x78, 0xfe, 0x98, 0x5f, 0x88, 0xd1, 0x54, 0x3c,
				0xb1, 0x99, 0x1a, 0x7d,
			]),
		];
		members.sort();
		members
	}
}

parameter_types! {
	pub const AuthorizationPeriod: crate::BlockNumber = 14 * crate::DAYS;
	// Priorities and longevities used by the transaction storage pallet extrinsics.
	//
	// `RemoveExpiredAuthorization` (permissionless cleanup) sits at the top so it always
	// runs before stores compete for blockspace.
	pub const RemoveExpiredAuthorizationPriority: TransactionPriority = TransactionPriority::MAX;
	pub const RemoveExpiredAuthorizationLongevity: TransactionLongevity = crate::DAYS as TransactionLongevity;
	// Base priority for `store` / `renew`. Picked well below `TransactionPriority::MAX` so
	// `AllowanceBasedPriority` can add its boost without saturating `u64`, while still
	// leaving plenty of headroom above generic transactions.
	pub const StoreRenewPriority: TransactionPriority = TransactionPriority::MAX / 4;
	pub const StoreRenewLongevity: TransactionLongevity = crate::DAYS as TransactionLongevity;
}

/// Tells [`pallet_bulletin_transaction_storage::extension::ValidateStorageCalls`] how to find
/// storage calls inside wrapper extrinsics so it can recursively validate and consume
/// authorization.
///
/// Also implements [`Contains<RuntimeCall>`] returning `true` for storage-mutating calls
/// (store, store_with_cid_config, renew). Used with `EverythingBut` as the XCM
/// `SafeCallFilter` to block these calls from XCM dispatch — they require on-chain
/// authorization that XCM cannot provide.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct StorageCallInspector;

impl pallet_bulletin_transaction_storage::CallInspector<Runtime> for StorageCallInspector {
	fn inspect_wrapper(call: &RuntimeCall) -> Option<Vec<&RuntimeCall>> {
		match call {
			RuntimeCall::Utility(c) => inspect_utility_wrapper(c),
			// Sudo is intentionally not inspected: the sudo key holder can store
			// data via `sudo(store)` without authorization, as Root origin is
			// accepted by `ensure_authorized`.
			_ => None,
		}
	}
}

/// Returns `true` for storage-mutating TransactionStorage calls (store, store_with_cid_config,
/// renew). Recursively inspects wrapper calls (Utility) to prevent bypass via nesting.
/// Used with `EverythingBut` as the XCM `SafeCallFilter`.
impl Contains<RuntimeCall> for StorageCallInspector {
	fn contains(call: &RuntimeCall) -> bool {
		Self::is_storage_mutating_call(call, 0)
	}
}

/// The main business of the Bulletin chain.
impl pallet_bulletin_transaction_storage::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type Currency = NoCurrency<Self::AccountId, RuntimeHoldReason>;
	type RuntimeHoldReason = RuntimeHoldReason;
	type FeeDestination = ();
	type WeightInfo = crate::weights::pallet_bulletin_transaction_storage::WeightInfo<Runtime>;
	type MaxBlockTransactions = crate::ConstU32<{ DEFAULT_MAX_BLOCK_TRANSACTIONS }>;
	/// Max transaction size per block needs to be aligned with `BlockLength`.
	type MaxTransactionSize = crate::ConstU32<{ DEFAULT_MAX_TRANSACTION_SIZE }>;
	type MaxPermanentStorageSize = ConstU64<{ MAX_PERMANENT_STORAGE_SIZE }>;
	type AuthorizationPeriod = AuthorizationPeriod;
	type Authorizer = EitherOfDiverse<
		EitherOfDiverse<
			// Root can do whatever.
			crate::EnsureRoot<Self::AccountId>,
			// Any sibling parachain can handle authorizations.
			EnsureXcm<IsSiblingParachain>,
		>,
		// Test accounts can also authorize for testing purposes.
		EnsureSignedBy<TestAccounts, Self::AccountId>,
	>;
	type StoreRenewPriority = StoreRenewPriority;
	type StoreRenewLongevity = StoreRenewLongevity;
	type RemoveExpiredAuthorizationPriority = RemoveExpiredAuthorizationPriority;
	type RemoveExpiredAuthorizationLongevity = RemoveExpiredAuthorizationLongevity;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper =
		pallet_bulletin_transaction_storage::benchmarking::DefaultCheckProofHelper;
}

parameter_types! {
	/// Maximum allowable skew between the user's submit timestamp and the on-chain
	/// time when validating a HOP promotion: 48 hours, in milliseconds.
	pub const SubmitTimestampTolerance: u64 = 48 * 60 * 60 * 1000;
}

impl pallet_hop_promotion::Config for Runtime {
	type SubmitTimestampTolerance = SubmitTimestampTolerance;
	type WeightInfo = crate::weights::pallet_hop_promotion::WeightInfo<Runtime>;
}
