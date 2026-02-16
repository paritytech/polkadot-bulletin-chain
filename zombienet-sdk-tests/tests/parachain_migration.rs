// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Parachain TransactionInfo migration test with stall recovery via code_substitutes.
//!
//! Simulates the real incident where a bulletin-westend runtime was upgraded without
//! the TransactionInfo v0→v1 migration, causing `check_proof` to fail on old entries
//! and stalling block production. Then recovers the stalled chain using:
//!
//! 1. `force_set_current_code` on the relay chain (validators use fix runtime)
//! 2. `codeSubstitutes` in the collator's chain spec (client uses fix runtime)
//! 3. Collator restart (picks up modified chain spec)
//!
//! # Recovery mechanism: code_substitutes
//!
//! The cumulus collator does NOT automatically sync validation code from the relay
//! chain. When `force_set_current_code` is called on the relay chain, the collator
//! only uses the code from its own `:code` storage (still the broken runtime).
//!
//! `codeSubstitutes` in the chain spec tells the substrate client to use alternative
//! WASM code at specific block numbers. The client reads these during initialization
//! and substitutes them for the on-chain `:code` when executing blocks at or after
//! the specified block number.
//!
//! Combined with `force_set_current_code` (relay validators accept the fix code),
//! this allows the collator to produce valid blocks using the fix runtime.
//!
//! # Runtime WASM blobs required
//!
//! Four pre-built runtime WASMs must be available (set via env vars or use defaults):
//!
//! | WASM | TransactionInfo | Migration | Description |
//! |------|-----------------|-----------|-------------|
//! | old_runtime | v0 | None | Chain starts with this |
//! | broken_runtime | v1 | **None** | Causes stall |
//! | fix_runtime | v1 | v0→v1 in `on_initialize` | Recovers chain via `codeSubstitutes` |
//! | next_runtime | v1 | None (un-wired) | Normal upgrade after recovery |
//!
//! # Test flow
//!
//! 1. Start parachain with old runtime (v0 TransactionInfo)
//! 2. Set a short retention period (30 blocks)
//! 3. Authorize and store data (creates v0 entries at block ~7)
//! 4. Upgrade to broken runtime (v1 struct, no migration)
//! 5. Store more data (v1 entries — works fine)
//! 6. Wait for chain stall (check_proof fails decoding v0 entries as v1)
//! 7. Recovery: force_set_current_code on relay + code_substitutes + collator restart
//! 8. Verify chain resumes block production
//! 9. Store new data to verify chain is fully functional
//! 10. Normal on-chain upgrade to next runtime (bumped spec_version, migration un-wired)
//! 11. Store data to verify normal upgrades work post-recovery

use crate::{test_log, utils::*};
use std::time::Duration;

const TEST: &str = "parachain_migration";

