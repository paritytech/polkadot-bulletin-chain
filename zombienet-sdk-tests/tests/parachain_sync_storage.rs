// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Parachain sync tests
//!
//! This module contains sync tests that run the bulletin chain as a parachain
//! on a relay chain. These tests verify sync behavior and transaction storage
//! (bitswap) functionality. The relay chain and parachain configuration can be
//! customized via environment variables (see below).
//!
//! ## Tests
//!
//! Test 1 is `#[ignore]`d: on polkadot-stable2609, `StorageChainBlockImport` executes the
//! skipped fast-sync block #1 and its import fails, so fast sync stalls at block #0.
//!
//! 1. `parachain_fast_sync_test` - Fast sync without block pruning
//!    - Starts 1 collator, stores transaction data
//!    - Adds a regular sync node with --sync=fast
//!    - Verifies state sync completes, bitswap returns DONT_HAVE (expected)
//!
//! 2. `parachain_fast_sync_with_pruning_test` - Fast sync with block pruning
//!    - Starts 1 collator with --blocks-pruning=15 on a chain with genesis retention period 10,
//!      stores transaction data
//!    - Waits until the collator has pruned every block up to the stores
//!    - Adds a regular sync node with --sync=fast
//!    - Verifies sync FAILS (peers respond with empty blocks - historical blocks pruned)
//!
//! 3. `parachain_warp_sync_test` - Warp sync without block pruning
//!    - Starts 3 relay validators, 1 collator, stores transaction data below the warp target
//!    - Adds an archive sync node with --sync=warp
//!    - Verifies the node resolves `GapSyncBodyPolicy::All`, gap sync restores the indexed
//!      transactions from the block bodies it downloads, and bitswap serves every item
//!
//! 4. `parachain_warp_sync_with_pruning_test` - Warp sync on a pruned sync node
//!    - Starts 3 relay validators, 1 archive collator, sets the retention period to 24
//!    - Stores items far below the later warp target, then one item just below it
//!    - Adds a sync node with --sync=warp and --blocks-pruning=24
//!    - Verifies the node resolves `GapSyncBodyPolicy::BodiesWithinWindow(12)`, bitswap serves the
//!      item inside the gap body window and returns DONT_HAVE for the items below it, which the
//!      collator still serves
//!
//! 5. `parachain_full_sync_test` - Full sync without block pruning
//!    - Starts 1 collator, stores transaction data
//!    - Adds a regular sync node with --sync=full
//!    - Verifies full sync completes, bitswap returns data (full sync downloads indexed body)
//!
//! 6. `parachain_full_sync_with_pruning_test` - Full sync with block pruning
//!    - Starts 1 collator with --blocks-pruning=15 on a chain with genesis retention period 10,
//!      stores transaction data
//!    - Waits until the collator has pruned every block up to the stores
//!    - Adds a regular sync node with --sync=full
//!    - Verifies sync FAILS (peers respond with empty blocks - historical blocks pruned)
//!
//! 7. `parachain_full_sync_relay_warp_sync_test` - Full sync parachain + warp sync relay chain
//!    - Starts 3 relay validators (GRANDPA finality for warp proofs), 1 collator, stores data
//!    - Adds sync node with --sync=full (parachain) and -- --sync=warp (embedded relay chain)
//!    - Verifies embedded relay chain warp syncs, parachain full syncs, bitswap returns data
//!
//! 8. `parachain_rpc_node_bitswap_test` - RPC node (non-collator) serving data via bitswap
//!    - Starts 1 collator, stores transaction data
//!    - Adds an RPC node with --sync=full (non-collator, --ipfs-server)
//!    - Verifies RPC node syncs and can serve historical data via bitswap
//!    - Submits new store extrinsic through RPC node (gossipped to collator for inclusion)
//!    - Verifies both collator and RPC node serve the newly produced data via bitswap
//!
//! 9. `parachain_ldb_storage_verification_test` - Database-level verification using rocksdb_ldb
//!    tool
//!    - Verifies col11 state before/after store operations
//!    - Verifies reference counting works correctly (refcount=1 after first store, refcount=2 after
//!      duplicate)
//!    - Verifies data expiration after retention period (col11 becomes empty)
//!
//! 10. `parachain_pruning_below_retention_test` - Pruning window below the retention period
//!     - Starts 3 relay validators, 1 collator on the main spec (retention period 201600)
//!     - Starts a --sync=full node with --blocks-pruning=1000 and verifies it refuses to start
//!     - Adds a --sync=warp node with --blocks-pruning=1000 and verifies it shuts down right after
//!       state sync, when it resolves its gap sync body policy
//!
//! ## Key Behavior Notes
//!
//! - **Full sync downloads indexed transactions**: Full sync (`--sync=full`) downloads all blocks
//!   including indexed body, so synced nodes CAN serve historical transaction data via bitswap.
//!
//! - **Warp sync restores indexed transactions from gap-sync bodies**: after state sync the node
//!   resolves a `GapSyncBodyPolicy` from the runtime's retention period at the warp target and its
//!   own `--blocks-pruning` (`polkadot-omni-node`'s `resolve_gap_sync_body_policy`). An archive
//!   node gets `All`; a pruned node gets `BodiesWithinWindow(retention - min(256, retention / 2))`.
//!   `StorageChainBlockImport` restores the indexed transactions of every gap block that arrives
//!   with a body, so the node serves that data via bitswap. Gap blocks below the window arrive
//!   header-only, and the warp target block itself is imported without its indexed transactions.
//!
//! - **The pruning window must cover the retention period**: a storage-chain node whose
//!   `--blocks-pruning` is below the runtime's retention period fails when it creates `ChainSync`:
//!   at startup for full and fast sync, right after state sync for warp sync. The syncing engine is
//!   an essential task, so the node shuts down.
//!
//! - **Fast sync does NOT download indexed transactions**: State sync skips block bodies entirely,
//!   so fast-synced nodes cannot serve historical transaction data via bitswap.
//!
//! - **Pruning prevents fast sync**: When all peers have pruning enabled, fast sync (state sync)
//!   cannot complete because historical blocks are unavailable for gap filling.
//!
//! - **Pruning affects data availability**: With block pruning, early blocks and their indexed
//!   transaction data are deleted. If data blocks are pruned before gap fill can download them, the
//!   data becomes unrecoverable.
//!
//! ## Environment Variables
//!
//! - `POLKADOT_RELAY_BINARY_PATH`: Path to the relay chain binary (default: "polkadot")
//! - `POLKADOT_PARACHAIN_BINARY_PATH`: Path to the parachain collator binary (default:
//!   "polkadot-omni-node")
//! - `PARACHAIN_CHAIN_SPEC_PATH`: Path to the parachain chain spec (default:
//!   "./zombienet/bulletin-westend-spec.json")
//! - `RELAY_CHAIN`: Relay chain spec name (default: "westend-local")
//! - `PARACHAIN_ID`: Parachain ID (default: 1010)
//! - `PARACHAIN_CHAIN_ID`: Chain ID for parachain DB path (default: "bulletin-westend")
//!
//! ## Running Tests
//!
//! Easiest: `just test-zombienet-sync westend parachain_fast_sync_test` from the repo
//! root. It fetches binaries, generates the chain spec, and invokes cargo. To run cargo
//! directly:
//!
//! ```bash
//! BIN_DIR=$(just binaries-polkadot)
//! POLKADOT_RELAY_BINARY_PATH=$BIN_DIR/polkadot \
//! POLKADOT_PARACHAIN_BINARY_PATH=$BIN_DIR/polkadot-omni-node \
//! PARACHAIN_CHAIN_SPEC_PATH=./zombienet/bulletin-westend-spec.json \
//!   cargo test --release -p bulletin-chain-zombienet-sdk-tests \
//!   --features bulletin-chain-zombienet-sdk-tests/zombie-sync-tests \
//!   parachain_fast_sync_test
//! ```

