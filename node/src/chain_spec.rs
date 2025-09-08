use sc_service::ChainType;

const PROTOCOL_ID: &str = "dot-bulletin";

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec;

pub fn rococo_development_config() -> Result<ChainSpec, String> {
	Ok(ChainSpec::builder(
		polkadot_bulletin_chain_runtime::WASM_BINARY
			.ok_or_else(|| "Development wasm not available".to_string())?,
		None,
	)
	.with_name("Development")
	.with_id("dev")
	.with_chain_type(ChainType::Development)
	.with_protocol_id(PROTOCOL_ID)
	.with_genesis_config_preset_name(sp_genesis_builder::DEV_RUNTIME_PRESET)
	.build())
}

pub fn rococo_local_testnet_config() -> Result<ChainSpec, String> {
	Ok(ChainSpec::builder(
		polkadot_bulletin_chain_runtime::WASM_BINARY
			.ok_or_else(|| "Development wasm not available".to_string())?,
		None,
	)
	.with_name("Rococo Bulletin Local Testnet")
	.with_id("local_testnet")
	.with_chain_type(ChainType::Local)
	.with_protocol_id(PROTOCOL_ID)
	.with_genesis_config_preset_name(sp_genesis_builder::LOCAL_TESTNET_RUNTIME_PRESET)
	.build())
}

pub fn bulletin_polkadot_development_config() -> Result<ChainSpec, String> {
	Ok(ChainSpec::builder(
		bulletin_polkadot_runtime::WASM_BINARY
			.ok_or_else(|| "bulletin_polkadot_runtime::WASM_BINARY not available".to_string())?,
		None,
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
		None,
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
