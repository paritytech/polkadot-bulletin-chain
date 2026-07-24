// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

//! Genesis config preset validation.

#![cfg(test)]

use bulletin_westend_runtime::{
	genesis_config_presets::{get_preset, preset_names, BULLETIN_PARA_ID},
	Balances, ParachainInfo, Runtime, RuntimeGenesisConfig, TransactionStorage,
};
use pallet_bulletin_transaction_storage::{
	AllowedAuthorizers, AuthorizationExtent, AuthorizerBudget, Quota,
};
use parachains_common::AccountId;
use sp_genesis_builder::{PresetId, DEV_RUNTIME_PRESET, LOCAL_TESTNET_RUNTIME_PRESET};
use sp_keyring::Sr25519Keyring;
use sp_runtime::BuildStorage;
use testnet_parachains_constants::westend::currency::UNITS as WND;

/// Merge `patch` into `base`, as chain-spec tooling does when applying a preset to the
/// default genesis config.
fn json_merge(base: &mut serde_json::Value, patch: serde_json::Value) {
	match (base, patch) {
		(serde_json::Value::Object(base), serde_json::Value::Object(patch)) =>
			for (k, v) in patch {
				json_merge(base.entry(k).or_insert(serde_json::Value::Null), v);
			},
		(base, patch) => *base = patch,
	}
}

/// Build externalities from the given preset, verifying it exists, is valid JSON and
/// produces a buildable `RuntimeGenesisConfig`.
fn build_preset(id: &str) -> sp_io::TestExternalities {
	let patch = get_preset(&PresetId::from(id)).unwrap_or_else(|| panic!("{id}: missing preset"));
	let patch: serde_json::Value =
		serde_json::from_slice(&patch).unwrap_or_else(|e| panic!("{id}: invalid JSON: {e}"));
	let mut config = serde_json::to_value(RuntimeGenesisConfig::default()).unwrap();
	json_merge(&mut config, patch);
	let config: RuntimeGenesisConfig = serde_json::from_value(config)
		.unwrap_or_else(|e| panic!("{id}: invalid genesis config: {e}"));
	sp_io::TestExternalities::new(
		config
			.build_storage()
			.unwrap_or_else(|e| panic!("{id}: build_storage failed: {e}")),
	)
}

/// Genesis state shared by all presets.
fn assert_common_genesis_state() {
	// Sudo key is Alice.
	assert_eq!(pallet_sudo::Key::<Runtime>::get(), Some(Sr25519Keyring::Alice.to_account_id()));

	// Eve is the only genesis authorizer: feeless, never expiring, 100k txs / 100 GiB budget.
	assert_eq!(
		AllowedAuthorizers::<Runtime>::iter().collect::<Vec<_>>(),
		vec![(
			Sr25519Keyring::Eve.to_account_id(),
			AuthorizerBudget {
				quota: Some(Quota { transactions: 100_000, bytes: 100 * 1024 * 1024 * 1024 }),
				valid_until: None,
				feeless: true,
			},
		)],
	);

	// Alice holds the genesis account authorization: 100 txs / 10 MiB, nothing consumed.
	assert_eq!(
		TransactionStorage::account_authorization_extent(Sr25519Keyring::Alice.to_account_id()),
		AuthorizationExtent {
			bytes: 0,
			bytes_permanent: 0,
			bytes_allowance: 10 * 1024 * 1024,
			transactions: 0,
			transactions_allowance: 100,
		},
	);

	assert_eq!(ParachainInfo::parachain_id(), BULLETIN_PARA_ID);
	assert_eq!(Balances::free_balance(Sr25519Keyring::Alice.to_account_id()), 1_000_000 * WND);
}

fn assert_collators(mut expected: Vec<AccountId>) {
	expected.sort();
	assert_eq!(pallet_collator_selection::Invulnerables::<Runtime>::get().into_inner(), expected);
	for collator in expected {
		assert!(pallet_session::NextKeys::<Runtime>::get(&collator).is_some());
	}
}

#[test]
fn preset_names_are_exactly_dev_and_local() {
	assert_eq!(
		preset_names(),
		vec![PresetId::from(DEV_RUNTIME_PRESET), PresetId::from(LOCAL_TESTNET_RUNTIME_PRESET)],
	);
}

#[test]
fn dev_preset_builds_expected_genesis_state() {
	build_preset(DEV_RUNTIME_PRESET).execute_with(|| {
		assert_common_genesis_state();
		assert_collators(vec![Sr25519Keyring::Alice.to_account_id()]);
	});
}

#[test]
fn local_testnet_preset_builds_expected_genesis_state() {
	build_preset(LOCAL_TESTNET_RUNTIME_PRESET).execute_with(|| {
		assert_common_genesis_state();
		assert_collators(vec![
			Sr25519Keyring::Alice.to_account_id(),
			Sr25519Keyring::Bob.to_account_id(),
		]);
	});
}

#[test]
fn unknown_preset_returns_none() {
	assert!(get_preset(&PresetId::from("unknown")).is_none());
}