#[tokio::test(flavor = "multi_thread")]
async fn parachain_migration_recovery_test() -> Result<(), anyhow::Error> {
	let _ = env_logger::builder()
		.is_test(true)
		.filter_level(log::LevelFilter::Info)
		.try_init();

	test_log!(TEST, "=== Starting parachain migration test ===");

	// --- Verify prerequisites ---
	verify_parachain_binaries()?;
	verify_wasm_files()?;

	let _old_wasm = get_wasm_path(OLD_RUNTIME_WASM_ENV, DEFAULT_OLD_RUNTIME_WASM);
	let broken_wasm = get_wasm_path(BROKEN_RUNTIME_WASM_ENV, DEFAULT_BROKEN_RUNTIME_WASM);
	let fix_wasm = get_wasm_path(FIX_RUNTIME_WASM_ENV, DEFAULT_FIX_RUNTIME_WASM);
	let next_wasm = get_wasm_path(NEXT_RUNTIME_WASM_ENV, DEFAULT_NEXT_RUNTIME_WASM);

	// --- Build and start network ---
	test_log!(TEST, "Building parachain network configuration...");
	let config = build_parachain_network_config_single_collator(vec![
		"-lruntime=trace".to_string(),
		"-ltransaction-storage=trace".to_string(),
		"-lcode-provider=trace".to_string(),
		"--rpc-max-request-size=100".to_string(),
		"--rpc-max-response-size=100".to_string(),
	])?;

	test_log!(TEST, "Spawning network...");
	let network = initialize_network(config).await?;

	// --- Wait for relay chain session change ---
	let relay_alice = network
		.get_node("alice")
		.map_err(|e| anyhow::anyhow!("Failed to get relay alice node: {}", e))?;

	test_log!(TEST, "Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, NETWORK_READY_TIMEOUT_SECS).await?;

	// --- Wait for parachain block production ---
	let collator = network
		.get_node("collator-1")
		.map_err(|e| anyhow::anyhow!("Failed to get collator node: {}", e))?;

	test_log!(TEST, "Waiting for parachain to produce blocks...");
	wait_for_block_height(collator, 3, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	test_log!(TEST, "Parachain is producing blocks");

	// --- Set short retention period ---
	let client: subxt::OnlineClient<BulletinConfig> = collator.wait_client().await?;
	let mut nonce = get_alice_nonce(collator).await?;

	test_log!(TEST, "Setting retention period to {} blocks...", TEST_RETENTION_PERIOD);
	set_retention_period(&client, TEST_RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// --- Store data with old runtime (creates v0 TransactionInfo entries) ---
	let data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	test_log!(TEST, "Storing {} bytes of test data (v0 format)...", data.len());
	let (store_block, new_nonce) = authorize_and_store_data(collator, &data, nonce).await?;
	nonce = new_nonce;
	test_log!(TEST, "Data stored at block {} (v0 TransactionInfo)", store_block);

	// --- Upgrade to broken runtime (v1 struct, no migration) ---
	test_log!(TEST, "=== Upgrading to BROKEN runtime (no migration) ===");
	do_parachain_runtime_upgrade(collator, &broken_wasm, &mut nonce).await?;
	test_log!(TEST, "Broken runtime upgrade complete");

	// --- Wait for upgrade to stabilize and store more data ---
	test_log!(TEST, "Waiting for new blocks after broken upgrade...");
	let pre_upgrade_height = collator
		.reports(BEST_BLOCK_METRIC)
		.await
		.map_err(|e| anyhow::anyhow!("Failed to read best block metric: {}", e))?
		as u64;
	wait_for_block_height(collator, pre_upgrade_height + 5, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	test_log!(TEST, "Blocks produced after broken upgrade, chain is stable");

	test_log!(TEST, "Storing more data with broken runtime (v1 format)...");
	test_log!(TEST, "Using tracked nonce after upgrade: {}", nonce);
	let (store_block_2, _new_nonce) = authorize_and_store_data(collator, &data, nonce).await?;
	test_log!(
		TEST,
		"Data stored at block {} (v1 TransactionInfo) — broken runtime works for new stores",
		store_block_2
	);

	// --- Wait for chain stall ---
	// Without the migration, the chain will stall when check_proof tries to decode
	// v0 entries using the v1 struct. This happens at approximately:
	//   store_block + retention_period
	let critical_block = store_block + TEST_RETENTION_PERIOD as u64;
	test_log!(
		TEST,
		"=== Waiting for chain stall (expected around block {}, store_block={} + retention={}) ===",
		critical_block,
		store_block,
		TEST_RETENTION_PERIOD
	);

	let stall_height = wait_for_chain_stall(
		collator,
		STALL_DETECTION_INTERVAL_SECS,
		STALL_THRESHOLD_SECS,
		STALL_DETECTION_TIMEOUT_SECS,
	)
	.await?;

	test_log!(TEST, "Chain stalled at block {} (expected around {})", stall_height, critical_block);

	// --- Recovery via code_substitutes ---
	test_log!(TEST, "=== Starting recovery via code_substitutes ===");

	// Step 1: force_set_current_code on relay chain
	// This updates the relay chain's view of the parachain validation code.
	// Relay validators will now validate parachain blocks using the fix runtime.
	let relay_alice = network
		.get_node("alice")
		.map_err(|e| anyhow::anyhow!("Failed to get relay alice node: {}", e))?;
	let para_id = get_para_id();
	let mut relay_nonce = get_alice_nonce(relay_alice).await?;

	test_log!(TEST, "Step 1: force_set_current_code on relay chain (para_id={})...", para_id);
	force_parachain_code_upgrade_via_relay(relay_alice, para_id, &fix_wasm, &mut relay_nonce)
		.await?;
	test_log!(TEST, "Relay chain now uses fix runtime for para validation");

	// Step 2: Add code_substitutes to collator's chain spec
	// This tells the collator client to use the fix runtime WASM starting at the
	// stall block, instead of the on-chain broken runtime.
	let base_dir = network
		.base_dir()
		.ok_or_else(|| anyhow::anyhow!("Network base_dir not available"))?;
	test_log!(TEST, "Step 2: Searching for chain spec in {}...", base_dir);

	let chain_id = get_parachain_chain_id();
	let spec_paths = find_all_parachain_chain_specs(base_dir, &chain_id)?;
	test_log!(TEST, "Found {} chain spec file(s)", spec_paths.len());

	for spec_path in &spec_paths {
		add_code_substitute(spec_path, stall_height, &fix_wasm)?;
		test_log!(TEST, "Code substitute added in {}", spec_path.display());
	}
	test_log!(
		TEST,
		"Code substitute added at block {} — collator will use fix runtime from this block",
		stall_height
	);

	// Step 3: Restart collator
	// The collator reads the modified chain spec on restart, picks up the code substitute,
	// and uses the fix runtime for block execution from stall_height onward.
	test_log!(TEST, "Step 3: Restarting collator...");
	collator
		.restart(Some(Duration::from_secs(3)))
		.await
		.map_err(|e| anyhow::anyhow!("Failed to restart collator: {}", e))?;

	test_log!(TEST, "Waiting for collator to come back up...");
	collator
		.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS)
		.await
		.map_err(|e| anyhow::anyhow!("Collator did not come back up: {}", e))?;
	test_log!(TEST, "Collator restarted successfully");

	// --- Verify chain recovery ---
	test_log!(TEST, "=== Waiting for chain to recover past stall height {} ===", stall_height);
	wait_for_chain_recovery(collator, stall_height, RECOVERY_TIMEOUT_SECS).await?;
	test_log!(TEST, "Chain recovered — blocks being produced past height {}!", stall_height);

	// --- Verify: store new data after recovery ---
	test_log!(TEST, "Verifying chain by storing new data...");
	let mut nonce = get_alice_nonce(collator).await?;
	let (store_block_3, new_nonce) = authorize_and_store_data(collator, &data, nonce).await?;
	nonce = new_nonce;
	test_log!(
		TEST,
		"Data stored at block {} after recovery — chain is fully functional",
		store_block_3
	);

	// --- Post-recovery: normal on-chain runtime upgrade ---
	// Upgrade to next runtime (bumped spec_version, migration un-wired from Executive).
	// This is a standard upgrade via authorize_upgrade + apply_authorized_upgrade
	// on the parachain itself — proves the chain can do normal upgrades after recovery.
	test_log!(TEST, "=== Upgrading to NEXT runtime (normal on-chain upgrade) ===");
	do_parachain_runtime_upgrade(collator, &next_wasm, &mut nonce).await?;
	test_log!(TEST, "Next runtime upgrade complete");

	// After the parachain applies the upgrade, the relay chain must process the new
	// validation code via its upgrade pipeline (validation_upgrade_delay). During this
	// window the collator can't produce backed blocks (local code != relay code).
	// Wait for the relay to catch up, then verify parachain blocks resume.
	test_log!(TEST, "Waiting for relay chain to process the upgrade pipeline...");
	tokio::time::sleep(Duration::from_secs(60)).await;

	test_log!(TEST, "Waiting for parachain blocks after relay processes upgrade...");
	let post_upgrade_height = collator
		.reports(BEST_BLOCK_METRIC)
		.await
		.map_err(|e| anyhow::anyhow!("Failed to read best block metric: {}", e))?
		as u64;
	wait_for_block_height(collator, post_upgrade_height + 3, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await?;
	test_log!(TEST, "Blocks produced after next runtime upgrade");

	test_log!(TEST, "Storing data with next runtime...");
	let nonce = get_alice_nonce(collator).await?;
	let (store_block_4, _) = authorize_and_store_data(collator, &data, nonce).await?;
	test_log!(
		TEST,
		"Data stored at block {} with next runtime — normal upgrades work post-recovery",
		store_block_4
	);

	// --- Cleanup ---
	test_log!(TEST, "=== Test passed! Destroying network. ===");
	let _ = network.destroy().await;

	Ok(())
}