use crate::{
	test_log,
	utils::{
		arg_value, authorize_and_store_data, authorize_and_store_data_finalized,
		authorize_and_store_items, build_parachain_network_config_three_relay_validators,
		content_hash_and_cid, expect_all_items_bitswap_dont_have, expect_log_line,
		generate_test_data, get_alice_nonce, get_db_path, get_para_id, get_parachain_binary_path,
		get_parachain_chain_id, initialize_network, log_line_at_least_once, omni_node_exit_and_log,
		parachain_network_config_with_retention_period, set_retention_period,
		set_retention_period_finalized, verify_all_items_bitswap, verify_col11, verify_ldb_tool,
		verify_node_bitswap, verify_parachain_binaries, verify_state_sync_completed,
		verify_warp_sync_completed, wait_for_block_height, wait_for_finalized_height,
		wait_for_fullnode, wait_for_relay_chain_to_sync, wait_for_session_change_on_node,
		BLOCK_PRODUCTION_TIMEOUT_SECS, LOG_TIMEOUT_SECS, NETWORK_READY_TIMEOUT_SECS,
		NODE_LOG_CONFIG, PARACHAIN_TEST_DATA_PATTERN, SYNC_TIMEOUT_SECS, TEST_DATA_SIZE,
	},
};
use anyhow::{anyhow, Context, Result};
use futures::try_join;
use std::time::Duration;
use subxt::{config::substrate::SubstrateConfig, OnlineClient};
use zombienet_orchestrator::AddCollatorOptions;

const MIN_BLOCKS_BEFORE_SYNC_NODE: u64 = 10;
/// Session changes are critical for parachain block production.
const SESSION_CHANGE_TIMEOUT_SECS: u64 = 300;
const RETENTION_PERIOD: u32 = 10;
/// Three items of different sizes for multi-item verification.
const ITEM_SIZES: [usize; 3] = [TEST_DATA_SIZE, TEST_DATA_SIZE / 2, TEST_DATA_SIZE * 2];

const STORAGE_CHAIN_IMPORT_LOG_CONFIG: &str = "-lstorage-chain-block-import=debug";
const GAP_SYNC_COMPLETE_LOG: &str = "Block history download is complete";
const GAP_SYNC_RESTORE_LOG: &str = r"gap-sync block #[0-9]+ .*: [1-9][0-9]* indexed entries";
const STATE_SYNC_COMPLETE_LOG: &str = "State sync is complete, continuing with block sync.";
const BELOW_RETENTION_ERROR: &str =
	"must be at least the runtime's transaction storage retention period";
const GAP_SYNC_BODY_SAFETY_MARGIN: u32 = 256;

const fn gap_sync_body_window(retention_period: u32) -> u32 {
	if retention_period / 2 < GAP_SYNC_BODY_SAFETY_MARGIN {
		retention_period - retention_period / 2
	} else {
		retention_period - GAP_SYNC_BODY_SAFETY_MARGIN
	}
}

/// Uses libp2p for embedded relay chain to avoid litep2p race conditions.
fn get_para_node_args() -> Vec<String> {
	vec![
		"--ipfs-server".into(),
		NODE_LOG_CONFIG.into(),
		// Arguments after "--" are passed to the embedded relay chain client.
		"--".into(),
		"--network-backend=libp2p".into(),
	]
}

fn get_para_node_args_with_pruning(blocks_pruning: u32) -> Vec<String> {
	vec![
		"--ipfs-server".into(),
		format!("--blocks-pruning={}", blocks_pruning),
		NODE_LOG_CONFIG.into(),
		// Arguments after "--" are passed to the embedded relay chain client.
		"--".into(),
		"--network-backend=libp2p".into(),
	]
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "polkadot-stable2609: StorageChainBlockImport executes the skipped fast-sync block #1 and \
            its import fails, so fast sync stalls at block #0 (upstream bug)"]
