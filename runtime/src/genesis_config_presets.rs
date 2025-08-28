use crate::{
	opaque::SessionKeys, AccountId, BabeConfig, RelayerSetConfig, RuntimeGenesisConfig,
	SessionConfig, Signature, SudoConfig, ValidatorSetConfig, BABE_GENESIS_EPOCH_CONFIG,
};

use crate::{
	bridge_config::XCM_LANE, BridgeRococoGrandpaConfig, BridgeRococoMessagesConfig,
	BridgeRococoParachainsConfig,
};

use scale_info::prelude::format;
use sp_consensus_babe::AuthorityId as BabeId;
use sp_consensus_grandpa::AuthorityId as GrandpaId;
use sp_core::{sr25519, Pair, Public};
use sp_genesis_builder::PresetId;
use sp_runtime::traits::{IdentifyAccount, Verify};
use sp_std::prelude::*;

type AccountPublic = <Signature as Verify>::Signer;

/// Generate a crypto pair from seed.
pub fn get_from_seed<TPublic: Public>(seed: &str) -> <TPublic::Pair as Pair>::Public {
	TPublic::Pair::from_string(&format!("//{}", seed), None)
		.expect("static values are valid; qed")
		.public()
}

/// Generate an account ID from seed.
pub fn get_account_id_from_seed<TPublic: Public>(seed: &str) -> AccountId
where
	AccountPublic: From<<TPublic::Pair as Pair>::Public>,
{
	AccountPublic::from(get_from_seed::<TPublic>(seed)).into_account()
}

/// Generate authority keys from a seed.
pub fn authority_keys_from_seed(seed: &str) -> (AccountId, BabeId, GrandpaId) {
	(
		get_account_id_from_seed::<sr25519::Public>(seed),
		get_from_seed::<BabeId>(seed),
		get_from_seed::<GrandpaId>(seed),
	)
}

fn session_keys(babe: BabeId, grandpa: GrandpaId) -> SessionKeys {
	SessionKeys { babe, grandpa }
}

/// Configure initial storage state for FRAME modules.
fn testnet_genesis(
	initial_authorities: Vec<(AccountId, BabeId, GrandpaId)>,
	bridges_pallet_owner: Option<AccountId>,
	root_key: AccountId,
) -> serde_json::Value {
	let config = RuntimeGenesisConfig {
		validator_set: ValidatorSetConfig {
			initial_validators: initial_authorities
				.iter()
				.map(|x| x.0.clone())
				.collect::<Vec<_>>()
				.try_into()
				.expect("Too many initial authorities"),
		},
		session: SessionConfig {
			keys: initial_authorities
				.iter()
				.map(|x| (x.0.clone(), x.0.clone(), session_keys(x.1.clone(), x.2.clone())))
				.collect(),
			non_authority_keys: Default::default(),
		},
		babe: BabeConfig { epoch_config: BABE_GENESIS_EPOCH_CONFIG, ..Default::default() },
		sudo: SudoConfig {
			// Assign network admin rights.
			key: Some(root_key.clone()),
		},
		relayer_set: RelayerSetConfig {
			// For simplicity just make the initial relayer set match the initial validator set. In
			// practice even if the same entities control the validators and the relayers they
			// would want to use separate keys for the relayers.
			initial_relayers: initial_authorities.into_iter().map(|x| x.0).collect::<Vec<_>>(),
		},
		bridge_rococo_grandpa: BridgeRococoGrandpaConfig {
			owner: bridges_pallet_owner.clone(),
			..Default::default()
		},
		bridge_rococo_parachains: BridgeRococoParachainsConfig {
			owner: bridges_pallet_owner.clone(),
			..Default::default()
		},
		bridge_rococo_messages: BridgeRococoMessagesConfig {
			owner: bridges_pallet_owner,
			opened_lanes: vec![XCM_LANE],
			..Default::default()
		},
		..Default::default()
	};

	serde_json::to_value(config).expect("Could not build genesis config.")
}

/// Provides the JSON representation of predefined genesis config for given `id`.
pub fn get_preset(id: &PresetId) -> Option<Vec<u8>> {
	let patch = match id.as_ref() {
		sp_genesis_builder::DEV_RUNTIME_PRESET => testnet_genesis(
			// Initial PoA authorities
			vec![authority_keys_from_seed("Alice")],
			// Bridges pallet owner
			Some(get_account_id_from_seed::<sr25519::Public>("Alice")),
			// Sudo account
			get_account_id_from_seed::<sr25519::Public>("Alice"),
		),
		sp_genesis_builder::LOCAL_TESTNET_RUNTIME_PRESET => testnet_genesis(
			// Initial PoA authorities
			vec![
				authority_keys_from_seed("Alice"),
				authority_keys_from_seed("Alice//stash"),
				authority_keys_from_seed("Bob"),
				authority_keys_from_seed("Bob//stash"),
			],
			// Bridges pallet owner
			Some(get_account_id_from_seed::<sr25519::Public>("Alice")),
			// Sudo account
			get_account_id_from_seed::<sr25519::Public>("Alice"),
		),
		_ => return None,
	};
	Some(
		serde_json::to_string(&patch)
			.expect("serialization to JSON is expected to work. qed.")
			.into_bytes(),
	)
}

/// List of supported presets.
pub fn preset_names() -> Vec<PresetId> {
	vec![
		PresetId::from(sp_genesis_builder::DEV_RUNTIME_PRESET),
		PresetId::from(sp_genesis_builder::LOCAL_TESTNET_RUNTIME_PRESET),
	]
}
