pub mod cli_runner;
pub mod expectations;

use anyhow::{anyhow, Result};
use std::path::PathBuf;
use subxt::{
	config::substrate::SubstrateConfig, dynamic::tx, ext::scale_value::Value, OnlineClient,
};
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

// --- Multi-node parachain network (elastic scaling: 3 cores, 2s blocks) ---
//
// Topology:
//   6 relay validators (alice, bob, charlie, dave, eve, ferdie)
//          │                │
//    collator-1      collator-2     ← 2 collators, slot-based authoring
//          │ gossip        │ gossip
//       rpc-1           rpc-2       ← 2 full nodes (non-collating), receive txs
//          │ ws://         │ ws://
//             stress-test            ← splits submitters across RPC nodes
//
// Relay chain genesis overrides:
//   - scheduler_params.num_cores = 3
//   - async_backing_params: allowed_ancestry_len = 6, max_candidate_depth = 6

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

	let relay_args: Vec<String> = vec!["-lruntime=debug".into()];

	let para_binary_str2 = para_binary_str.clone();
	let para_binary_str3 = para_binary_str.clone();
	let para_binary_str4 = para_binary_str.clone();

	// Relay chain genesis overrides for elastic scaling (3 cores).
	// num_cores: 2 extra cores + 1 auto-assigned per registered parachain = 3 total.
	// max_validators_per_core: 1 (need at least 3 validators).
	let relay_genesis_overrides = serde_json::json!({
		"configuration": {
			"config": {
				"scheduler_params": {
					"num_cores": 2,
					"max_validators_per_core": 1
				},
				"async_backing_params": {
					"max_candidate_depth": 12,
					"allowed_ancestry_len": 2
				}
			}
		}
	});

	let config = NetworkConfigBuilder::new()
		.with_relaychain(|rc| {
			rc.with_chain(relay_chain.as_str())
				.with_default_command(relay_binary_str.as_str())
				.with_default_args(relay_args.iter().map(|s| s.as_str().into()).collect())
				.with_genesis_overrides(relay_genesis_overrides)
				.with_node(|node| node.with_name("alice").validator(true))
				.with_node(|node| node.with_name("bob").validator(true))
				.with_node(|node| node.with_name("charlie").validator(true))
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
							"--authoring=slot-based".into(),
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace,txpool=trace"
								.into(),
							"--ipfs-server".into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),

						])
				})
				// Collator 2: authors blocks (slot-based for elastic scaling)
				.with_collator(|c| {
					c.with_name("collator-2")
						.validator(true)
						.with_command(para_binary_str2.as_str())
						.with_args(vec![
							"--authoring=slot-based".into(),
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace,txpool=trace"
								.into(),
							"--ipfs-server".into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),

						])
				})
				// RPC 1: full node, syncs but does not collate
				.with_collator(|c| {
					c.with_name("rpc-1")
						.validator(false)
						.with_command(para_binary_str3.as_str())
						.with_args(vec![
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace,txpool=trace"
								.into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),

						])
				})
				// RPC 2: full node, syncs but does not collate
				.with_collator(|c| {
					c.with_name("rpc-2")
						.validator(false)
						.with_command(para_binary_str4.as_str())
						.with_args(vec![
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace,txpool=trace"
								.into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),

						])
				})
		})
		.build()
		.map_err(|errs| anyhow!("Network config errors: {errs:?}"))?;

	let spawn_fn = zombienet_sdk::environment::get_spawn_fn();
	let network = spawn_fn(config).await?;

	// Assign extra cores as early as possible so they take effect by the first
	// session change. The para gets 1 core from genesis registration; we add 2
	// more for 3-core elastic scaling.
	let alice = network.get_node("alice")?;
	let relay_ws_url = alice.ws_uri().to_string();
	assign_cores(&relay_ws_url, para_id, 2).await?;

	// Wait for relay chain session change — core assignments and parachain
	// registration both activate at session boundaries.
	tracing::info!("Waiting for relay chain to reach block 20 (session change)...");
	alice
		.wait_metric_with_timeout("block_height{status=\"best\"}", |h| h >= 20.0, 300u64)
		.await?;
	tracing::info!("Session change occurred, elastic scaling should be active");

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
	tracing::info!("RPC-2 is synced — multi-node network ready (3 cores, slot-based authoring)");

	Ok(network)
}

// --- Single-core network (async backing, no elastic scaling) ---

