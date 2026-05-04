pub mod cli_runner;
pub mod expectations;

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use zombienet_sdk::{LocalFileSystem, NetworkConfigBuilder};

// --- Parachain defaults ---

const DEFAULT_RELAY_BINARY: &str = "polkadot";
const DEFAULT_PARACHAIN_BINARY: &str = "polkadot-parachain";
const DEFAULT_PARACHAIN_CHAIN_SPEC: &str = "./zombienet/bulletin-westend-spec.json";
const DEFAULT_RELAY_CHAIN: &str = "westend-local";
const DEFAULT_PARA_ID: u32 = 1010;

fn env_or_default(var: &str, default: &str) -> String {
	std::env::var(var).unwrap_or_else(|_| default.to_string())
}

fn resolve_binary(path: &str) -> Result<PathBuf> {
	let p = PathBuf::from(path);
	if p.is_absolute() && p.exists() {
		return Ok(p);
	}
	// Try relative to crate dir
	if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
		let relative = PathBuf::from(&manifest_dir).join("..").join(path);
		if let Ok(resolved) = relative.canonicalize() {
			return Ok(resolved);
		}
	}
	// Try current dir
	if let Ok(cwd) = std::env::current_dir() {
		let full = cwd.join(path);
		if let Ok(resolved) = full.canonicalize() {
			return Ok(resolved);
		}
	}
	// Assume it's on PATH
	Ok(p)
}

// --- Multi-node parachain network ---
//
// Topology:
//   2 relay validators (alice, bob)
//          │                │
//    collator-1      collator-2     ← 2 collators, both author blocks
//          │ gossip        │ gossip
//       rpc-1           rpc-2       ← 2 full nodes (non-collating), receive txs
//          │ ws://         │ ws://
//             stress-test            ← splits submitters across RPC nodes
//
// This simulates a real network where transactions are submitted to RPC nodes
// and must propagate via gossip to collators. In the single-collator test,
// the stress-test submits directly to the collator which sees all txs instantly.

