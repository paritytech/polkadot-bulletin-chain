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
//! 1. `parachain_fast_sync_test` - Fast sync without block pruning
//!    - Starts 1 collator, stores transaction data
//!    - Adds a regular sync node with --sync=fast
//!    - Verifies state sync completes, bitswap returns DONT_HAVE (expected)
//!
//! 2. `parachain_fast_sync_with_pruning_test` - Fast sync with block pruning
//!    - Starts 1 collator with --blocks-pruning=5, stores transaction data
//!    - Adds a regular sync node with --sync=fast
//!    - Verifies sync FAILS (peers respond with empty blocks - historical blocks pruned)
//!
//! 3. `parachain_warp_sync_test` - Warp sync without block pruning
//!    - Starts 3 relay validators, 1 collator, stores transaction data
//!    - Adds a regular sync node with --sync=warp
//!    - Verifies warp sync completes, bitswap returns DONT_HAVE (expected)
//!
//! 4. `parachain_warp_sync_with_pruning_test` - Warp sync with block pruning
//!    - Starts 3 relay validators, 1 collator with --blocks-pruning=10, stores transaction data
//!    - Adds a regular sync node with --sync=warp
//!    - Verifies warp sync completes, bitswap returns DONT_HAVE (expected)
//!
//! 5. `parachain_full_sync_test` - Full sync without block pruning
//!    - Starts 1 collator, stores transaction data
//!    - Adds a regular sync node with --sync=full
//!    - Verifies full sync completes, bitswap returns data (full sync downloads indexed body)
//!
//! 6. `parachain_full_sync_with_pruning_test` - Full sync with block pruning
//!    - Starts 1 collator with --blocks-pruning=5, stores transaction data
//!    - Adds a regular sync node with --sync=full
//!    - Verifies sync FAILS (peers respond with empty blocks - historical blocks pruned)
//!
//! 7. `parachain_ldb_storage_verification_test` - Database-level verification using rocksdb_ldb
//!    tool
//!    - Verifies col11 state before/after store operations
//!    - Verifies reference counting works correctly (refcount=1 after first store, refcount=2 after
//!      duplicate)
//!    - Verifies data expiration after retention period (col11 becomes empty)
//!
//! ## Key Behavior Notes
//!
//! - **Full sync downloads indexed transactions**: Full sync (`--sync=full`) downloads all blocks
//!   including indexed body, so synced nodes CAN serve historical transaction data via bitswap.
//!
//! - **Warp sync does NOT index transactions**: After warp proof + state sync, the gap fill phase
//!   downloads full block bodies (`HEADER|BODY|JUSTIFICATION`) but does not execute them. Bodies
//!   are stored in the BODY column, not TRANSACTIONS. Indexed data is not available, so warp-synced
//!   nodes return DONT_HAVE via bitswap (same as fast sync).
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
//! - `PARACHAIN_ID`: Parachain ID (default: 2487)
//! - `PARACHAIN_CHAIN_ID`: Chain ID for parachain DB path (default: "bulletin-westend")
//!
//! ## Running Tests
//!
//! ```bash
//! POLKADOT_RELAY_BINARY_PATH=~/local_bulletin_testing/bin/polkadot \
//! POLKADOT_PARACHAIN_BINARY_PATH=~/local_bulletin_testing/bin/polkadot-omni-node \
//! PARACHAIN_CHAIN_SPEC_PATH=./zombienet/bulletin-westend-spec.json \
//!   cargo test -p bulletin-chain-zombienet-sdk-tests \
//!   --features bulletin-chain-zombienet-sdk-tests/zombie-sync-tests \
//!   parachain_fast_sync_test
//! ```

