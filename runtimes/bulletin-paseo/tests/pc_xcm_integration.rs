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

//! People Chain → Bulletin Chain `authorize_account` integration tests.
//!
//! Drives `XcmExecutor::prepare_and_execute` directly with messages shaped the
//! way People Chain sends them (sibling parachain origin, `OriginKind::Xcm`,
//! unpaid execution + Transact). Exercises the full receive-side pipeline:
//! barrier, origin conversion, `SafeCallFilter`, `Authorizer = EnsureXcm<
//! IsSiblingParachain>`, and the pallet's `authorize_account` /
//! `refresh_account_authorization` semantics.
//!
//! Each scenario lives in its own submodule file under
//! `tests/pc_xcm_integration/`. This file holds the shared helpers.

#![cfg(test)]

mod common;

#[path = "pc_xcm_integration/authorize_semantics.rs"]
mod authorize_semantics;
#[path = "pc_xcm_integration/end_to_end.rs"]
mod end_to_end;
#[path = "pc_xcm_integration/origin_rejections.rs"]
mod origin_rejections;
#[path = "pc_xcm_integration/refresh.rs"]
mod refresh;
#[path = "pc_xcm_integration/safe_call_filter.rs"]
mod safe_call_filter;

use bulletin_paseo_runtime::{
	paseo_constants::locations::PeopleLocation,
	xcm_config::{LocationToAccountId, XcmConfig},
	Runtime, RuntimeCall, RuntimeGenesisConfig, RuntimeOrigin, System, TransactionStorage,
};
use common::{advance_block, assert_extrinsic_ok, construct_and_apply_extrinsic};
use frame_support::{assert_ok, traits::Get};
use pallet_bulletin_transaction_storage::{
	AuthorizationExtent, Call as TxStorageCall, Config as TxStorageConfig,
};
use parachains_common::{AccountId, BlockNumber};
use sp_core::Encode;
use sp_keyring::Sr25519Keyring;
use sp_runtime::BuildStorage;
use xcm::latest::{prelude::*, InstructionError};
use xcm_executor::traits::ConvertLocation;

/// People Chain location on Paseo. Matches `paseo_constants::PeopleLocation`.
fn pc_location() -> Location {
	PeopleLocation::get()
}

fn auth_period() -> BlockNumber {
	<<Runtime as TxStorageConfig>::AuthorizationPeriod as Get<BlockNumber>>::get()
}

fn empty() -> AuthorizationExtent {
	AuthorizationExtent::default()
}

fn extent(
	bytes: u64,
	bytes_allowance: u64,
	transactions: u32,
	transactions_allowance: u32,
) -> AuthorizationExtent {
	AuthorizationExtent {
		bytes,
		bytes_permanent: 0,
		bytes_allowance,
		transactions,
		transactions_allowance,
	}
}

fn extent_of(who: &AccountId) -> AuthorizationExtent {
	TransactionStorage::account_authorization_extent(who.clone())
}

/// Build an XCM message in the shape PC uses: free unpaid execution + Transact.
fn xcm_transact(call: RuntimeCall, kind: OriginKind) -> Xcm<RuntimeCall> {
	Xcm::builder_unsafe()
		.unpaid_execution(Unlimited, None)
		.transact(kind, None, call.encode())
		.build()
}

fn execute_from(origin: Location, message: Xcm<RuntimeCall>) -> Result<(), InstructionError> {
	let mut id = [0u8; 32];
	xcm_executor::XcmExecutor::<XcmConfig>::prepare_and_execute(
		origin,
		message,
		&mut id,
		Weight::MAX,
		Weight::MAX,
	)
	.ensure_complete()
}

fn pc_authorize(who: AccountId, transactions: u32, bytes: u64) -> Result<(), InstructionError> {
	let call = RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
		who,
		transactions,
		bytes,
	});
	execute_from(pc_location(), xcm_transact(call, OriginKind::Xcm))
}

fn pc_refresh(who: AccountId) -> Result<(), InstructionError> {
	let call =
		RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::refresh_account_authorization {
			who,
		});
	execute_from(pc_location(), xcm_transact(call, OriginKind::Xcm))
}

fn new_test_ext() -> sp_io::TestExternalities {
	let mut ext =
		sp_io::TestExternalities::new(RuntimeGenesisConfig::default().build_storage().unwrap());
	ext.execute_with(advance_block);
	ext
}