pub async fn spawn_parachain_network_multi_node() -> Result<zombienet_sdk::Network<LocalFileSystem>>
{
	let relay_binary = env_or_default("POLKADOT_RELAY_BINARY_PATH", DEFAULT_RELAY_BINARY);
	let relay_binary_path = resolve_binary(&relay_binary)?;
	let relay_binary_str = relay_binary_path.to_string_lossy().to_string();

	let para_binary = env_or_default("POLKADOT_PARACHAIN_BINARY_PATH", DEFAULT_PARACHAIN_BINARY);
	let para_binary_path = resolve_binary(&para_binary)?;
	let para_binary_str = para_binary_path.to_string_lossy().to_string();

	let chain_spec = env_or_default("PARACHAIN_CHAIN_SPEC_PATH", DEFAULT_PARACHAIN_CHAIN_SPEC);
	let chain_spec_path = resolve_binary(&chain_spec)?;
	let chain_spec_str = chain_spec_path.to_string_lossy().to_string();

	if !chain_spec_path.exists() {
		anyhow::bail!(
			"Chain spec not found at '{chain_spec_str}'. \
			 Run ./scripts/create_bulletin_westend_spec.sh first, \
			 or set PARACHAIN_CHAIN_SPEC_PATH.",
		);
	}

	let relay_chain = env_or_default("RELAY_CHAIN", DEFAULT_RELAY_CHAIN);
	let para_id: u32 = std::env::var("PARACHAIN_ID")
		.ok()
		.and_then(|v| v.parse().ok())
		.unwrap_or(DEFAULT_PARA_ID);

	tracing::info!("Multi-node network: relay={relay_binary_str}, para={para_binary_str}");
	tracing::info!("Chain spec: {chain_spec_str}, relay: {relay_chain}, para ID: {para_id}");

	let relay_args = vec!["-lruntime=debug".into()];
	let relay_args2 = vec!["-lruntime=debug".into()];

	let para_binary_str2 = para_binary_str.clone();
	let para_binary_str3 = para_binary_str.clone();
	let para_binary_str4 = para_binary_str.clone();

	let config = NetworkConfigBuilder::new()
		.with_relaychain(|rc| {
			rc.with_chain(relay_chain.as_str())
				.with_default_command(relay_binary_str.as_str())
				.with_node(|node| node.with_name("alice").validator(true).with_args(relay_args))
				.with_node(|node| node.with_name("bob").validator(true).with_args(relay_args2))
		})
		.with_parachain(|parachain| {
			parachain
				.with_id(para_id)
				.with_chain_spec_path(chain_spec_str.as_str())
				.cumulus_based(true)
				// Collator 1: authors blocks (slot-based for elastic scaling)
				.with_collator(|c| {
					c.with_name("collator-1")
						.validator(true)
						.with_command(para_binary_str.as_str())
						.with_args(vec![
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace"
								.into(),
							"--authoring=slot-based".into(),
							"--ipfs-server".into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),
							"--network-backend=libp2p".into(),
						])
				})
				// Collator 2: authors blocks (slot-based for elastic scaling)
				.with_collator(|c| {
					c.with_name("collator-2")
						.validator(true)
						.with_command(para_binary_str2.as_str())
						.with_args(vec![
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace"
								.into(),
							"--authoring=slot-based".into(),
							"--ipfs-server".into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),
							"--network-backend=libp2p".into(),
						])
				})
				// RPC 1: full node, syncs but does not collate
				.with_collator(|c| {
					c.with_name("rpc-1")
						.validator(false)
						.with_command(para_binary_str3.as_str())
						.with_args(vec![
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace"
								.into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),
							"--network-backend=libp2p".into(),
						])
				})
				// RPC 2: full node, syncs but does not collate
				.with_collator(|c| {
					c.with_name("rpc-2")
						.validator(false)
						.with_command(para_binary_str4.as_str())
						.with_args(vec![
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace"
								.into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),
							"--network-backend=libp2p".into(),
						])
				})
		})
		.build()
		.map_err(|errs| anyhow!("Network config errors: {errs:?}"))?;

	let spawn_fn = zombienet_sdk::environment::get_spawn_fn();
	let network = spawn_fn(config).await?;

	// Wait for relay chain session change
	let alice = network.get_node("alice")?;
	tracing::info!("Waiting for relay chain to reach block 20 (session change)...");
	alice
		.wait_metric_with_timeout("block_height{status=\"best\"}", |h| h >= 20.0, 300u64)
		.await?;
	tracing::info!("Relay chain session change should have occurred");

	// Wait for collator-1 to start producing blocks
	let collator1 = network.get_node("collator-1")?;
	tracing::info!("Waiting for collator-1 to start producing blocks...");
	collator1
		.wait_metric_with_timeout("block_height{status=\"best\"}", |h| h >= 2.0, 300u64)
		.await?;
	tracing::info!("Collator-1 is producing blocks");

	// Wait for RPC nodes to sync (they should follow collator blocks)
	let rpc1 = network.get_node("rpc-1")?;
	tracing::info!("Waiting for rpc-1 to sync...");
	rpc1.wait_metric_with_timeout("block_height{status=\"best\"}", |h| h >= 1.0, 300u64)
		.await?;
	tracing::info!("RPC-1 is synced");

	let rpc2 = network.get_node("rpc-2")?;
	tracing::info!("Waiting for rpc-2 to sync...");
	rpc2.wait_metric_with_timeout("block_height{status=\"best\"}", |h| h >= 1.0, 300u64)
		.await?;
	tracing::info!("RPC-2 is synced — multi-node network ready");

	Ok(network)
}

// --- Helpers ---

pub fn get_node_ws_url(
	network: &zombienet_sdk::Network<LocalFileSystem>,
	name: &str,
) -> Result<String> {
	let node = network.get_node(name)?;
	Ok(node.ws_uri().to_string())
}
