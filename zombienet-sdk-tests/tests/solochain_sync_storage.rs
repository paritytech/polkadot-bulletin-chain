// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Solo chain sync tests for bulletin-polkadot
//!
//! This module contains sync tests that run the bulletin chain as a solo chain.
//! These tests verify sync behavior and transaction storage (bitswap) functionality.
//!
//! ## Tests
//!
//! 1. `fast_sync_test` - Fast sync without block pruning
//!    - Starts Alice (validator), stores transaction data
//!    - Adds Bob with --sync=fast
//!    - Verifies state sync completes, bitswap returns DONT_HAVE (expected)
//!
//! 2. `fast_sync_with_pruning_test` - Fast sync with block pruning
//!    - Starts Alice with --blocks-pruning=5, stores transaction data
//!    - Adds Bob with --sync=fast
//!    - Verifies sync FAILS (peers respond with empty blocks - historical blocks pruned)
//!
//! 3. `warp_sync_test` - Warp sync without block pruning
//!    - Starts 3 validators (Alice, Bob, Dave) for GRANDPA finality
//!    - Stores transaction data, adds Charlie with --sync=warp
//!    - Verifies warp sync completes, bitswap returns DONT_HAVE (expected)
//!
//! 4. `warp_sync_with_pruning_test` - Warp sync with block pruning
//!    - Starts 3 validators with --blocks-pruning, stores data
//!    - Adds Charlie with --sync=warp
//!    - Verifies warp sync completes, bitswap returns DONT_HAVE (expected)
//!
//! 5. `full_sync_test` - Full sync without block pruning
//!    - Starts Alice, stores transaction data
//!    - Adds Bob with --sync=full
//!    - Verifies sync completes, bitswap returns data (full sync downloads indexed body)
//!
//! 6. `full_sync_with_pruning_test` - Full sync with block pruning
//!    - Starts Alice with --blocks-pruning=5, stores data
//!    - Adds Bob with --sync=full
//!    - Verifies sync FAILS (peers respond with empty blocks)
//!
//! 7. `ldb_storage_verification_test` - Database-level verification using rocksdb_ldb tool
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
//! - **Pruning prevents sync**: When all peers have pruning enabled, fast/full sync cannot complete
//!   because historical blocks are unavailable for gap filling.
//!
//! ## Environment Variables
//!
//! - `POLKADOT_BULLETIN_BINARY_PATH`: Path to the solo chain binary (required)
//! - `ROCKSDB_LDB_PATH`: Path to rocksdb_ldb tool (required for LDB test)
//!
//! ## Running Tests
//!
//! ```bash
//! POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain \
//!   cargo test -p bulletin-chain-zombienet-sdk-tests \
//!   --features bulletin-chain-zombienet-sdk-tests/zombie-sync-tests \
//!   solochain_sync_storage -- --nocapture
//! ```

use crate::{
	test_log,
	utils::{
		authorize_and_store_data, build_single_node_network_config,
		build_three_node_network_config, content_hash_and_cid, expect_bitswap_dont_have,
		generate_test_data, get_alice_nonce, get_db_path, initialize_network,
		log_line_at_least_once, set_retention_period, verify_col11, verify_ldb_tool,
		verify_node_bitswap, verify_solo_binary, verify_state_sync_completed,
		verify_warp_sync_completed, wait_for_block_height, wait_for_finalized_height,
		wait_for_fullnode, wait_for_validator, BEST_BLOCK_METRIC, BLOCK_PRODUCTION_TIMEOUT_SECS,
		CHAIN_ID, NETWORK_READY_TIMEOUT_SECS, NODE_LOG_CONFIG, SOLO_TEST_DATA_PATTERN,
		SYNC_TIMEOUT_SECS, TEST_DATA_SIZE,
	},
};
use anyhow::{anyhow, Context, Result};
use env_logger::Env;
use futures::try_join;
use subxt::{config::substrate::SubstrateConfig, OnlineClient};
use zombienet_sdk::AddNodeOptions;

const MIN_BLOCKS_BEFORE_SYNC_NODE: u64 = 10;
const MIN_BLOCKS_FOR_INIT: u64 = 5;
const RETENTION_PERIOD: u32 = 10;
const PRUNING_BLOCKS: u32 = 5;
const LDB_TEST_RETENTION_PERIOD: u32 = 3;