use crate::{
	test_log,
	utils::{
		authorize_and_store_data, authorize_and_store_data_finalized,
		build_parachain_network_config_single_collator,
		build_parachain_network_config_three_relay_validators, content_hash_and_cid,
		expect_bitswap_dont_have, generate_test_data, get_alice_nonce, get_db_path, get_para_id,
		get_parachain_binary_path, get_parachain_chain_id, initialize_network,
		log_line_at_least_once, set_retention_period, set_retention_period_finalized, verify_col11,
		verify_ldb_tool, verify_node_bitswap, verify_parachain_binaries,
		verify_state_sync_completed, verify_warp_sync_completed, wait_for_block_height,
		wait_for_finalized_height, wait_for_fullnode, wait_for_relay_chain_to_sync,
		wait_for_session_change_on_node, BLOCK_PRODUCTION_TIMEOUT_SECS, NETWORK_READY_TIMEOUT_SECS,
		NODE_LOG_CONFIG, PARACHAIN_TEST_DATA_PATTERN, SYNC_TIMEOUT_SECS, TEST_DATA_SIZE,
	},
};
use anyhow::{anyhow, Context, Result};
use env_logger::Env;
use futures::try_join;
use subxt::{config::substrate::SubstrateConfig, OnlineClient};
use zombienet_orchestrator::AddCollatorOptions;

