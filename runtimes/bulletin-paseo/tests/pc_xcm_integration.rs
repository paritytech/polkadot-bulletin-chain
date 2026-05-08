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

//! People Chain → Bulletin Chain `authorize_account` integration tests for the
//! Paseo runtime. The conformance suite lives in
//! `bulletin-runtimes-test-utils`; each scenario file under
//! `tests/pc_xcm_integration/` is a thin wrapper that supplies Paseo's runtime
//! types, the People Chain location, an externalities builder, and an
//! `advance_block` callback.
//!
//! The end-to-end scenario stays runtime-local because it constructs signed
//! extrinsics with the runtime's tx-extension stack.

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
	Runtime, RuntimeCall, RuntimeGenesisConfig, System, TransactionStorage,
};
use bulletin_runtimes_test_utils as utils;
use common::{advance_block, assert_extrinsic_ok, construct_and_apply_extrinsic};
use pallet_bulletin_transaction_storage::{AuthorizationExtent, Call as TxStorageCall};
use parachains_common::AccountId;
use sp_keyring::Sr25519Keyring;
use sp_runtime::BuildStorage;
use xcm::latest::Location;

fn pc_location() -> Location {
	PeopleLocation::get()
}

fn new_test_ext() -> sp_io::TestExternalities {
	sp_io::TestExternalities::new(RuntimeGenesisConfig::default().build_storage().unwrap())
}
