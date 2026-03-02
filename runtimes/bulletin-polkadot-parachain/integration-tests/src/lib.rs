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

//! XCM integration tests for Bulletin Polkadot parachain.
//!
//! This crate contains integration tests demonstrating DOT reserve transfers
//! between Asset Hub and the Bulletin chain using the xcm-emulator framework.
//!
//! NOTE: The Polkadot SDK does not provide Polkadot mainnet emulated chains,
//! so we use Westend emulated chains as a stand-in. This works because:
//! - Asset Hub Westend uses parachain ID 1000, matching the Polkadot parachain
//!   runtime's `AssetHubLocation` configuration.
//! - The xcm-emulator routes messages by topology, not by network ID.
//! - The reserve transfer logic (`IsRelayTokenFrom<AssetHubLocation>`) checks
//!   origin and asset location, both of which are network-ID-agnostic.

#[cfg(test)]
mod tests;

use asset_hub_westend_emulated_chain::AssetHubWestend;
use bulletin_polkadot_parachain_runtime::SessionKeys;
use cumulus_primitives_core::ParaId;
use emulated_integration_tests_common::{
	accounts::{self, ALICE, BOB},
	impl_accounts_helpers_for_parachain, impl_assert_events_helpers_for_parachain,
	impl_xcm_helpers_for_parachain,
	xcm_emulator::decl_test_parachains,
	AuraDigestProvider,
};
use frame_support::traits::OnInitialize;
use parachains_common::{AuraId, Balance};
use sp_core::sr25519;
use sp_keyring::Sr25519Keyring;
use westend_emulated_chain::Westend;
use xcm_emulator::{
	decl_test_networks, decl_test_sender_receiver_accounts_parameter_types, Parachain,
};

/// Bulletin Polkadot parachain ID.
pub const BULLETIN_PARA_ID: u32 = 1006;

/// Initial balance for test accounts.
pub const INITIAL_BALANCE: Balance = 100_000_000_000_000; // 100 DOT

decl_test_parachains! {
	pub struct BulletinPolkadotParachain {
		genesis = bulletin_genesis(),
		on_init = {
			// Initialize Aura pallet - must be done on first block
			bulletin_polkadot_parachain_runtime::AuraExt::on_initialize(1);
		},
		runtime = bulletin_polkadot_parachain_runtime,
		core = {
			XcmpMessageHandler: bulletin_polkadot_parachain_runtime::XcmpQueue,
			LocationToAccountId: bulletin_polkadot_parachain_runtime::xcm_config::LocationToAccountId,
			ParachainInfo: bulletin_polkadot_parachain_runtime::ParachainInfo,
			MessageOrigin: cumulus_primitives_core::AggregateMessageOrigin,
			DigestProvider: AuraDigestProvider,
			AdditionalInherentCode: (),
		},
		pallets = {
			PolkadotXcm: bulletin_polkadot_parachain_runtime::PolkadotXcm,
			Balances: bulletin_polkadot_parachain_runtime::Balances,
		}
	}
}

decl_test_networks! {
	pub struct MockNet {
		relay_chain = Westend,
		parachains = vec![
			AssetHubWestend,
			BulletinPolkadotParachain,
		],
		bridge = ()
	}
}

impl_accounts_helpers_for_parachain!(BulletinPolkadotParachain);
impl_assert_events_helpers_for_parachain!(BulletinPolkadotParachain);
impl_xcm_helpers_for_parachain!(BulletinPolkadotParachain);

decl_test_sender_receiver_accounts_parameter_types! {
	BulletinPolkadotParachainPara { sender: ALICE, receiver: BOB },
	AssetHubWestendPara { sender: ALICE, receiver: BOB }
}

/// Genesis configuration for Bulletin Polkadot parachain.
pub fn bulletin_genesis() -> sp_runtime::Storage {
	use bulletin_polkadot_parachain_runtime::RuntimeGenesisConfig;
	use sp_runtime::BuildStorage;

	let genesis_config = RuntimeGenesisConfig {
		system: Default::default(),
		parachain_system: Default::default(),
		balances: bulletin_polkadot_parachain_runtime::BalancesConfig {
			balances: accounts::init_balances()
				.iter()
				.map(|account| (account.clone(), INITIAL_BALANCE))
				.collect(),
			dev_accounts: None,
		},
		parachain_info: bulletin_polkadot_parachain_runtime::ParachainInfoConfig {
			parachain_id: ParaId::from(BULLETIN_PARA_ID),
			..Default::default()
		},
		collator_selection: bulletin_polkadot_parachain_runtime::CollatorSelectionConfig {
			invulnerables: vec![],
			candidacy_bond: 0,
			desired_candidates: 0,
		},
		session: bulletin_polkadot_parachain_runtime::SessionConfig {
			keys: vec![(
				Sr25519Keyring::Alice.to_account_id(),
				Sr25519Keyring::Alice.to_account_id(),
				SessionKeys { aura: AuraId::from(sr25519::Public::from_raw([0u8; 32])) },
			)],
			..Default::default()
		},
		aura: bulletin_polkadot_parachain_runtime::AuraConfig {
			authorities: vec![AuraId::from(sr25519::Public::from_raw([0u8; 32]))],
		},
		aura_ext: Default::default(),
		polkadot_xcm: bulletin_polkadot_parachain_runtime::PolkadotXcmConfig {
			safe_xcm_version: Some(xcm::latest::VERSION),
			..Default::default()
		},
		sudo: Default::default(),
		transaction_payment: Default::default(),
		transaction_storage: Default::default(),
	};

	genesis_config.build_storage().unwrap()
}