const MIN_BLOCKS_BEFORE_SYNC_NODE: u64 = 10;
/// Session changes are critical for parachain block production.
const SESSION_CHANGE_TIMEOUT_SECS: u64 = 300;
const RETENTION_PERIOD: u32 = 10;

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
async fn parachain_fast_sync_test() -> Result<()> {
	const TEST: &str = "para_fast_sync";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Parachain Fast Sync Test (without pruning) ===");
	log::info!("This test verifies fast sync with 1 collator and a sync node");

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args();
	let config = build_parachain_network_config_single_collator(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - this is when validators get assigned to the parachain
	// and collators can start producing backed blocks
	log::info!(
		"Waiting for relay chain session change (required for parachain block production)..."
	);
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	// Get collator
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Store test data
	let test_data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	// Get initial nonce for Alice
	let nonce = get_alice_nonce(collator1).await?;

	let (store_block, _) = authorize_and_store_data(collator1, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from collator1 via bitswap
	verify_node_bitswap(collator1, &test_data, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	// State sync triggers when: finalized_number + 8 >= network_median
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add a sync node with fast sync
	log::info!("Adding sync-node with --sync=fast");
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
	log::info!("Verifying sync-node's sync progress (target: block {})", target_block);
	wait_for_block_height(sync_node, target_block, SYNC_TIMEOUT_SECS).await?;

	// Verify state sync was used
	verify_state_sync_completed(sync_node).await?;

	// Verify bitswap returns DONT_HAVE from sync-node
	// This is expected because state sync (and block sync with storage_chain_mode: false)
	// does not download indexed transaction data (INDEXED_BODY), so synced nodes
	// cannot serve historical data via bitswap.
	expect_bitswap_dont_have(sync_node, &test_data, 30, "sync-node").await?;
	log::info!("Note: sync-node doesn't have indexed transactions - this is expected for state-synced nodes");

	test_log!(TEST, "=== Parachain Fast Sync Test (without pruning) PASSED ===");
	network.destroy().await?;
	Ok(())
}

const PRUNING_BLOCKS: u32 = 5;

#[tokio::test(flavor = "multi_thread")]
async fn parachain_fast_sync_with_pruning_test() -> Result<()> {
	const TEST: &str = "para_fast_sync_pruning";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Parachain Fast Sync Test (with pruning) ===");
	log::info!(
		"Using 1 collator with --blocks-pruning={}, retention-period={}",
		PRUNING_BLOCKS,
		RETENTION_PERIOD
	);

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(PRUNING_BLOCKS);
	let config = build_parachain_network_config_single_collator(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	log::info!("Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Set retention period and store data (blocks are produced in parallel)
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	let collator1_client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;
	let mut nonce = get_alice_nonce(collator1).await?;

	set_retention_period(&collator1_client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	let test_data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	let (store_block, _) = authorize_and_store_data(collator1, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from collator1 via bitswap
	verify_node_bitswap(collator1, &test_data, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add sync node with fast sync and pruning
	log::info!("Adding sync-node with --sync=fast and --blocks-pruning");
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
	log::info!("Expecting sync-node sync to fail (historical blocks are pruned on peers)");

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
			log::info!("✓ Detected 'BlockResponse with 0 blocks' - peers don't have pruned blocks");
		},
		_ => {
			anyhow::bail!("Expected to detect 'BlockResponse with 0 blocks' in logs, but did not find it within timeout");
		},
	}

	test_log!(TEST, "=== Parachain Fast Sync Test (with pruning) PASSED ===");
	log::info!(
		"Note: This test verifies that sync cannot complete when historical blocks are pruned"
	);
	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_warp_sync_test() -> Result<()> {
	const TEST: &str = "para_warp_sync";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Parachain Warp Sync Test ===");
	log::info!("This test requires 3 relay validators for GRANDPA finality");

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args();
	let config = build_parachain_network_config_three_relay_validators(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	log::info!("Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	// Get collator
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Store test data
	let test_data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	// Get initial nonce for Alice
	let nonce = get_alice_nonce(collator1).await?;

	let (store_block, _) = authorize_and_store_data(collator1, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from collator1 via bitswap
	verify_node_bitswap(collator1, &test_data, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add a sync node with warp sync
	log::info!("Adding sync-node with --sync=warp");
	let para_binary = get_parachain_binary_path();
	let sync_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=warp".into(),
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

	// Wait for the node to be up first
	wait_for_fullnode(sync_node).await?;

	// Wait for the sync node's embedded relay chain to sync.
	// This is critical for warp sync because the parachain warp sync target is determined
	// by querying the embedded relay chain for the finalized parachain head.
	// If the relay chain hasn't synced yet, it returns genesis (#0) as the target,
	// which causes warp sync to get stuck.
	log::info!("Waiting for sync-node's embedded relay chain to sync...");
	wait_for_relay_chain_to_sync(sync_node, SYNC_TIMEOUT_SECS)
		.await
		.context("Sync node's embedded relay chain did not sync")?;
	log::info!("Verifying sync-node's progress (target: block {})", target_block);
	wait_for_block_height(sync_node, target_block, SYNC_TIMEOUT_SECS)
		.await
		.context("Sync node failed to sync via warp sync")?;

	// Verify warp sync completed and node is idle
	verify_warp_sync_completed(sync_node).await?;

	// Warp sync gap fill downloads block bodies but does not execute them.
	// Bodies go to the BODY column, not TRANSACTIONS - so indexed data is not available.
	expect_bitswap_dont_have(sync_node, &test_data, 30, "Sync-node").await?;
	log::info!(
		"Note: Sync-node doesn't have indexed transactions - warp sync gap fill doesn't index data"
	);

	test_log!(TEST, "=== Parachain Warp Sync Test PASSED ===");
	network.destroy().await?;
	Ok(())
}

const WARP_PRUNING_BLOCKS: u32 = 10;

#[tokio::test(flavor = "multi_thread")]
async fn parachain_warp_sync_with_pruning_test() -> Result<()> {
	const TEST: &str = "para_warp_sync_pruning";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Parachain Warp Sync Test (with block pruning) ===");
	log::info!(
		"Collator will use --blocks-pruning={}, retention-period={}",
		WARP_PRUNING_BLOCKS,
		RETENTION_PERIOD
	);

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(WARP_PRUNING_BLOCKS);
	let config = build_parachain_network_config_three_relay_validators(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	log::info!("Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	// Get collator
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Set retention period and store data (blocks are produced in parallel)
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	let collator1_client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;
	let mut nonce = get_alice_nonce(collator1).await?;

	set_retention_period(&collator1_client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// Store transaction data - this will be in an early block that may get pruned
	let test_data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	let (store_block, _) = authorize_and_store_data(collator1, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from collator1 via bitswap
	verify_node_bitswap(collator1, &test_data, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add sync node with warp sync
	log::info!("Adding sync-node with --sync=warp");
	let para_binary = get_parachain_binary_path();
	let sync_node_opts = AddCollatorOptions {
		command: Some(para_binary.as_str().try_into()?),
		args: vec![
			"--sync=warp".into(),
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

	// Wait for the node to be up first
	wait_for_fullnode(sync_node).await?;

	// Wait for the sync node's embedded relay chain to sync.
	// This is critical for warp sync because the parachain warp sync target is determined
	// by querying the embedded relay chain for the finalized parachain head.
	// If the relay chain hasn't synced yet, it returns genesis (#0) as the target,
	// which causes warp sync to get stuck.
	log::info!("Waiting for sync-node's embedded relay chain to sync...");
	wait_for_relay_chain_to_sync(sync_node, SYNC_TIMEOUT_SECS)
		.await
		.context("Sync node's embedded relay chain did not sync")?;

	// Wait for warp sync to complete
	log::info!("Verifying sync-node's progress (target: block {})", target_block);
	wait_for_block_height(sync_node, target_block, SYNC_TIMEOUT_SECS)
		.await
		.context("Sync node failed to sync via warp sync")?;

	// Verify warp sync completed and node is idle
	verify_warp_sync_completed(sync_node).await?;

	// Warp sync gap fill downloads block bodies but does not execute them.
	// Bodies go to the BODY column, not TRANSACTIONS - so indexed data is not available.
	expect_bitswap_dont_have(sync_node, &test_data, 30, "Sync-node").await?;
	log::info!(
		"Note: Sync-node doesn't have indexed transactions - warp sync gap fill doesn't index data"
	);

	test_log!(TEST, "=== Parachain Warp Sync Test (with block pruning) PASSED ===");
	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_full_sync_test() -> Result<()> {
	const TEST: &str = "para_full_sync";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Parachain Full Sync Test (without pruning) ===");
	log::info!("This test verifies full sync with 1 collator and a sync node");

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args();
	let config = build_parachain_network_config_single_collator(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	log::info!(
		"Waiting for relay chain session change (required for parachain block production)..."
	);
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	// Get collator
	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Store test data
	let test_data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	// Get initial nonce for Alice
	let nonce = get_alice_nonce(collator1).await?;

	let (store_block, _) = authorize_and_store_data(collator1, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from collator1 via bitswap
	verify_node_bitswap(collator1, &test_data, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add a sync node with full sync
	log::info!("Adding sync-node with --sync=full");
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
	log::info!("Verifying sync-node's sync progress (target: block {})", target_block);
	wait_for_block_height(sync_node, target_block, SYNC_TIMEOUT_SECS).await?;

	// Verify bitswap returns data from sync-node
	// Full sync downloads all blocks including indexed body, so bitswap should work
	verify_node_bitswap(sync_node, &test_data, 30, "sync-node").await?;
	log::info!("✓ Bitswap works from sync-node - full sync downloads indexed transactions");

	test_log!(TEST, "=== Parachain Full Sync Test (without pruning) PASSED ===");
	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_full_sync_with_pruning_test() -> Result<()> {
	const TEST: &str = "para_full_sync_pruning";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Parachain Full Sync Test (with pruning) ===");
	log::info!(
		"Using 1 collator with --blocks-pruning={}, retention-period={}",
		PRUNING_BLOCKS,
		RETENTION_PERIOD
	);

	// Early validation of required binaries
	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(PRUNING_BLOCKS);
	let config = build_parachain_network_config_single_collator(para_args)?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	log::info!("Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Set retention period and store data
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	let collator1_client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;
	let mut nonce = get_alice_nonce(collator1).await?;

	set_retention_period(&collator1_client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	let test_data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	let (store_block, _) = authorize_and_store_data(collator1, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from collator1 via bitswap
	verify_node_bitswap(collator1, &test_data, 30, "Collator-1").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(collator1, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add sync node with full sync and pruning
	log::info!("Adding sync-node with --sync=full and --blocks-pruning");
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
	log::info!("Expecting sync-node sync to fail (historical blocks are pruned on peers)");

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
			log::info!("✓ Detected 'BlockResponse with 0 blocks' - peers don't have pruned blocks");
		},
		_ => {
			anyhow::bail!("Expected to detect 'BlockResponse with 0 blocks' in logs, but did not find it within timeout");
		},
	}

	test_log!(TEST, "=== Parachain Full Sync Test (with pruning) PASSED ===");
	log::info!(
		"Note: This test verifies that sync cannot complete when historical blocks are pruned"
	);
	network.destroy().await?;
	Ok(())
}

/// Long enough for both stores before expiration (~6 blocks each on parachain).
const LDB_TEST_RETENTION_PERIOD: u32 = 20;

#[tokio::test(flavor = "multi_thread")]
async fn parachain_ldb_storage_verification_test() -> Result<()> {
	const TEST: &str = "para_ldb_storage";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Parachain LDB Storage Verification Test ===");
	log::info!("This test verifies transaction storage database behavior using rocksdb_ldb tool");
	log::info!(
		"Using --blocks-pruning={} and retention-period={}",
		LDB_TEST_RETENTION_PERIOD,
		LDB_TEST_RETENTION_PERIOD
	);

	// === Early validation of required external tools ===
	log::info!("=== Verifying required external tools ===");
	verify_ldb_tool()?;
	verify_parachain_binaries()?;

	let para_args = vec![
		"--ipfs-server".into(),
		format!("--blocks-pruning={}", LDB_TEST_RETENTION_PERIOD),
		"-ltransaction-storage=trace".into(),
		"--".into(),
		"--network-backend=libp2p".into(),
	];
	let config = build_parachain_network_config_single_collator(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	// Get relay chain validator for session change detection
	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;

	// Wait for first session change - required for parachain block production
	log::info!(
		"Waiting for relay chain session change (required for parachain block production)..."
	);
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;

	// Wait for parachain to start producing blocks after session change
	log::info!("Waiting for parachain to produce blocks...");
	wait_for_block_height(collator1, 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// Get client and fetch initial nonce for Alice
	let collator1_client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;
	let mut nonce = get_alice_nonce(collator1).await?;

	// Set short retention period for fast testing
	// NOTE: LDB test uses finalized transactions for database consistency
	log::info!(
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
	log::info!("Collator-1 DB path: {:?}", collator_db_path);

	// === Step 1: Verify col11 is empty before store ===
	test_log!(TEST, "=== Step 1: Verify col11 is empty BEFORE store ===");
	let dump = verify_col11(&collator_db_path, "col11 BEFORE store")?;
	if !dump.is_empty() {
		anyhow::bail!("Expected col11 to be empty before store, but found {} keys", dump.key_count);
	}
	log::info!("✓ col11 is empty as expected before store");

	// Generate test data and calculate expected content hash
	let test_data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (expected_hash, expected_cid) = content_hash_and_cid(&test_data);
	log::info!("Generated {} bytes of test data", test_data.len());
	log::info!("Expected content hash: {}", expected_hash);
	log::info!("Expected CID: {}", expected_cid);

	// === Step 2: First store - verify refcount = 1 and content hash matches ===
	test_log!(TEST, "=== Step 2: First store - expecting refcount = 1 ===");
	let (first_store_block, next_nonce) =
		authorize_and_store_data_finalized(collator1, &test_data, nonce).await?;
	nonce = next_nonce;
	log::info!("First store completed at block {}", first_store_block);

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
	log::info!("✓ Content hash matches: {}", stored_hash);

	let refcount = dump
		.get_refcount(stored_hash)
		.ok_or_else(|| anyhow!("Could not find refcount for content hash {}", stored_hash))?;
	if refcount != 1 {
		anyhow::bail!("Expected refcount=1 after first store, found refcount={}", refcount);
	}
	log::info!("✓ Reference count is 1 as expected after first store");

	// === Step 3: Second store (same data) - verify refcount = 2, still 2 keys ===
	test_log!(TEST, "=== Step 3: Second store - expecting refcount = 2, still 2 keys ===");
	let (second_store_block, _) =
		authorize_and_store_data_finalized(collator1, &test_data, nonce).await?;
	log::info!("Second store completed at block {}", second_store_block);

	let dump = verify_col11(&collator_db_path, "col11 AFTER second store")?;
	if dump.key_count != 2 {
		anyhow::bail!(
			"Expected 2 keys in col11 after second store (no duplicates), found {} - duplicated data may exist!",
			dump.key_count
		);
	}
	log::info!("✓ Still only 2 keys in col11 - no duplicate data rows");

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
	log::info!("✓ Reference count is 2 as expected after second store");

	// === Step 4: Wait for retention period and verify col11 is empty ===
	log::info!("=== Step 4: Wait for data expiration ({} blocks) ===", LDB_TEST_RETENTION_PERIOD);

	// Calculate when both stores should have expired
	let expiration_block = second_store_block + LDB_TEST_RETENTION_PERIOD as u64 + 2; // +2 for safety margin

	log::info!(
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
		log::error!(
			"Expected col11 to be empty after retention period, but found {} keys:",
			dump.key_count
		);
		for entry in &dump.entries {
			log::error!("  Key: {}", entry.key);
		}
		anyhow::bail!(
			"Data did not expire after retention period! Found {} keys in col11",
			dump.key_count
		);
	}
	log::info!("✓ col11 is empty as expected - data expired after retention period");

	test_log!(TEST, "=== Parachain LDB Storage Verification Test PASSED ===");
	network.destroy().await?;
	Ok(())
}
