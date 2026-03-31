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
use frame_support::{
	parameter_types,
	traits::{Contains, EitherOfDiverse, SortedMembers},
};
use frame_system::EnsureSignedBy;
use pallet_transaction_storage::{CallInspector, DEFAULT_MAX_TRANSACTION_SIZE};
use pallet_xcm::EnsureXcm;
use pallets_common::{inspect_utility_wrapper, NoCurrency};
use sp_keyring::Sr25519Keyring;
use sp_runtime::transaction_validity::{TransactionLongevity, TransactionPriority};
/// Provides test accounts for use with `EnsureSignedBy`.
pub struct TestAccounts;
impl SortedMembers<AccountId> for TestAccounts {
	fn sorted_members() -> Vec<AccountId> {
		alloc::vec![Sr25519Keyring::Alice.to_account_id()]
	}
}

parameter_types! {
	pub const AuthorizationPeriod: crate::BlockNumber = 90 * crate::DAYS;
	// Priorities and longevities used by the transaction storage pallet extrinsics.
	pub const SudoPriority: TransactionPriority = TransactionPriority::MAX;
	pub const SetPurgeKeysPriority: TransactionPriority = SudoPriority::get() - 1;
	pub const RemoveExpiredAuthorizationPriority: TransactionPriority = SetPurgeKeysPriority::get() - 1;
	pub const RemoveExpiredAuthorizationLongevity: TransactionLongevity = crate::DAYS as TransactionLongevity;
	pub const StoreRenewPriority: TransactionPriority = RemoveExpiredAuthorizationPriority::get() - 1;
	pub const StoreRenewLongevity: TransactionLongevity = crate::DAYS as TransactionLongevity;
}

/// Tells [`pallet_transaction_storage::extension::ValidateStorageCalls`] how to find storage
/// calls inside wrapper extrinsics so it can recursively validate and consume authorization.
///
/// Also implements [`Contains<RuntimeCall>`] returning `true` for storage-mutating calls
/// (store, store_with_cid_config, renew). Used with `EverythingBut` as the XCM
/// `SafeCallFilter` to block these calls from XCM dispatch — they require on-chain
/// authorization that XCM cannot provide.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct StorageCallInspector;

impl pallet_transaction_storage::CallInspector<Runtime> for StorageCallInspector {
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
impl pallet_transaction_storage::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type Currency = NoCurrency<Self::AccountId, RuntimeHoldReason>;
	type RuntimeHoldReason = RuntimeHoldReason;
	type FeeDestination = ();
	type WeightInfo = crate::weights::pallet_transaction_storage::WeightInfo<Runtime>;
	type MaxBlockTransactions = crate::ConstU32<512>;
	/// Max transaction size per block needs to be aligned with `BlockLength`.
	type MaxTransactionSize = crate::ConstU32<{ DEFAULT_MAX_TRANSACTION_SIZE }>;
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
	type BenchmarkHelper = CheckProofHelper;
}

/// Pre-computed storage proof for benchmarking `check_proof`.
///
/// Generated with `verify_benchmark_proof` test for 512 transactions of 2MB each
/// with `[0u8; 32]` as randomness. Must be regenerated when `MaxTransactionSize` or
/// `MaxBlockTransactions` change.
#[cfg(feature = "runtime-benchmarks")]
pub struct CheckProofHelper;

pub const CHECK_PROOF: &str = "\
	0104000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000ccd0780ffff0080\
	f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825\
	c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83\
	a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a373\
	3464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b\
	5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d305\
	5c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e31\
	3ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771\
	032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc\
	9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0\
	f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464\
	780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2\
	e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04\
	a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad9\
	ad03803333008041038b346937eae08686bc2166a94e8ebcad3aac044655f5e016556efab645\
	178010fd81bc1359802f0b871aeb95e4410a8ec92b93af10ea767a2027cf4734e8de8041038b\
	346937eae08686bc2166a94e8ebcad3aac044655f5e016556efab645178010fd81bc1359802f\
	0b871aeb95e4410a8ec92b93af10ea767a2027cf4734e8de8041038b346937eae08686bc2166\
	a94e8ebcad3aac044655f5e016556efab645178010fd81bc1359802f0b871aeb95e4410a8ec9\
	2b93af10ea767a2027cf4734e8de8041038b346937eae08686bc2166a94e8ebcad3aac044655\
	f5e016556efab64517084000\
";

#[cfg(feature = "runtime-benchmarks")]
impl pallet_transaction_storage::BenchmarkHelper for CheckProofHelper {
	fn check_proof_encoded() -> Vec<u8> {
		array_bytes::hex2bytes_unchecked(CHECK_PROOF)
	}
}