pub async fn spawn_single_core_network() -> Result<zombienet_sdk::Network<LocalFileSystem>> {
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

	tracing::info!("Single-core network: relay={relay_binary_str}, para={para_binary_str}");
	tracing::info!("Chain spec: {chain_spec_str}, relay: {relay_chain}, para ID: {para_id}");

	let relay_genesis_overrides = serde_json::json!({
		"configuration": {
			"config": {
				"async_backing_params": {
					"max_candidate_depth": 6,
					"allowed_ancestry_len": 2
				}
			}
		}
	});

	let para_binary_str2 = para_binary_str.clone();
	let para_binary_str3 = para_binary_str.clone();
	let para_binary_str4 = para_binary_str.clone();

	let config = NetworkConfigBuilder::new()
		.with_relaychain(|rc| {
			rc.with_chain(relay_chain.as_str())
				.with_default_command(relay_binary_str.as_str())
				.with_default_args(vec!["-lruntime=debug".into()])
				.with_genesis_overrides(relay_genesis_overrides)
				.with_node(|node| node.with_name("alice").validator(true))
				.with_node(|node| node.with_name("bob").validator(true))
				.with_node(|node| node.with_name("charlie").validator(true))
		})
		.with_parachain(|parachain| {
			parachain
				.with_id(para_id)
				.with_chain_spec_path(chain_spec_str.as_str())
				.cumulus_based(true)
				// Collator 1
				.with_collator(|c| {
					c.with_name("collator-1")
						.validator(true)
						.with_command(para_binary_str.as_str())
						.with_args(vec![
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace,txpool=trace"
								.into(),
							"--ipfs-server".into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),

						])
				})
				// Collator 2
				.with_collator(|c| {
					c.with_name("collator-2")
						.validator(true)
						.with_command(para_binary_str2.as_str())
						.with_args(vec![
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace,txpool=trace"
								.into(),
							"--ipfs-server".into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),

						])
				})
				// RPC 1: non-collating full node
				.with_collator(|c| {
					c.with_name("rpc-1")
						.validator(false)
						.with_command(para_binary_str3.as_str())
						.with_args(vec![
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace,txpool=trace"
								.into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),

						])
				})
				// RPC 2: non-collating full node
				.with_collator(|c| {
					c.with_name("rpc-2")
						.validator(false)
						.with_command(para_binary_str4.as_str())
						.with_args(vec![
							"-lparachain=info,runtime=debug,runtime::transaction-storage=trace,txpool=trace"
								.into(),
							"--rpc-max-request-size=20".into(),
							"--rpc-max-response-size=20".into(),
							"--pool-kbytes=65536".into(),
							"--".into(),

						])
				})
		})
		.build()
		.map_err(|errs| anyhow!("Network config errors: {errs:?}"))?;

	let spawn_fn = zombienet_sdk::environment::get_spawn_fn();
	let network = spawn_fn(config).await?;

	// No core assignment needed — single core auto-assigned to the parachain.
	// Wait for relay chain to produce blocks.
	let alice = network.get_node("alice")?;
	tracing::info!("Waiting for relay chain to reach block 10...");
	alice
		.wait_metric_with_timeout("block_height{status=\"best\"}", |h| h >= 10.0, 300u64)
		.await?;

	// Wait for collators to start producing.
	let collator1 = network.get_node("collator-1")?;
	tracing::info!("Waiting for collator-1 to start producing blocks...");
	collator1
		.wait_metric_with_timeout("block_height{status=\"best\"}", |h| h >= 2.0, 300u64)
		.await?;

	// Wait for RPC nodes to sync.
	let rpc1 = network.get_node("rpc-1")?;
	tracing::info!("Waiting for rpc-1 to sync...");
	rpc1.wait_metric_with_timeout("block_height{status=\"best\"}", |h| h >= 1.0, 300u64)
		.await?;

	let rpc2 = network.get_node("rpc-2")?;
	tracing::info!("Waiting for rpc-2 to sync...");
	rpc2.wait_metric_with_timeout("block_height{status=\"best\"}", |h| h >= 1.0, 300u64)
		.await?;

	tracing::info!("Single-core network ready (2 collators, 2 RPCs)");

	Ok(network)
}

// --- Core assignment ---

/// Assign extra coretime cores to a parachain on the relay chain.
///
/// Submits one `sudo(Coretime::assign_core(...))` per core, sequentially.
/// The parachain already has 1 core from genesis registration; this assigns
/// additional cores. Must be called after the relay chain is running and
/// Alice has sudo.
async fn assign_cores(relay_ws_url: &str, para_id: u32, num_extra_cores: u32) -> Result<()> {
	tracing::info!(
		"Assigning {num_extra_cores} extra cores to para {para_id} on {relay_ws_url}..."
	);

	let client = OnlineClient::<SubstrateConfig>::from_url(relay_ws_url).await?;
	let alice = subxt_signer::sr25519::dev::alice();

	for core in 0..num_extra_cores {
		let assign_call = tx(
			"Coretime",
			"assign_core",
			vec![
				Value::u128(core as u128),
				Value::u128(0),
				Value::unnamed_composite([Value::unnamed_composite([
					Value::named_variant("Task", [("0".to_string(), Value::u128(para_id as u128))]),
					Value::u128(57600),
				])]),
				Value::unnamed_variant("None", []),
			],
		);

		let sudo_call = tx("Sudo", "sudo", vec![assign_call.into_value()]);

		let mut progress =
			client.tx().sign_and_submit_then_watch_default(&sudo_call, &alice).await?;
		// Wait for finalization so the nonce is updated for the next tx.
		while let Some(status) = progress.next().await {
			match status? {
				subxt::tx::TxStatus::InFinalizedBlock(_) => break,
				subxt::tx::TxStatus::Error { message } |
				subxt::tx::TxStatus::Invalid { message } |
				subxt::tx::TxStatus::Dropped { message } => anyhow::bail!("Core assignment failed: {message}"),
				_ => continue,
			}
		}

		tracing::info!("  Assigned core {core} to para {para_id}");
	}

	tracing::info!(
		"Core assignment complete, {num_extra_cores} extra cores assigned to para {para_id}"
	);

	Ok(())
}

// --- Helpers ---

pub fn get_node_ws_url(
	network: &zombienet_sdk::Network<LocalFileSystem>,
	name: &str,
) -> Result<String> {
	let node = network.get_node(name)?;
	Ok(node.ws_uri().to_string())
}
