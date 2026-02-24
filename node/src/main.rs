//! Polkadot Bulletin Chain node.

#![warn(missing_docs)]

// Re-export SDK crates from umbrella for use in submodules
use polkadot_sdk::{
	frame_benchmarking_cli, frame_system_rpc_runtime_api, pallet_transaction_payment,
};

// Benchmarking imports (only used with runtime-benchmarks feature)
#[cfg(feature = "runtime-benchmarks")]
use polkadot_sdk::frame_benchmarking;
#[cfg(feature = "runtime-benchmarks")]
use polkadot_sdk::frame_support;

// Try-runtime imports
#[cfg(feature = "try-runtime")]
use polkadot_sdk::frame_try_runtime;
use polkadot_sdk::{
	pallet_transaction_payment_rpc_runtime_api, polkadot_primitives, sc_basic_authorship,
	sc_chain_spec, sc_cli, sc_client_api, sc_consensus, sc_consensus_babe, sc_consensus_babe_rpc,
	sc_consensus_grandpa, sc_consensus_grandpa_rpc, sc_executor, sc_network, sc_offchain, sc_rpc,
	sc_service, sc_sync_state_rpc, sc_telemetry, sc_transaction_pool, sc_transaction_pool_api,
	sp_api, sp_block_builder, sp_blockchain, sp_consensus, sp_consensus_babe, sp_consensus_grandpa,
	sp_core, sp_genesis_builder, sp_inherents, sp_io, sp_keystore, sp_offchain, sp_runtime,
	sp_session, sp_timestamp, sp_transaction_pool, sp_transaction_storage_proof, sp_version,
	sp_weights, substrate_frame_rpc_system,
};

mod chain_spec;
#[macro_use]
mod service;
mod benchmarking;
mod cli;
mod command;
mod fake_runtime_api;
mod node_primitives;
mod rpc;

#[allow(clippy::result_large_err)]
fn main() -> sc_cli::Result<()> {
	command::run()
}
