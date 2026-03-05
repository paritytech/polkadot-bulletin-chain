use sc_chain_spec::ChainSpecExtension;
use sc_service::ChainType;
use sp_runtime::{Deserialize, Serialize};

const PROTOCOL_ID: &str = "dot-bulletin";

/// Node `ChainSpec` extensions.
///
/// Additional parameters for some Substrate core modules,
/// customizable from the chain spec.
#[derive(Default, Clone, Serialize, Deserialize, ChainSpecExtension)]
#[serde(rename_all = "camelCase")]
pub struct Extensions {
	/// The light sync state.
	///
	/// This value will be set by the `sync-state rpc` implementation.
	pub light_sync_state: sc_sync_state_rpc::LightSyncStateExtension,
}

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec<Extensions>;

pub fn bulletin_polkadot_development_config() -> Result<ChainSpec, String> {
	Ok(ChainSpec::builder(
		bulletin_polkadot_runtime::WASM_BINARY
			.ok_or_else(|| "bulletin_polkadot_runtime::WASM_BINARY not available".to_string())?,
		Default::default(),
	)
	.with_name("Polkadot Bulletin Development")
	.with_id("dev")
	.with_chain_type(ChainType::Development)
	.with_protocol_id(PROTOCOL_ID)
	.with_genesis_config_preset_name(sp_genesis_builder::DEV_RUNTIME_PRESET)
	.build())
}

pub fn bulletin_polkadot_local_testnet_config() -> Result<ChainSpec, String> {
	Ok(ChainSpec::builder(
		bulletin_polkadot_runtime::WASM_BINARY
			.ok_or_else(|| "bulletin_polkadot_runtime::WASM_BINARY not available".to_string())?,
		Default::default(),
	)
	.with_name("Polkadot Bulletin Local Testnet")
	.with_id("local_testnet")
	.with_chain_type(ChainType::Local)
	.with_protocol_id(PROTOCOL_ID)
	.with_genesis_config_preset_name(sp_genesis_builder::LOCAL_TESTNET_RUNTIME_PRESET)
	.build())
}

/// Production/live Bulletin Polkadot chain configuration.
pub fn bulletin_polkadot_config() -> Result<ChainSpec, String> {
	ChainSpec::from_json_bytes(&include_bytes!("../chain-specs/bulletin-polkadot.json")[..])
}