async fn wait_for_alice_ready(alice: &zombienet_sdk::NetworkNode) -> Result<()> {
	wait_for_validator(alice).await?;
	wait_for_block_height(alice, MIN_BLOCKS_FOR_INIT, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await
		.context("Alice did not produce initial blocks")?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ldb_storage_verification_test() -> Result<()> {
	const TEST: &str = "ldb_storage_verification";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== LDB Storage Verification Test ===");
	test_log!(
		TEST,
		"This test verifies transaction storage database behavior using rocksdb_ldb tool"
	);
	test_log!(
		TEST,
		"Using --blocks-pruning={} and retention-period={}",
		LDB_TEST_RETENTION_PERIOD,
		LDB_TEST_RETENTION_PERIOD
	);

	// Early validation of required tools
	verify_ldb_tool()?;
	verify_solo_binary()?;

	let pruning_arg = format!("--blocks-pruning={}", LDB_TEST_RETENTION_PERIOD);
	let config = build_single_node_network_config(vec![
		"--ipfs-server".into(),
		pruning_arg,
		"-ltransaction-storage=trace".into(),
	])?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let alice = network.get_node("alice").context("Failed to get alice node")?;
	wait_for_alice_ready(alice).await?;

	// Get client and fetch initial nonce for Alice
	let alice_client: OnlineClient<SubstrateConfig> = alice.wait_client().await?;
	let mut nonce = get_alice_nonce(alice).await?;

	// Set short retention period for fast testing
	log::info!(
		"Setting RetentionPeriod to {} blocks for fast expiration testing",
		LDB_TEST_RETENTION_PERIOD
	);
	set_retention_period(&alice_client, LDB_TEST_RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// Get database path
	let base_dir = network
		.base_dir()
		.ok_or_else(|| anyhow!("Failed to get network base directory"))?
		.to_string();
	let alice_db_path = get_db_path(&base_dir, "alice", CHAIN_ID);
	log::info!("Alice DB path: {:?}", alice_db_path);

	// === Step 1: Verify col11 is empty before store ===
	test_log!(TEST, "=== Step 1: Verify col11 is empty BEFORE store ===");
	let dump = verify_col11(&alice_db_path, "col11 BEFORE store")?;
	if !dump.is_empty() {
		anyhow::bail!("Expected col11 to be empty before store, but found {} keys", dump.key_count);
	}
	log::info!("✓ col11 is empty as expected before store");

	// Generate test data and calculate expected content hash
	let test_data = generate_test_data(TEST_DATA_SIZE, SOLO_TEST_DATA_PATTERN);
	let (expected_hash, expected_cid) = content_hash_and_cid(&test_data);
	log::info!("Generated {} bytes of test data", test_data.len());
	log::info!("Expected content hash: {}", expected_hash);
	log::info!("Expected CID: {}", expected_cid);

	// === Step 2: First store - verify refcount = 1 and content hash matches ===
	test_log!(TEST, "=== Step 2: First store - expecting refcount = 1 ===");
	let (first_store_block, next_nonce) =
		authorize_and_store_data(alice, &test_data, nonce).await?;
	nonce = next_nonce;
	log::info!("First store completed at block {}", first_store_block);

	let dump = verify_col11(&alice_db_path, "col11 AFTER first store")?;
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
	let (second_store_block, _) = authorize_and_store_data(alice, &test_data, nonce).await?;
	log::info!("Second store completed at block {}", second_store_block);

	let dump = verify_col11(&alice_db_path, "col11 AFTER second store")?;
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
	test_log!(
		TEST,
		"=== Step 4: Wait for data expiration ({} blocks) ===",
		LDB_TEST_RETENTION_PERIOD
	);

	// Calculate when both stores should have expired
	// First store expires at: first_store_block + retention_period
	// Second store expires at: second_store_block + retention_period
	// We need to wait for the later one
	let expiration_block = second_store_block + LDB_TEST_RETENTION_PERIOD as u64 + 2; // +2 for safety margin

	log::info!(
		"Waiting for block {} (second_store_block {} + retention {} + margin 2)",
		expiration_block,
		second_store_block,
		LDB_TEST_RETENTION_PERIOD
	);

	alice
		.wait_metric_with_timeout(
			BEST_BLOCK_METRIC,
			|height| height >= expiration_block as f64,
			BLOCK_PRODUCTION_TIMEOUT_SECS,
		)
		.await
		.context("Alice did not reach expiration block height")?;

	test_log!(TEST, "=== Verify col11 is empty AFTER retention period ===");
	let dump = verify_col11(&alice_db_path, "col11 AFTER retention period")?;
	if !dump.is_empty() {
		log::error!(
			"Expected col11 to be empty after retention period, but found {} keys:",
			dump.key_count
		);
		for entry in &dump.entries {
			if entry.is_refcount() {
				log::error!(
					"  Unexpected refcount entry: {} => {} (refcount={})",
					entry.key,
					entry.value,
					entry.parse_refcount().unwrap_or(0)
				);
			} else {
				log::error!("  Unexpected data entry: {} => ...", entry.key);
			}
		}
		anyhow::bail!(
			"Expected col11 to be empty after retention period, but found {} keys",
			dump.key_count
		);
	}
	log::info!("✓ col11 is empty as expected - all data expired after retention period");

	test_log!(TEST, "=== LDB Storage Verification Test PASSED ===");
	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fast_sync_test() -> Result<()> {
	const TEST: &str = "fast_sync";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Fast Sync Test (without pruning) ===");

	// Early validation of required binaries
	verify_solo_binary()?;

	let config =
		build_single_node_network_config(vec!["--ipfs-server".into(), NODE_LOG_CONFIG.into()])?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let alice = network.get_node("alice").context("Failed to get alice node")?;
	wait_for_alice_ready(alice).await?;

	// Store data and calculate content hash
	let test_data = generate_test_data(TEST_DATA_SIZE, SOLO_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	// Get initial nonce for Alice
	let nonce = get_alice_nonce(alice).await?;

	let (store_block, _) = authorize_and_store_data(alice, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from Alice via bitswap
	verify_node_bitswap(alice, &test_data, 30, "Alice").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add Bob with fast sync
	log::info!("Adding Bob with --sync=fast");
	let bob_opts = AddNodeOptions {
		args: vec!["--sync=fast".into(), "--ipfs-server".into(), NODE_LOG_CONFIG.into()],
		is_validator: false,
		..Default::default()
	};

	network.add_node("bob", bob_opts).await?;
	let bob = network.get_node("bob").context("Failed to get bob node")?;

	// Wait for Bob to sync
	wait_for_fullnode(bob).await?;
	log::info!("Verifying Bob's sync progress (target: block {})", target_block);
	wait_for_block_height(bob, target_block, SYNC_TIMEOUT_SECS).await?;

	// Verify state sync was used
	verify_state_sync_completed(bob).await?;

	// Verify bitswap returns DONT_HAVE from Bob
	// This is expected because state sync does not download indexed transaction data
	expect_bitswap_dont_have(bob, &test_data, 30, "Bob").await?;
	log::info!("Note: Bob doesn't have indexed transactions - expected for state-synced nodes");

	test_log!(TEST, "=== Fast Sync Test (without pruning) PASSED ===");
	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fast_sync_with_pruning_test() -> Result<()> {
	const TEST: &str = "fast_sync_pruning";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Fast Sync Test (with pruning) ===");
	log::info!("Using --blocks-pruning={}, retention-period={}", PRUNING_BLOCKS, RETENTION_PERIOD);

	// Early validation of required binaries
	verify_solo_binary()?;

	let pruning_arg = format!("--blocks-pruning={}", PRUNING_BLOCKS);
	let config = build_single_node_network_config(vec![
		"--ipfs-server".into(),
		pruning_arg.clone(),
		NODE_LOG_CONFIG.into(),
	])?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let alice = network.get_node("alice").context("Failed to get alice node")?;
	wait_for_alice_ready(alice).await?;

	// Set retention period
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	let alice_client: OnlineClient<SubstrateConfig> = alice.wait_client().await?;
	let mut nonce = get_alice_nonce(alice).await?;

	set_retention_period(&alice_client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// Store data
	let test_data = generate_test_data(TEST_DATA_SIZE, SOLO_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	let (store_block, _) = authorize_and_store_data(alice, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from Alice via bitswap
	verify_node_bitswap(alice, &test_data, 30, "Alice").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add Bob with fast sync and pruning
	log::info!("Adding Bob with --sync=fast and --blocks-pruning");
	let bob_opts = AddNodeOptions {
		args: vec![
			"--sync=fast".into(),
			"--ipfs-server".into(),
			pruning_arg.as_str().into(),
			NODE_LOG_CONFIG.into(),
		],
		is_validator: false,
		..Default::default()
	};

	network.add_node("bob", bob_opts).await?;
	let bob = network.get_node("bob").context("Failed to get bob node")?;

	// Wait for Bob's node to be up before checking logs (the log file is created
	// asynchronously and wait_log_line_count_with_timeout errors immediately if
	// the file doesn't exist yet).
	wait_for_fullnode(bob).await?;

	// With pruning enabled on all nodes, historical blocks are unavailable.
	// Bob cannot sync because blocks 1-N are pruned on peers.
	// We expect to see "BlockResponse ... with 0 blocks" - peers don't have the blocks.
	log::info!("Expecting Bob's sync to fail (historical blocks are pruned on peers)");

	// Wait for the telltale sign: peers responding with 0 blocks (they don't have them)
	let zero_blocks_response = bob
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

	test_log!(TEST, "=== Fast Sync Test (with pruning) PASSED ===");
	log::info!(
		"Note: This test verifies that sync cannot complete when historical blocks are pruned"
	);
	network.destroy().await?;
	Ok(())
}

const WARP_SYNC_MIN_FINALIZED_BLOCKS: u64 = 5;

#[tokio::test(flavor = "multi_thread")]
async fn warp_sync_test() -> Result<()> {
	const TEST: &str = "warp_sync";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Warp Sync Test ===");
	log::info!("This test requires 3 validators for GRANDPA finality");

	// Early validation of required binaries
	verify_solo_binary()?;

	// Build network with three validators for GRANDPA finality and warp sync peer requirement
	let node_args = vec!["--ipfs-server".to_string(), NODE_LOG_CONFIG.to_string()];
	let config = build_three_node_network_config(node_args)?;

	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let alice = network.get_node("alice").context("Failed to get alice node")?;
	let bob = network.get_node("bob").context("Failed to get bob node")?;
	let dave = network.get_node("dave").context("Failed to get dave node")?;

	// Wait for all validators to be ready (in parallel)
	try_join!(wait_for_validator(alice), wait_for_validator(bob), wait_for_validator(dave),)?;
	log::info!("All validators (Alice, Bob, Dave) are ready");

	// Wait for GRANDPA finality - critical for warp sync
	log::info!(
		"Waiting for GRANDPA finality (min {} finalized blocks)",
		WARP_SYNC_MIN_FINALIZED_BLOCKS
	);
	wait_for_finalized_height(alice, WARP_SYNC_MIN_FINALIZED_BLOCKS, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await
		.context("GRANDPA finality not achieved - warp sync requires finalized blocks")?;
	log::info!("GRANDPA finality achieved");

	// Store data
	let test_data = generate_test_data(TEST_DATA_SIZE, SOLO_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	// Get initial nonce for Alice
	let nonce = get_alice_nonce(alice).await?;

	let (store_block, _) = authorize_and_store_data(alice, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from Alice via bitswap
	verify_node_bitswap(alice, &test_data, 30, "Alice").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add Charlie with warp sync
	log::info!("Adding Charlie with --sync=warp");
	let charlie_opts = AddNodeOptions {
		args: vec!["--sync=warp".into(), "--ipfs-server".into(), NODE_LOG_CONFIG.into()],
		is_validator: false,
		..Default::default()
	};

	network.add_node("charlie", charlie_opts).await?;
	let charlie = network.get_node("charlie").context("Failed to get charlie node")?;

	// Wait for Charlie to sync
	wait_for_fullnode(charlie).await?;
	log::info!("Verifying Charlie's sync progress (target: block {})", target_block);
	wait_for_block_height(charlie, target_block, SYNC_TIMEOUT_SECS)
		.await
		.context("Charlie failed to sync via warp sync")?;

	// Verify warp sync completed
	verify_warp_sync_completed(charlie).await?;

	// Warp sync gap fill downloads block bodies but does not execute them.
	// Bodies go to the BODY column, not TRANSACTIONS - so indexed data is not available.
	expect_bitswap_dont_have(charlie, &test_data, 30, "Charlie").await?;
	log::info!(
		"Note: Charlie doesn't have indexed transactions - warp sync gap fill doesn't index data"
	);

	test_log!(TEST, "=== Warp Sync Test PASSED ===");
	network.destroy().await?;
	Ok(())
}

const WARP_PRUNING_BLOCKS: u32 = 10;

#[tokio::test(flavor = "multi_thread")]
async fn warp_sync_with_pruning_test() -> Result<()> {
	const TEST: &str = "warp_sync_pruning";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Warp Sync Test (with block pruning) ===");
	log::info!(
		"Validators will use --blocks-pruning={}, retention-period={}",
		WARP_PRUNING_BLOCKS,
		RETENTION_PERIOD
	);

	// Early validation of required binaries
	verify_solo_binary()?;

	// Build network with three validators, all with block pruning enabled
	let pruning_arg = format!("--blocks-pruning={}", WARP_PRUNING_BLOCKS);
	let node_args = vec!["--ipfs-server".to_string(), pruning_arg, NODE_LOG_CONFIG.to_string()];
	let config = build_three_node_network_config(node_args)?;

	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let alice = network.get_node("alice").context("Failed to get alice node")?;
	let bob = network.get_node("bob").context("Failed to get bob node")?;
	let dave = network.get_node("dave").context("Failed to get dave node")?;

	// Wait for all validators to be ready (in parallel)
	try_join!(wait_for_validator(alice), wait_for_validator(bob), wait_for_validator(dave),)?;
	log::info!("All validators (Alice, Bob, Dave) are ready with pruning enabled");

	// Wait for GRANDPA finality - critical for warp sync
	log::info!(
		"Waiting for GRANDPA finality (min {} finalized blocks)",
		WARP_SYNC_MIN_FINALIZED_BLOCKS
	);
	wait_for_finalized_height(alice, WARP_SYNC_MIN_FINALIZED_BLOCKS, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await
		.context("GRANDPA finality not achieved")?;
	log::info!("GRANDPA finality achieved");

	// Set retention period
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	let alice_client: OnlineClient<SubstrateConfig> = alice.wait_client().await?;
	let mut nonce = get_alice_nonce(alice).await?;

	set_retention_period(&alice_client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// Store data
	let test_data = generate_test_data(TEST_DATA_SIZE, SOLO_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	let (store_block, _) = authorize_and_store_data(alice, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from Alice via bitswap
	verify_node_bitswap(alice, &test_data, 30, "Alice").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add Charlie with warp sync
	log::info!("Adding Charlie with --sync=warp");
	let charlie_opts = AddNodeOptions {
		args: vec!["--sync=warp".into(), "--ipfs-server".into(), NODE_LOG_CONFIG.into()],
		is_validator: false,
		..Default::default()
	};

	network.add_node("charlie", charlie_opts).await?;
	let charlie = network.get_node("charlie").context("Failed to get charlie node")?;

	// Wait for Charlie to sync
	wait_for_fullnode(charlie).await?;
	log::info!("Verifying Charlie's sync progress (target: block {})", target_block);
	wait_for_block_height(charlie, target_block, SYNC_TIMEOUT_SECS)
		.await
		.context("Charlie failed to sync via warp sync")?;

	// Verify warp sync completed
	verify_warp_sync_completed(charlie).await?;

	// Warp sync gap fill downloads block bodies but does not execute them.
	// Bodies go to the BODY column, not TRANSACTIONS - so indexed data is not available.
	expect_bitswap_dont_have(charlie, &test_data, 30, "Charlie").await?;
	log::info!(
		"Note: Charlie doesn't have indexed transactions - warp sync gap fill doesn't index data"
	);

	test_log!(TEST, "=== Warp Sync Test (with block pruning) PASSED ===");
	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn full_sync_test() -> Result<()> {
	const TEST: &str = "full_sync";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Full Sync Test (without pruning) ===");

	// Early validation of required binaries
	verify_solo_binary()?;

	let config =
		build_single_node_network_config(vec!["--ipfs-server".into(), NODE_LOG_CONFIG.into()])?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let alice = network.get_node("alice").context("Failed to get alice node")?;
	wait_for_alice_ready(alice).await?;

	// Store data
	let test_data = generate_test_data(TEST_DATA_SIZE, SOLO_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	// Get initial nonce for Alice
	let nonce = get_alice_nonce(alice).await?;

	let (store_block, _) = authorize_and_store_data(alice, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from Alice via bitswap
	verify_node_bitswap(alice, &test_data, 30, "Alice").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add Bob with full sync
	log::info!("Adding Bob with --sync=full");
	let bob_opts = AddNodeOptions {
		args: vec!["--sync=full".into(), "--ipfs-server".into(), NODE_LOG_CONFIG.into()],
		is_validator: false,
		..Default::default()
	};

	network.add_node("bob", bob_opts).await?;
	let bob = network.get_node("bob").context("Failed to get bob node")?;

	// Wait for Bob to sync
	wait_for_fullnode(bob).await?;
	log::info!("Verifying Bob's sync progress (target: block {})", target_block);
	wait_for_block_height(bob, target_block, SYNC_TIMEOUT_SECS).await?;

	// Verify bitswap returns data from Bob
	// Full sync downloads all blocks including indexed body, so bitswap should work
	verify_node_bitswap(bob, &test_data, 30, "Bob").await?;
	log::info!("✓ Bitswap works from Bob - full sync downloads indexed transactions");

	test_log!(TEST, "=== Full Sync Test (without pruning) PASSED ===");
	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn full_sync_with_pruning_test() -> Result<()> {
	const TEST: &str = "full_sync_pruning";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Full Sync Test (with pruning) ===");
	log::info!("Using --blocks-pruning={}, retention-period={}", PRUNING_BLOCKS, RETENTION_PERIOD);

	// Early validation of required binaries
	verify_solo_binary()?;

	let pruning_arg = format!("--blocks-pruning={}", PRUNING_BLOCKS);
	let config = build_single_node_network_config(vec![
		"--ipfs-server".into(),
		pruning_arg.clone(),
		NODE_LOG_CONFIG.into(),
	])?;
	let mut network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let alice = network.get_node("alice").context("Failed to get alice node")?;
	wait_for_alice_ready(alice).await?;

	// Set retention period
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	let alice_client: OnlineClient<SubstrateConfig> = alice.wait_client().await?;
	let mut nonce = get_alice_nonce(alice).await?;

	set_retention_period(&alice_client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// Store data
	let test_data = generate_test_data(TEST_DATA_SIZE, SOLO_TEST_DATA_PATTERN);
	let (content_hash, cid) = content_hash_and_cid(&test_data);
	log::info!("Storing {} bytes of test data", test_data.len());
	log::info!("Content hash: {}, CID: {}", content_hash, cid);

	let (store_block, _) = authorize_and_store_data(alice, &test_data, nonce).await?;
	log::info!("Store completed at block {}", store_block);

	// Verify data can be fetched from Alice via bitswap
	verify_node_bitswap(alice, &test_data, 30, "Alice").await?;

	// Wait for enough blocks and finality before adding sync node
	let target_block = std::cmp::max(store_block, MIN_BLOCKS_BEFORE_SYNC_NODE);
	log::info!("Waiting for block {} and finality", target_block);
	try_join!(
		wait_for_block_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
		wait_for_finalized_height(alice, target_block, BLOCK_PRODUCTION_TIMEOUT_SECS),
	)?;

	// Add Bob with full sync and pruning
	log::info!("Adding Bob with --sync=full and --blocks-pruning");
	let bob_opts = AddNodeOptions {
		args: vec![
			"--sync=full".into(),
			"--ipfs-server".into(),
			pruning_arg.as_str().into(),
			NODE_LOG_CONFIG.into(),
		],
		is_validator: false,
		..Default::default()
	};

	network.add_node("bob", bob_opts).await?;
	let bob = network.get_node("bob").context("Failed to get bob node")?;

	// Wait for Bob's node to be up before checking logs (the log file is created
	// asynchronously and wait_log_line_count_with_timeout errors immediately if
	// the file doesn't exist yet).
	wait_for_fullnode(bob).await?;

	// With pruning enabled on all nodes, historical blocks are unavailable.
	// Bob cannot sync because blocks 1-N are pruned on peers.
	// We expect to see "BlockResponse ... with 0 blocks" - peers don't have the blocks.
	log::info!("Expecting Bob's sync to fail (historical blocks are pruned on peers)");

	// Wait for the telltale sign: peers responding with 0 blocks (they don't have them)
	let zero_blocks_response = bob
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

	test_log!(TEST, "=== Full Sync Test (with pruning) PASSED ===");
	log::info!(
		"Note: This test verifies that sync cannot complete when historical blocks are pruned"
	);
	network.destroy().await?;
	Ok(())
}
