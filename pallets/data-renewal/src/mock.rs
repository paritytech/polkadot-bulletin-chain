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

//! Test environment for the data-renewal pallet. Wires both
//! `pallet-bulletin-transaction-storage` and `pallet-bulletin-data-renewal` into a
//! single mock runtime so the cross-pallet flow (storage → trait callback → renewal
//! drain) is exercised end-to-end.

use crate as pallet_bulletin_data_renewal;
use bulletin_pallets_common::NoCurrency;
use pallet_bulletin_transaction_storage::{
	AsAuthorizer, EnsureAllowedAuthorizers, DEFAULT_MAX_BLOCK_TRANSACTIONS,
	DEFAULT_MAX_TRANSACTION_SIZE,
};
use polkadot_sdk_frame::{
	deps::{frame_support, frame_system},
	prelude::*,
	runtime::prelude::*,
	testing_prelude::*,
	traits::EitherOf,
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
	pub type DataRenewal = pallet_bulletin_data_renewal;
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
	type Authorizer = EitherOf<
		AsAuthorizer<EnsureRoot<Self::AccountId>, Self::AccountId, BlockNumberFor<Self>>,
		EnsureAllowedAuthorizers<Self>,
	>;
	type StoreRenewPriority = StoreRenewPriority;
	type StoreRenewLongevity = StoreRenewLongevity;
	type RemoveExpiredAuthorizationPriority = RemoveExpiredAuthorizationPriority;
	type RemoveExpiredAuthorizationLongevity = RemoveExpiredAuthorizationLongevity;
	type OnObsoleteTransactions = DataRenewal;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper =
		pallet_bulletin_transaction_storage::benchmarking::DefaultCheckProofHelper;
}

impl pallet_bulletin_data_renewal::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = ();
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

/// Run to block `n`, calling `apply_block_inherents` (proof inherent) and the
/// renewal-pallet drain inherent before each `on_finalize`. Mirrors the runtime's
/// block-author behaviour so the per-block `ProofChecked` and `PendingAutoRenewals`
/// invariants are satisfied.
#[allow(dead_code)]
pub fn run_to_block(
	n: u64,
	f: impl Fn() -> Option<sp_transaction_storage_proof::TransactionStorageProof> + 'static,
) {
	System::run_to_block_with::<AllPalletsWithSystem>(
		n,
		RunToBlockHooks::default().before_finalize(move |_| {
			let proof = f();
			TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), proof).unwrap();
			if !crate::PendingAutoRenewals::<Test>::get().is_empty() {
				DataRenewal::process_pending_renewals(RuntimeOrigin::none()).unwrap();
			}
		}),
	);
}

/// Apply both block-level inherents in the order a runtime would: proof check
/// (storage pallet) then renewal drain (renewal pallet). Mirrors what
/// `run_to_block`'s `before_finalize` does for tests that drove block setup
/// manually with [`init_block`].
#[allow(dead_code)]
pub fn apply_block_inherents_full(
	proof: Option<sp_transaction_storage_proof::TransactionStorageProof>,
) -> polkadot_sdk_frame::deps::sp_runtime::DispatchResult {
	TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), proof)
		.map(|_| ())
		.map_err(|e| e.error)?;
	if !crate::PendingAutoRenewals::<Test>::get().is_empty() {
		DataRenewal::process_pending_renewals(RuntimeOrigin::none())
			.map(|_| ())
			.map_err(|e| e.error)?;
	}
	Ok(())
}

/// Initialize block `n` for tests that need an extrinsic context to call
/// dispatchables manually (without the runtime's authorize/prepare pipeline).
#[allow(dead_code)]
pub fn init_block(n: u64) {
	System::set_block_number(n);
	System::reset_events();
	// Set extrinsic index so `sp_io::transaction_index::renew` works inside the
	// renewal dispatchables.
	frame_system::Pallet::<Test>::set_extrinsic_index(0);
	let weight = TransactionStorage::on_initialize(n);
	frame_system::Pallet::<Test>::register_extra_weight_unchecked(
		weight,
		polkadot_sdk_frame::deps::frame_support::dispatch::DispatchClass::Mandatory,
	);
}

/// Mirrors `pallet_bulletin_transaction_storage`'s removed test helper. Runs
/// `pre_dispatch_renewal_signed` (charges bytes_permanent + tx slot at the
/// extension level), then dispatches `DataRenewal::enable_auto_renew` with the
/// `Origin::Authorized` the extension would have set.
#[allow(dead_code)]
pub fn enable_auto_renew_via_extension(
	who: <Test as frame_system::Config>::AccountId,
	content_hash: bulletin_transaction_storage_primitives::ContentHash,
) -> polkadot_sdk_frame::deps::sp_runtime::DispatchResult {
	let call = crate::Call::<Test>::enable_auto_renew { content_hash };
	DataRenewal::pre_dispatch_renewal_signed(&who, &call)
		.expect("pre_dispatch_renewal_signed must succeed for the via-extension test helper");
	let origin: RuntimeOrigin = pallet_bulletin_transaction_storage::Origin::<Test>::Authorized {
		who,
		scope: pallet_bulletin_transaction_storage::AuthorizationScope::Account(who),
	}
	.into();
	DataRenewal::enable_auto_renew(origin, content_hash)
}

/// Sibling helper for `disable_auto_renew`. Builds the rewritten origin directly
/// (skips `pre_dispatch_renewal_signed`) since most disable tests exercise
/// dispatch-level errors after admission.
#[allow(dead_code)]
pub fn disable_auto_renew_via_extension(
	who: <Test as frame_system::Config>::AccountId,
	content_hash: bulletin_transaction_storage_primitives::ContentHash,
) -> polkadot_sdk_frame::deps::sp_runtime::DispatchResult {
	let origin: RuntimeOrigin = pallet_bulletin_transaction_storage::Origin::<Test>::Authorized {
		who,
		scope: pallet_bulletin_transaction_storage::AuthorizationScope::Account(who),
	}
	.into();
	DataRenewal::disable_auto_renew(origin, content_hash)
}

/// Sibling helper for one-shot `renew`. Runs `pre_dispatch_renewal_signed`
/// (charges the slot) and dispatches with the rewritten origin.
#[allow(dead_code)]
pub fn renew_via_extension(
	who: <Test as frame_system::Config>::AccountId,
	entry: pallet_bulletin_transaction_storage::TransactionRef<BlockNumberFor<Test>>,
) -> polkadot_sdk_frame::deps::sp_runtime::DispatchResult {
	let call = crate::Call::<Test>::renew { entry: entry.clone() };
	DataRenewal::pre_dispatch_renewal_signed(&who, &call)
		.expect("pre_dispatch_renewal_signed must succeed for the via-extension test helper");
	let origin: RuntimeOrigin = pallet_bulletin_transaction_storage::Origin::<Test>::Authorized {
		who,
		scope: pallet_bulletin_transaction_storage::AuthorizationScope::Account(who),
	}
	.into();
	DataRenewal::renew(origin, entry)
}
