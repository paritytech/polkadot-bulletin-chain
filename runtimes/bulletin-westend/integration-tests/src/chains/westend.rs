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

//! Emulated Westend relay chain definition.
//!
//! Based on upstream `westend-emulated-chain` from polkadot-sdk, simplified for
//! Bulletin chain integration tests.

use emulated_integration_tests_common::{
	accounts, build_genesis_storage, get_host_config, impl_accounts_helpers_for_relay_chain,
	impl_assert_events_helpers_for_relay_chain, impl_hrmp_channels_helpers_for_relay_chain,
	impl_send_transact_helpers_for_relay_chain, validators, xcm_emulator::decl_test_relay_chains,
};
use parachains_common::Balance;
use polkadot_primitives::{AssignmentId, ValidatorId};
use sc_consensus_grandpa::AuthorityId as GrandpaId;
use sp_authority_discovery::AuthorityId as AuthorityDiscoveryId;
use sp_consensus_babe::AuthorityId as BabeId;
use sp_consensus_beefy::ecdsa_crypto::AuthorityId as BeefyId;
use sp_core::storage::Storage;
use westend_runtime_constants::currency::UNITS as WND;

const ENDOWMENT: Balance = 1_000_000 * WND;

fn session_keys(
	babe: BabeId,
	grandpa: GrandpaId,
	para_validator: ValidatorId,
	para_assignment: AssignmentId,
	authority_discovery: AuthorityDiscoveryId,
	beefy: BeefyId,
) -> westend_runtime::SessionKeys {
	westend_runtime::SessionKeys {
		babe,
		grandpa,
		para_validator,
		para_assignment,
		authority_discovery,
		beefy,
	}
}

fn genesis() -> Storage {
	let genesis_config = westend_runtime::RuntimeGenesisConfig {
		system: westend_runtime::SystemConfig::default(),
		balances: westend_runtime::BalancesConfig {
			balances: accounts::init_balances().iter().cloned().map(|k| (k, ENDOWMENT)).collect(),
			..Default::default()
		},
		session: westend_runtime::SessionConfig {
			keys: validators::initial_authorities()
				.iter()
				.map(|x| {
					(
						x.0.clone(),
						x.0.clone(),
						session_keys(
							x.2.clone(),
							x.3.clone(),
							x.4.clone(),
							x.5.clone(),
							x.6.clone(),
							x.7.clone(),
						),
					)
				})
				.collect::<Vec<_>>(),
			..Default::default()
		},
		babe: westend_runtime::BabeConfig {
			authorities: Default::default(),
			epoch_config: westend_runtime::BABE_GENESIS_EPOCH_CONFIG,
			..Default::default()
		},
		configuration: westend_runtime::ConfigurationConfig { config: get_host_config() },
		..Default::default()
	};

	build_genesis_storage(&genesis_config, westend_runtime::WASM_BINARY.unwrap())
}

// Westend relay chain declaration
decl_test_relay_chains! {
	#[api_version(16)]
	pub struct Westend {
		genesis = genesis(),
		on_init = (),
		runtime = westend_runtime,
		core = {
			SovereignAccountOf: westend_runtime::xcm_config::LocationConverter,
		},
		pallets = {
			XcmPallet: westend_runtime::XcmPallet,
			Sudo: westend_runtime::Sudo,
			Balances: westend_runtime::Balances,
			Treasury: westend_runtime::Treasury,
			AssetRate: westend_runtime::AssetRate,
			Hrmp: westend_runtime::Hrmp,
			Identity: westend_runtime::Identity,
			IdentityMigrator: westend_runtime::IdentityMigrator,
		}
	},
}

impl_accounts_helpers_for_relay_chain!(Westend);
impl_assert_events_helpers_for_relay_chain!(Westend);
impl_hrmp_channels_helpers_for_relay_chain!(Westend);
impl_send_transact_helpers_for_relay_chain!(Westend);