async fn parachain_fast_sync_test() -> Result<()> {
	const TEST: &str = "para_fast_sync";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain Fast Sync Test (without pruning) ===");
	tracing::info!("This test verifies fast sync with 1 collator and a sync node");

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args();
	let config = build_parachain_network_config_three_relay_validators(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - this is when validators get assigned to the parachain
	// and collators can start producing backed blocks
	tracing::info!(
		"Waiting for relay chain session change (required for parachain block production)..."
	);
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	// Get collator
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Store multiple test data items with different sizes
	let nonce = get_alice_nonce(collator1).await?;
	let (stored_items, _) =
		authorize_and_store_items(collator1, PARACHAIN_TEST_DATA_PATTERN, &ITEM_SIZES, nonce)
			.await?;
	let last_store_block = stored_items.iter().map(|i| i.block_number).max().unwrap();
	tracing::info!("All {} items stored, last at block {}", stored_items.len(), last_store_block);

	// Verify all items can be fetched from collator1 via bitswap
	verify_all_items_bitswap(collator1, &stored_items, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	// State sync triggers when: finalized_number + 8 >= network_median
	let target_block = std::cmp::max(last_store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	tracing::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add a sync node with fast sync
	tracing::info!("Adding sync-node with --sync=fast");
	let para_binary = get_parachain_binary_path();
	let sync_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=fast".into(),
			"--ipfs-server".into(),
			NODE_LOG_CONFIG.into(),
			"--".into(),
			"--network-backend=libp2p".into(),
		],
		is_validator: false,
		..Default::default()
	};

	network.add_collator("sync-node", sync_node_opts, get_para_id()).await?;
	let sync_node = network.get_node("sync-node").context("Failed to get sync-node")?;

	// Wait for the sync node to sync
	wait_for_fullnode(sync_node).await?;
	tracing::info!("Verifying sync-node's sync progress (target: block {})", target_block);
	wait_for_block_height(sync_node, target_block, SYNC_TIMEOUT_SECS).await?;

	// Verify state sync was used
	verify_state_sync_completed(sync_node).await?;

	// Verify bitswap returns DONT_HAVE for all items from sync-node
	// This is expected because state sync does not download indexed transaction data.
	expect_all_items_bitswap_dont_have(sync_node, &stored_items, 30, "sync-node").await?;
	tracing::info!("Note: sync-node doesn't have indexed transactions - this is expected for state-synced nodes");

	test_log!(TEST, "=== Parachain Fast Sync Test (without pruning) PASSED ===");
	network.destroy().await?;
	Ok(())
}

/// Must be at least `RETENTION_PERIOD`, the genesis retention period of the pruned-node
/// networks: a storage-chain node refuses to
/// start when its `--blocks-pruning` window is smaller.
const PRUNING_BLOCKS: u32 = 15;
const PRUNING_MARGIN_BLOCKS: u64 = 2;

#[tokio::test(flavor = "multi_thread")]
async fn parachain_fast_sync_with_pruning_test() -> Result<()> {
	const TEST: &str = "para_fast_sync_pruning";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain Fast Sync Test (with pruning) ===");
	tracing::info!(
		"Using 1 collator with --blocks-pruning={}, retention-period={}",
		PRUNING_BLOCKS,
		RETENTION_PERIOD
	);

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(PRUNING_BLOCKS);
	let config = parachain_network_config_with_retention_period(para_args, RETENTION_PERIOD)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	tracing::info!("Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Set retention period and store multiple data items
	tracing::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	let collator1_client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;
	let mut nonce = get_alice_nonce(collator1).await?;

	set_retention_period(&collator1_client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	let (stored_items, _) =
		authorize_and_store_items(collator1, PARACHAIN_TEST_DATA_PATTERN, &ITEM_SIZES, nonce)
			.await?;
	let last_store_block = stored_items.iter().map(|i| i.block_number).max().unwrap();

	// Verify all items can be fetched from collator1 via bitswap
	verify_all_items_bitswap(collator1, &stored_items, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(last_store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	tracing::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;
	let pruned_height = target_block + PRUNING_BLOCKS as u64 + PRUNING_MARGIN_BLOCKS;
	tracing::info!(
		"Waiting for finality at block {} so the collator prunes the stores",
		pruned_height
	);
	wait_for_finalized_height(collator1, pruned_height, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	expect_all_items_bitswap_dont_have(collator1, &stored_items, 30, "Collator-1").await?;

	// Add sync node with fast sync and pruning
	tracing::info!("Adding sync-node with --sync=fast and --blocks-pruning");
	let para_binary = get_parachain_binary_path();
	let sync_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=fast".into(),
			"--ipfs-server".into(),
			format!("--blocks-pruning={}", PRUNING_BLOCKS).as_str().into(),
			NODE_LOG_CONFIG.into(),
			"--".into(),
			"--network-backend=libp2p".into(),
		],
		is_validator: false,
		..Default::default()
	};

	network.add_collator("sync-node", sync_node_opts, get_para_id()).await?;
	let sync_node = network.get_node("sync-node").context("Failed to get sync-node")?;

	// Wait for the sync node to start up, discover peers, and attempt block requests.
	// Parachain nodes need extra time: embedded relay chain must initialize and connect.
	wait_for_fullnode(sync_node).await?;

	// With pruning enabled on all nodes, historical blocks are unavailable.
	// sync-node cannot sync because blocks 1-N are pruned on peers.
	// We expect to see "BlockResponse ... with 0 blocks" - peers don't have the blocks.
	tracing::info!("Expecting sync-node sync to fail (historical blocks are pruned on peers)");

	// Wait for the telltale sign: peers responding with 0 blocks (they don't have them)
	let zero_blocks_response = sync_node
		.wait_log_line_count_with_timeout(
			"with 0 blocks",
			false,
			log_line_at_least_once(SYNC_TIMEOUT_SECS),
		)
		.await;

	match zero_blocks_response {
		Ok(result) if result.success() => {
			tracing::info!(
				"✓ Detected 'BlockResponse with 0 blocks' - peers don't have pruned blocks"
			);
		},
		_ => {
			anyhow::bail!("Expected to detect 'BlockResponse with 0 blocks' in logs, but did not find it within timeout");
		},
	}

	test_log!(TEST, "=== Parachain Fast Sync Test (with pruning) PASSED ===");
	tracing::info!(
		"Note: This test verifies that sync cannot complete when historical blocks are pruned"
	);
	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_warp_sync_test() -> Result<()> {
	const TEST: &str = "para_warp_sync";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain Warp Sync Test ===");
	tracing::info!("This test requires 3 relay validators for GRANDPA finality");

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args();
	let config = build_parachain_network_config_three_relay_validators(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	tracing::info!("Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	// Get collator
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Store multiple test data items
	let nonce = get_alice_nonce(collator1).await?;
	let (stored_items, _) =
		authorize_and_store_items(collator1, PARACHAIN_TEST_DATA_PATTERN, &ITEM_SIZES, nonce)
			.await?;
	let last_store_block = stored_items.iter().map(|i| i.block_number).max().unwrap();

	// Verify all items can be fetched from collator1 via bitswap
	verify_all_items_bitswap(collator1, &stored_items, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(last_store_block + 1, MIN_BLOCKS_BEFORE_SYNC_NODE);
	tracing::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add a sync node with warp sync. We point its cumulus relay-chain interface at
	// alice's already-synced RPC instead of running an embedded relay client. Otherwise
	// cumulus's wait_for_finalized_para_head (cumulus/client/service/src/lib.rs) races
	// the embedded relay's cold start and locks in target = parachain genesis, which
	// neuters warp sync.
	tracing::info!("Adding sync-node with --sync=warp (external relay RPC: alice)");
	let para_binary = get_parachain_binary_path();
	let alice_rpc_url = relay_alice.ws_uri().to_string();
	let sync_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=warp".into(),
			"--ipfs-server".into(),
			"--relay-chain-rpc-url".into(),
			alice_rpc_url.as_str().into(),
			NODE_LOG_CONFIG.into(),
			STORAGE_CHAIN_IMPORT_LOG_CONFIG.into(),
		],
		is_validator: false,
		..Default::default()
	};

	network.add_collator("sync-node", sync_node_opts, get_para_id()).await?;
	let sync_node = network.get_node("sync-node").context("Failed to get sync-node")?;

	// Wait for the node to be up first
	wait_for_fullnode(sync_node).await?;

	tracing::info!("Verifying sync-node's progress (target: block {})", target_block);
	wait_for_block_height(sync_node, target_block, SYNC_TIMEOUT_SECS)
		.await
		.context("Sync node failed to sync via warp sync")?;

	// Verify warp sync completed and node is idle
	verify_warp_sync_completed(sync_node).await?;

	expect_log_line(
		sync_node,
		"Resolved gap sync body policy All ",
		LOG_TIMEOUT_SECS,
		"archive sync node did not resolve GapSyncBodyPolicy::All",
	)
	.await?;
	expect_log_line(
		sync_node,
		GAP_SYNC_COMPLETE_LOG,
		SYNC_TIMEOUT_SECS,
		"gap sync did not finish downloading the block history below the warp target",
	)
	.await?;
	expect_log_line(
		sync_node,
		GAP_SYNC_RESTORE_LOG,
		LOG_TIMEOUT_SECS,
		"gap sync imported no block whose indexed transactions it restored",
	)
	.await?;
	verify_all_items_bitswap(sync_node, &stored_items, 30, "Sync-node").await?;

	test_log!(TEST, "=== Parachain Warp Sync Test PASSED ===");
	network.destroy().await?;
	Ok(())
}

const WINDOW_TEST_RETENTION_PERIOD: u32 = 24;
const WINDOW_TEST_MAX_BLOCKS_PER_REQUEST: u64 = 4;
const WINDOW_TEST_MARGIN_BLOCKS: u64 = 3;

#[tokio::test(flavor = "multi_thread")]
async fn parachain_warp_sync_with_pruning_test() -> Result<()> {
	const TEST: &str = "para_warp_sync_pruning";
	crate::utils::init_logging();

	let body_window = gap_sync_body_window(WINDOW_TEST_RETENTION_PERIOD);
	test_log!(TEST, "=== Parachain Warp Sync Test (pruned sync node) ===");
	tracing::info!(
		"Sync node will use --blocks-pruning={} against retention-period={}, gap body window {}",
		WINDOW_TEST_RETENTION_PERIOD,
		WINDOW_TEST_RETENTION_PERIOD,
		body_window
	);

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let config = build_parachain_network_config_three_relay_validators(get_para_node_args())?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	tracing::info!("Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	// Get collator
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	tracing::info!("Setting RetentionPeriod to {} blocks", WINDOW_TEST_RETENTION_PERIOD);
	let collator1_client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;
	let nonce = get_alice_nonce(collator1).await?;
	set_retention_period_finalized(&collator1_client, WINDOW_TEST_RETENTION_PERIOD, nonce).await?;

	let (old_items, nonce) =
		authorize_and_store_items(collator1, b"WINDOW_TEST_OLD_DATA_", &ITEM_SIZES, nonce + 1)
			.await?;
	let last_old_block = old_items.iter().map(|item| item.block_number).max().unwrap();
	let header_only_height = last_old_block +
		body_window as u64 +
		WINDOW_TEST_MAX_BLOCKS_PER_REQUEST +
		WINDOW_TEST_MARGIN_BLOCKS;
	tracing::info!(
		"Waiting for finality at block {} so the old items fall below the gap body window",
		header_only_height
	);
	wait_for_finalized_height(collator1, header_only_height, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	let (recent_items, _) =
		authorize_and_store_items(collator1, b"WINDOW_TEST_RECENT_DATA_", &[TEST_DATA_SIZE], nonce)
			.await?;
	let target_block = recent_items[0].block_number + 1;
	tracing::info!(
		"Waiting for finality at block {} so the warp target lies above every store",
		target_block
	);
	wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// Add sync node with warp sync. We point its cumulus relay-chain interface at
	// alice's already-synced RPC instead of running an embedded relay client. Otherwise
	// cumulus's wait_for_finalized_para_head races the embedded relay's cold start and
	// locks in target = parachain genesis, which neuters warp sync.
	tracing::info!(
		"Adding sync-node with --sync=warp and --blocks-pruning (external relay RPC: alice)"
	);
	let para_binary = get_parachain_binary_path();
	let alice_rpc_url = relay_alice.ws_uri().to_string();
	let sync_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=warp".into(),
			"--ipfs-server".into(),
			format!("--blocks-pruning={WINDOW_TEST_RETENTION_PERIOD}").as_str().into(),
			format!("--max-blocks-per-request={WINDOW_TEST_MAX_BLOCKS_PER_REQUEST}")
				.as_str()
				.into(),
			"--relay-chain-rpc-url".into(),
			alice_rpc_url.as_str().into(),
			NODE_LOG_CONFIG.into(),
			STORAGE_CHAIN_IMPORT_LOG_CONFIG.into(),
		],
		is_validator: false,
		..Default::default()
	};

	network.add_collator("sync-node", sync_node_opts, get_para_id()).await?;
	let sync_node = network.get_node("sync-node").context("Failed to get sync-node")?;

	// Wait for the node to be up first
	wait_for_fullnode(sync_node).await?;

	tracing::info!("Verifying sync-node's progress (target: block {})", target_block);
	wait_for_block_height(sync_node, target_block, SYNC_TIMEOUT_SECS)
		.await
		.context("Sync node failed to sync via warp sync")?;
	verify_warp_sync_completed(sync_node).await?;

	expect_log_line(
		sync_node,
		&format!(r"Resolved gap sync body policy BodiesWithinWindow\({body_window}\)"),
		LOG_TIMEOUT_SECS,
		&format!(
			"pruned sync node did not resolve GapSyncBodyPolicy::BodiesWithinWindow({body_window})"
		),
	)
	.await?;
	expect_log_line(
		sync_node,
		GAP_SYNC_COMPLETE_LOG,
		SYNC_TIMEOUT_SECS,
		"gap sync did not finish downloading the block history below the warp target",
	)
	.await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	verify_all_items_bitswap(sync_node, &recent_items, 30, "Sync-node").await?;
	expect_all_items_bitswap_dont_have(sync_node, &old_items, 30, "Sync-node").await?;
	verify_all_items_bitswap(collator1, &old_items, 30, "Collator-1").await?;

	test_log!(TEST, "=== Parachain Warp Sync Test (pruned sync node) PASSED ===");
	network.destroy().await?;
	Ok(())
}

const BELOW_RETENTION_BLOCKS_PRUNING: u32 = 1000;
const STARTUP_REFUSAL_TIMEOUT_SECS: u64 = 120;

#[tokio::test(flavor = "multi_thread")]
async fn parachain_pruning_below_retention_test() -> Result<()> {
	const TEST: &str = "para_pruning_below_retention";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain Pruning Window Below Retention Period Test ===");
	verify_parachain_binaries()?;

	let config = build_parachain_network_config_three_relay_validators(get_para_node_args())?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	try_join!(
		wait_for_block_height(
			collator1,
			MIN_BLOCKS_BEFORE_SYNC_NODE,
			BLOCK_PRODUCTION_TIMEOUT_SECS
		),
		wait_for_finalized_height(
			collator1,
			MIN_BLOCKS_BEFORE_SYNC_NODE,
			BLOCK_PRODUCTION_TIMEOUT_SECS
		),
	)?;

	let para_binary = get_parachain_binary_path();
	let alice_rpc_url = relay_alice.ws_uri().to_string();
	let collator_args: Vec<String> = collator1.args().iter().map(|arg| arg.to_string()).collect();
	let relay_args_start = collator_args
		.iter()
		.position(|arg| arg == "--")
		.context("collator-1 args have no relay chain section")?;
	let para_chain_spec = arg_value(&collator_args[..relay_args_start], "--chain")
		.context("collator-1 args do not contain --chain")?;
	let relay_chain_spec = arg_value(&collator_args[relay_args_start + 1..], "--chain")
		.context("collator-1 relay chain args do not contain --chain")?;
	let full_node_base_path = tempfile::tempdir()?;
	let full_node_args: Vec<String> = vec![
		"--chain".into(),
		para_chain_spec,
		"--base-path".into(),
		full_node_base_path.path().display().to_string(),
		"--sync=full".into(),
		format!("--blocks-pruning={BELOW_RETENTION_BLOCKS_PRUNING}"),
		"--relay-chain-rpc-url".into(),
		alice_rpc_url.clone(),
		"--port".into(),
		"0".into(),
		"--rpc-port".into(),
		"0".into(),
		"--no-prometheus".into(),
		"--no-telemetry".into(),
		"--".into(),
		"--chain".into(),
		relay_chain_spec,
	];
	let (exit_code, full_node_log) = omni_node_exit_and_log(
		&para_binary,
		&full_node_args,
		Duration::from_secs(STARTUP_REFUSAL_TIMEOUT_SECS),
	)
	.await?;
	assert!(
		matches!(exit_code, Some(code) if code != 0),
		"full-sync node with --blocks-pruning={BELOW_RETENTION_BLOCKS_PRUNING} did not refuse to \
		 start (exit={exit_code:?}):\n{full_node_log}",
	);
	assert!(
		full_node_log.contains(BELOW_RETENTION_ERROR),
		"full-sync node exited ({exit_code:?}) without the retention period error:\n{full_node_log}",
	);
	test_log!(TEST, "✓ Full-sync node refused to start");

	let warp_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=warp".into(),
			"--ipfs-server".into(),
			format!("--blocks-pruning={BELOW_RETENTION_BLOCKS_PRUNING}").as_str().into(),
			"--relay-chain-rpc-url".into(),
			alice_rpc_url.as_str().into(),
			NODE_LOG_CONFIG.into(),
		],
		is_validator: false,
		..Default::default()
	};
	network.add_collator("warp-node", warp_node_opts, get_para_id()).await?;
	let warp_node = network.get_node("warp-node").context("Failed to get warp-node")?;
	wait_for_fullnode(warp_node).await?;
	expect_log_line(
		warp_node,
		BELOW_RETENTION_ERROR,
		SYNC_TIMEOUT_SECS,
		"warp-sync node did not reject its pruning window",
	)
	.await?;
	expect_log_line(
		warp_node,
		"Essential task `syncing` failed",
		LOG_TIMEOUT_SECS,
		"warp-sync node kept running without its syncing engine",
	)
	.await?;
	let warp_node_log = warp_node.logs().await?;
	let state_sync_end = warp_node_log
		.find(STATE_SYNC_COMPLETE_LOG)
		.context("warp-sync node did not complete state sync")?;
	let rejection = warp_node_log
		.find(BELOW_RETENTION_ERROR)
		.context("warp-sync node log lacks the retention period error")?;
	assert!(
		state_sync_end < rejection,
		"warp-sync node rejected its pruning window before state sync completed",
	);
	test_log!(TEST, "✓ Warp-sync node shut down after state sync");

	test_log!(TEST, "=== Parachain Pruning Window Below Retention Period Test PASSED ===");
	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_full_sync_test() -> Result<()> {
	const TEST: &str = "para_full_sync";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain Full Sync Test (without pruning) ===");
	tracing::info!("This test verifies full sync with 1 collator and a sync node");

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args();
	let config = build_parachain_network_config_three_relay_validators(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	tracing::info!(
		"Waiting for relay chain session change (required for parachain block production)..."
	);
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	// Get collator
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Store multiple test data items
	let nonce = get_alice_nonce(collator1).await?;
	let (stored_items, _) =
		authorize_and_store_items(collator1, PARACHAIN_TEST_DATA_PATTERN, &ITEM_SIZES, nonce)
			.await?;
	let last_store_block = stored_items.iter().map(|i| i.block_number).max().unwrap();

	// Verify all items can be fetched from collator1 via bitswap
	verify_all_items_bitswap(collator1, &stored_items, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(last_store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	tracing::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add a sync node with full sync
	tracing::info!("Adding sync-node with --sync=full");
	let para_binary = get_parachain_binary_path();
	let sync_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=full".into(),
			"--ipfs-server".into(),
			NODE_LOG_CONFIG.into(),
			"--".into(),
			"--network-backend=libp2p".into(),
		],
		is_validator: false,
		..Default::default()
	};

	network.add_collator("sync-node", sync_node_opts, get_para_id()).await?;
	let sync_node = network.get_node("sync-node").context("Failed to get sync-node")?;

	// Wait for the sync node to sync
	wait_for_fullnode(sync_node).await?;
	tracing::info!("Verifying sync-node's sync progress (target: block {})", target_block);
	wait_for_block_height(sync_node, target_block, SYNC_TIMEOUT_SECS).await?;

	// Verify all items via bitswap from sync-node
	// Full sync downloads all blocks including indexed body, so bitswap should work
	verify_all_items_bitswap(sync_node, &stored_items, 30, "sync-node").await?;
	tracing::info!("✓ Bitswap works from sync-node - full sync downloads indexed transactions");

	test_log!(TEST, "=== Parachain Full Sync Test (without pruning) PASSED ===");
	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_full_sync_with_pruning_test() -> Result<()> {
	const TEST: &str = "para_full_sync_pruning";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain Full Sync Test (with pruning) ===");
	tracing::info!(
		"Using 1 collator with --blocks-pruning={}, retention-period={}",
		PRUNING_BLOCKS,
		RETENTION_PERIOD
	);

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(PRUNING_BLOCKS);
	let config = parachain_network_config_with_retention_period(para_args, RETENTION_PERIOD)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	tracing::info!("Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Set retention period and store multiple data items
	tracing::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	let collator1_client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;
	let mut nonce = get_alice_nonce(collator1).await?;

	set_retention_period(&collator1_client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	let (stored_items, _) =
		authorize_and_store_items(collator1, PARACHAIN_TEST_DATA_PATTERN, &ITEM_SIZES, nonce)
			.await?;
	let last_store_block = stored_items.iter().map(|i| i.block_number).max().unwrap();

	// Verify all items can be fetched from collator1 via bitswap
	verify_all_items_bitswap(collator1, &stored_items, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(last_store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	tracing::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;
	let pruned_height = target_block + PRUNING_BLOCKS as u64 + PRUNING_MARGIN_BLOCKS;
	tracing::info!(
		"Waiting for finality at block {} so the collator prunes the stores",
		pruned_height
	);
	wait_for_finalized_height(collator1, pruned_height, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	expect_all_items_bitswap_dont_have(collator1, &stored_items, 30, "Collator-1").await?;

	// Add sync node with full sync and pruning
	tracing::info!("Adding sync-node with --sync=full and --blocks-pruning");
	let para_binary = get_parachain_binary_path();
	let sync_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=full".into(),
			"--ipfs-server".into(),
			format!("--blocks-pruning={}", PRUNING_BLOCKS).as_str().into(),
			NODE_LOG_CONFIG.into(),
			"--".into(),
			"--network-backend=libp2p".into(),
		],
		is_validator: false,
		..Default::default()
	};

	network.add_collator("sync-node", sync_node_opts, get_para_id()).await?;
	let sync_node = network.get_node("sync-node").context("Failed to get sync-node")?;

	// Wait for the sync node to start up, discover peers, and attempt block requests.
	wait_for_fullnode(sync_node).await?;

	// With pruning enabled on all nodes, historical blocks are unavailable.
	// sync-node cannot sync because blocks 1-N are pruned on peers.
	// We expect to see "BlockResponse ... with 0 blocks" - peers don't have the blocks.
	tracing::info!("Expecting sync-node sync to fail (historical blocks are pruned on peers)");

	// Wait for the telltale sign: peers responding with 0 blocks (they don't have them)
	let zero_blocks_response = sync_node
		.wait_log_line_count_with_timeout(
			"with 0 blocks",
			false,
			log_line_at_least_once(SYNC_TIMEOUT_SECS),
		)
		.await;

	match zero_blocks_response {
		Ok(result) if result.success() => {
			tracing::info!(
				"✓ Detected 'BlockResponse with 0 blocks' - peers don't have pruned blocks"
			);
		},
		_ => {
			anyhow::bail!("Expected to detect 'BlockResponse with 0 blocks' in logs, but did not find it within timeout");
		},
	}

	test_log!(TEST, "=== Parachain Full Sync Test (with pruning) PASSED ===");
	tracing::info!(
		"Note: This test verifies that sync cannot complete when historical blocks are pruned"
	);
	network.destroy().await?;
	Ok(())
}

/// Parachain full sync with embedded relay chain warp sync.
///
/// This test verifies that a parachain sync node can use `--sync=full` for the parachain
/// while the embedded relay chain uses `--sync=warp`. The relay chain warp syncs first
/// (requiring GRANDPA finality proofs → 3 relay validators), then the parachain does a
/// full sync downloading all blocks including indexed body — so bitswap should work.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_full_sync_relay_warp_sync_test() -> Result<()> {
	const TEST: &str = "para_full_sync_relay_warp";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain Full Sync + Relay Warp Sync Test ===");
	tracing::info!("Parachain: --sync=full, Embedded relay chain: --sync=warp");
	tracing::info!("Requires 3 relay validators for GRANDPA finality (warp sync proofs)");

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args();
	let config = build_parachain_network_config_three_relay_validators(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	tracing::info!("Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	// Get collator
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Store multiple test data items
	let nonce = get_alice_nonce(collator1).await?;
	let (stored_items, _) =
		authorize_and_store_items(collator1, PARACHAIN_TEST_DATA_PATTERN, &ITEM_SIZES, nonce)
			.await?;
	let last_store_block = stored_items.iter().map(|i| i.block_number).max().unwrap();

	// Verify all items can be fetched from collator1 via bitswap
	verify_all_items_bitswap(collator1, &stored_items, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(last_store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	tracing::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add a sync node with full sync for parachain + warp sync for embedded relay chain
	tracing::info!("Adding sync-node with --sync=full (parachain) + --sync=warp (relay chain)");
	let para_binary = get_parachain_binary_path();
	let sync_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=full".into(),
			"--ipfs-server".into(),
			NODE_LOG_CONFIG.into(),
			// Arguments after "--" are passed to the embedded relay chain client.
			"--".into(),
			"--sync=warp".into(),
			"--network-backend=libp2p".into(),
		],
		is_validator: false,
		..Default::default()
	};

	network.add_collator("sync-node", sync_node_opts, get_para_id()).await?;
	let sync_node = network.get_node("sync-node").context("Failed to get sync-node")?;

	// Wait for the node to be up first
	wait_for_fullnode(sync_node).await?;

	// Wait for the sync node's embedded relay chain to sync via warp sync.
	tracing::info!("Waiting for sync-node's embedded relay chain to warp sync...");
	wait_for_relay_chain_to_sync(sync_node, SYNC_TIMEOUT_SECS)
		.await
		.context("Sync node's embedded relay chain did not sync via warp")?;

	tracing::info!(
		"Verifying sync-node's parachain full sync progress (target: block {})",
		target_block
	);
	wait_for_block_height(sync_node, target_block, SYNC_TIMEOUT_SECS)
		.await
		.context("Sync node failed to full-sync parachain blocks")?;

	// Full sync downloads all blocks including indexed body, so bitswap should work
	verify_all_items_bitswap(sync_node, &stored_items, 30, "sync-node").await?;
	tracing::info!("✓ Bitswap works from sync-node - full sync downloads indexed transactions");

	test_log!(TEST, "=== Parachain Full Sync + Relay Warp Sync Test PASSED ===");
	network.destroy().await?;
	Ok(())
}

/// Parachain RPC node (non-collator) that syncs via full sync and serves data via bitswap.
///
/// This test verifies the "RPC node" deployment pattern: a non-collator node that
/// full-syncs with collators and can serve historical and live transaction data over
/// bitswap. This is the expected production setup for nodes that serve data to external
/// consumers (e.g., IPFS gateways) without participating in block production.
///
/// Test flow:
/// 1. Collator stores data, verify bitswap works on collator
/// 2. RPC node joins with --sync=full + --ipfs-server (non-collator)
/// 3. RPC node full-syncs and can serve historical data via bitswap
/// 4. New store extrinsic submitted through RPC node (gossipped to collator)
/// 5. Both collator and RPC node serve the new data via bitswap
#[tokio::test(flavor = "multi_thread")]
async fn parachain_rpc_node_bitswap_test() -> Result<()> {
	const TEST: &str = "para_rpc_node";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain RPC Node Bitswap Test ===");
	tracing::info!("This test verifies a non-collator RPC node can serve data via bitswap");

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args();
	let config = build_parachain_network_config_three_relay_validators(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	tracing::info!(
		"Waiting for relay chain session change (required for parachain block production)..."
	);
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	// Store multiple historical data items on collator before RPC node joins.
	// Scoped to release the immutable borrow before add_collator.
	let (historical_items, nonce, target_block) = {
		let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

		let nonce = get_alice_nonce(collator1).await?;
		let (items, next_nonce) =
			authorize_and_store_items(collator1, b"PARA_RPC_HISTORICAL_", &ITEM_SIZES, nonce)
				.await?;
		let last_store_block = items.iter().map(|i| i.block_number).max().unwrap();
		tracing::info!("Historical items stored, last at block {}", last_store_block);

		// Verify all items can be fetched from collator1 via bitswap
		verify_all_items_bitswap(collator1, &items, 30, "Collator-1 (historical)").await?;

		// Wait for enough blocks and finality before adding RPC node
		let target_block = std::cmp::max(last_store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
		tracing::info!("Waiting for block {} and finality", target_block);
		try_join!(
			wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
			wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		)?;

		(items, next_nonce, target_block)
	};

	// Add RPC node: non-collator with full sync and ipfs-server
	tracing::info!("Adding rpc-node with --sync=full --ipfs-server (non-collator)");
	let para_binary = get_parachain_binary_path();
	let rpc_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=full".into(),
			"--ipfs-server".into(),
			NODE_LOG_CONFIG.into(),
			"--".into(),
			"--network-backend=libp2p".into(),
		],
		is_validator: false,
		..Default::default()
	};

	network.add_collator("rpc-node", rpc_node_opts, get_para_id()).await?;

	// Re-acquire node references after mutable borrow
	let rpc_node = network.get_node("rpc-node").context("Failed to get rpc-node")?;

	// Wait for the RPC node to sync
	wait_for_fullnode(rpc_node).await?;
	tracing::info!("Verifying rpc-node's sync progress (target: block {})", target_block);
	wait_for_block_height(rpc_node, target_block, SYNC_TIMEOUT_SECS).await?;

	// Verify RPC node can serve all historical items via bitswap
	verify_all_items_bitswap(rpc_node, &historical_items, 30, "rpc-node (historical)").await?;
	tracing::info!("✓ RPC node serves all historical items via bitswap after full sync");

	// Now store NEW data by submitting the transaction THROUGH the RPC node.
	// This is the realistic production pattern: users submit extrinsics to an RPC node,
	// which gossips them to collators for inclusion in blocks.
	let live_data = generate_test_data(TEST_DATA_SIZE, b"PARA_RPC_LIVE_DATA_");
	let (live_hash, live_cid) = content_hash_and_cid(&live_data);
	tracing::info!("Storing live data via RPC node: {} bytes", live_data.len());
	tracing::info!("Live content hash: {}, CID: {}", live_hash, live_cid);

	let (live_store_block, _) = authorize_and_store_data(rpc_node, &live_data, nonce).await?;
	tracing::info!("Live data stored at block {} (submitted via RPC node)", live_store_block);

	// Verify collator has the live data (it was included by collator even though
	// the transaction was submitted through the RPC node)
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	verify_node_bitswap(collator1, &live_data, 30, "Collator-1 (live)").await?;

	// Verify RPC node can also serve the data it helped submit
	let rpc_node = network.get_node("rpc-node").context("Failed to get rpc-node")?;
	tracing::info!("Waiting for rpc-node to catch up to block {}", live_store_block);
	wait_for_block_height(rpc_node, live_store_block, SYNC_TIMEOUT_SECS).await?;

	verify_node_bitswap(rpc_node, &live_data, 30, "rpc-node (live)").await?;
	tracing::info!("✓ RPC node serves live data via bitswap (tx submitted through RPC node)");

	test_log!(TEST, "=== Parachain RPC Node Bitswap Test PASSED ===");
	network.destroy().await?;
	Ok(())
}

/// Long enough for both stores before expiration (~6 blocks each on parachain).
const LDB_TEST_RETENTION_PERIOD: u32 = 20;

#[tokio::test(flavor = "multi_thread")]
async fn parachain_ldb_storage_verification_test() -> Result<()> {
	const TEST: &str = "para_ldb_storage";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain LDB Storage Verification Test ===");
	tracing::info!(
		"This test verifies transaction storage database behavior using rocksdb_ldb tool"
	);
	tracing::info!(
		"Using --blocks-pruning={} and retention-period={}",
		LDB_TEST_RETENTION_PERIOD,
		LDB_TEST_RETENTION_PERIOD
	);

	// === Early validation of required external tools ===
	tracing::info!("=== Verifying required external tools ===");
	verify_ldb_tool()?;
	verify_parachain_binaries()?;

	let para_args = vec![
		"--ipfs-server".into(),
		"--database=rocksdb".into(),
		format!("--blocks-pruning={}", LDB_TEST_RETENTION_PERIOD),
		"-ltransaction-storage=trace".into(),
		"--".into(),
		"--network-backend=libp2p".into(),
	];
	let config =
		parachain_network_config_with_retention_period(para_args, LDB_TEST_RETENTION_PERIOD)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	tracing::info!(
		"Waiting for relay chain session change (required for parachain block production)..."
	);
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Wait for parachain to start producing blocks after session change
	tracing::info!("Waiting for parachain to produce blocks...");
	wait_for_block_height(collator1, 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// Get client and fetch initial nonce for Alice
	let collator1_client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;
	let mut nonce = get_alice_nonce(collator1).await?;

	// Set short retention period for fast testing
	// NOTE: LDB test uses finalized transactions for database consistency
	tracing::info!(
		"Setting RetentionPeriod to {} blocks for fast expiration testing",
		LDB_TEST_RETENTION_PERIOD
	);
	set_retention_period_finalized(&collator1_client, LDB_TEST_RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// Get database path for the collator
	let base_dir = network
		.base_dir()
		.ok_or_else(|| anyhow!("Failed to get network base directory"))?
		.to_string();
	let parachain_chain_id = get_parachain_chain_id();
	let collator_db_path = get_db_path(&base_dir, "collator-1", &parachain_chain_id);
	tracing::info!("Collator-1 DB path: {:?}", collator_db_path);

	// === Step 1: Verify col11 is empty before store ===
	test_log!(TEST, "=== Step 1: Verify col11 is empty BEFORE store ===");
	let dump = verify_col11(&collator_db_path, "col11 BEFORE store")?;
	if !dump.is_empty() {
		anyhow::bail!("Expected col11 to be empty before store, but found {} keys", dump.key_count);
	}
	tracing::info!("✓ col11 is empty as expected before store");

	// Generate test data and calculate expected content hash
	let test_data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (expected_hash, expected_cid) = content_hash_and_cid(&test_data);
	tracing::info!("Generated {} bytes of test data", test_data.len());
	tracing::info!("Expected content hash: {}", expected_hash);
	tracing::info!("Expected CID: {}", expected_cid);

	// === Step 2: First store - verify refcount = 1 and content hash matches ===
	test_log!(TEST, "=== Step 2: First store - expecting refcount = 1 ===");
	let (first_store_block, next_nonce) =
		authorize_and_store_data_finalized(collator1, &test_data, nonce).await?;
	nonce = next_nonce;
	tracing::info!("First store completed at block {}", first_store_block);

	let dump = verify_col11(&collator_db_path, "col11 AFTER first store")?;
	if dump.key_count != 2 {
		anyhow::bail!(
			"Expected 2 keys in col11 after first store (data + refcount), found {}",
			dump.key_count
		);
	}

	let data_entries = dump.data_entries();
	let data_entry = data_entries
		.first()
		.ok_or_else(|| anyhow!("No data entries found in col11 after first store"))?;
	let stored_hash = data_entry.content_hash();

	// Verify the content hash matches our calculated hash
	if !stored_hash.eq_ignore_ascii_case(&expected_hash) {
		anyhow::bail!("Content hash mismatch! Expected: {}, Got: {}", expected_hash, stored_hash);
	}
	tracing::info!("✓ Content hash matches: {}", stored_hash);

	let refcount = dump
		.get_refcount(stored_hash)
		.ok_or_else(|| anyhow!("Could not find refcount for content hash {}", stored_hash))?;
	if refcount != 1 {
		anyhow::bail!("Expected refcount=1 after first store, found refcount={}", refcount);
	}
	tracing::info!("✓ Reference count is 1 as expected after first store");

	// === Step 3: Second store (same data) - verify refcount = 2, still 2 keys ===
	test_log!(TEST, "=== Step 3: Second store - expecting refcount = 2, still 2 keys ===");
	let (second_store_block, _) =
		authorize_and_store_data_finalized(collator1, &test_data, nonce).await?;
	tracing::info!("Second store completed at block {}", second_store_block);

	let dump = verify_col11(&collator_db_path, "col11 AFTER second store")?;
	if dump.key_count != 2 {
		anyhow::bail!(
			"Expected 2 keys in col11 after second store (no duplicates), found {} - duplicated data may exist!",
			dump.key_count
		);
	}
	tracing::info!("✓ Still only 2 keys in col11 - no duplicate data rows");

	let data_entries = dump.data_entries();
	let data_entry = data_entries
		.first()
		.ok_or_else(|| anyhow!("No data entries found in col11 after second store"))?;
	let content_hash = data_entry.content_hash();
	let refcount = dump
		.get_refcount(content_hash)
		.ok_or_else(|| anyhow!("Could not find refcount for content hash {}", content_hash))?;
	if refcount != 2 {
		anyhow::bail!("Expected refcount=2 after second store, found refcount={}", refcount);
	}
	tracing::info!("✓ Reference count is 2 as expected after second store");

	// === Step 4: Wait for retention period and verify col11 is empty ===
	tracing::info!(
		"=== Step 4: Wait for data expiration ({} blocks) ===",
		LDB_TEST_RETENTION_PERIOD
	);

	// Calculate when both stores should have expired
	// +2 for safety margin
	let expiration_block = second_store_block + LDB_TEST_RETENTION_PERIOD as u64 + 2;

	tracing::info!(
		"Waiting for block {} (second_store_block {} + retention {} + margin 2)",
		expiration_block,
		second_store_block,
		LDB_TEST_RETENTION_PERIOD
	);

	// Must wait for FINALIZED height since block pruning (which triggers col11 cleanup)
	// only happens for finalized blocks: prune_block() is called when finalized_number -
	// blocks_pruning reaches the block containing the data.
	wait_for_finalized_height(collator1, expiration_block, BLOCK_PRODUCTION_TIMEOUT_SECS * 2)
		.await
		.context("Collator did not reach finalized expiration block height")?;

	test_log!(TEST, "=== Verify col11 is empty AFTER retention period ===");
	let dump = verify_col11(&collator_db_path, "col11 AFTER retention period")?;
	if !dump.is_empty() {
		tracing::error!(
			"Expected col11 to be empty after retention period, but found {} keys:",
			dump.key_count
		);
		for entry in &dump.entries {
			tracing::error!("  Key: {}", entry.key);
		}
		anyhow::bail!(
			"Data did not expire after retention period! Found {} keys in col11",
			dump.key_count
		);
	}
	tracing::info!("✓ col11 is empty as expected - data expired after retention period");

	test_log!(TEST, "=== Parachain LDB Storage Verification Test PASSED ===");
	network.destroy().await?;
	Ok(())
}
