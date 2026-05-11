// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Auto-renewal end-to-end test (multi-cycle).
//!
//! Verifies that data with auto-renewal enabled survives **multiple consecutive** retention
//! deadlines, not just the first one.
//!
//! ## Lifecycle being exercised
//!
//! With `S` = store block, `RP` = `RetentionPeriod`:
//!
//! - `R1 = S + RP + 1` — first renewal block.
//!   - `on_initialize(R1)` takes `Transactions[obsolete = R1 - RP - 1 = S]` and pushes the entry
//!     onto `PendingAutoRenewals` because `AutoRenewals[content_hash]` is set.
//!   - The mandatory `apply_block_inherents` inherent drains `PendingAutoRenewals` via `do_renew`,
//!     which calls `sp_io::transaction_index::renew(extrinsic_index, content_hash)` to re-index the
//!     existing col11 entry under block `R1`. `TransactionByContentHash` is rewritten to `(R1,
//!     new_index)`.
//! - `R2 = R1 + RP + 1 = S + 2 * (RP + 1)` — second renewal block.
//!   - Same lifecycle, taking `Transactions[R1]` and re-indexing under `R2`.
//!
//! Because each renewal consumes one transaction slot and `data.len()` bytes from the account
//! authorization, the test up-front authorizes enough capacity for the initial store **plus**
//! `NUM_RENEWAL_CYCLES` renewals.
//!
//! ## Assertions
//!
//! - Bitswap fetch from the collator succeeds **right after the original retention deadline**
//!   (block `R1 + 1`) — proves the first renewal was applied.
//! - Bitswap fetch succeeds again **after the second retention deadline** (block `R2 + 1`) — proves
//!   the renewal lifecycle keeps running indefinitely as long as authorization is replenished.
//!
//! ## Environment variables
//!
//! Same as `parachain_sync_storage`:
//! - `POLKADOT_RELAY_BINARY_PATH`, `POLKADOT_PARACHAIN_BINARY_PATH`, `PARACHAIN_CHAIN_SPEC_PATH`,
//!   `RELAY_CHAIN`, `PARACHAIN_ID`, `PARACHAIN_CHAIN_ID`.
//!
//! ## Running
//!
//! ```bash
//! POLKADOT_RELAY_BINARY_PATH=~/local_bulletin_testing/bin/polkadot \
//! POLKADOT_PARACHAIN_BINARY_PATH=~/local_bulletin_testing/bin/polkadot-omni-node \
//! PARACHAIN_CHAIN_SPEC_PATH=$(pwd)/zombienet/bulletin-westend-spec.json \
//!   cargo test -p bulletin-chain-zombienet-sdk-tests \
//!   --features bulletin-chain-zombienet-sdk-tests/zombie-sync-tests \
//!   parachain_auto_renew_test -- --nocapture --test-threads=1
//! ```

use crate::{
	test_log,
	utils::{
		authorize_account_via_sudo, authorize_and_store_data, blake2_256,
		build_parachain_network_config_single_collator, content_hash_and_cid, enable_auto_renew,
		expect_bitswap_dont_have, generate_test_data, get_alice_nonce, initialize_network,
		override_alice_authorization, set_retention_period, submit_renew_pair, submit_store_signed,
		top_up_alice_authorization, verify_node_bitswap, verify_parachain_binaries,
		wait_for_block_height, wait_for_session_change_on_node, AuthorizationOverride,
		BLOCK_PRODUCTION_TIMEOUT_SECS, NETWORK_READY_TIMEOUT_SECS, NODE_LOG_CONFIG,
		PARACHAIN_TEST_DATA_PATTERN, TEST_DATA_SIZE,
	},
};
use anyhow::{Context, Result};
use env_logger::Env;
use std::{collections::HashMap, str::FromStr};
use subxt::{
	config::substrate::{SubstrateConfig, SubstrateExtrinsicParamsBuilder},
	dynamic::{tx, Value},
	ext::scale_value::value,
	OnlineClient,
};
use subxt_signer::{
	sr25519::{dev, Keypair},
	SecretUri,
};

/// Fetch the latest **best** parachain block. `client.blocks().at_latest()` returns the latest
/// **finalized** block via chainHead_v2; on cumulus parachains finality lags production by 10+
/// seconds at startup, so at_latest can be stuck at block 0 well after the chain is producing.
/// `subscribe_best().next()` returns the current best block immediately.
async fn current_best_block(
	client: &OnlineClient<SubstrateConfig>,
) -> Result<subxt::blocks::Block<SubstrateConfig, OnlineClient<SubstrateConfig>>> {
	let mut sub = client.blocks().subscribe_best().await?;
	let block = sub
		.next()
		.await
		.ok_or_else(|| anyhow::anyhow!("subscribe_best stream empty"))??;
	Ok(block)
}

const SESSION_CHANGE_TIMEOUT_SECS: u64 = 300;
const RETENTION_PERIOD: u32 = 10;
const BITSWAP_TIMEOUT_SECS: u64 = 30;

/// Number of renewal cycles to verify end-to-end. Bumping this requires more authorization
/// headroom (see [`TOPUP_TX_COUNT`] / [`TOPUP_BYTES_MULTIPLIER`]) and a longer wait at the end
/// of the test.
const NUM_RENEWAL_CYCLES: u64 = 2;
/// Extra transaction slots to add on top of the 9 left over from `authorize_and_store_data`
/// (which authorizes 10 — store consumes 1). Adds margin so the test isn't sensitive to
/// off-by-one accounting.
const TOPUP_TX_COUNT: u32 = 5;
/// Extra bytes — sized as `(NUM_RENEWAL_CYCLES + 1) × data.len()`. The +1 is safety; without
/// it, a single byte short would silently flip auto-renewal into `AutoRenewalFailed`.
const TOPUP_BYTES_MULTIPLIER: u64 = NUM_RENEWAL_CYCLES + 1;

/// Aggressive block pruning, smaller than [`RETENTION_PERIOD`]. The store block is pruned
/// **before** the proof block (`store_block + RetentionPeriod`); the inherent provider can no
/// longer construct a `TransactionStorageProof` from col11 (the entry has been deleted as its
/// last `transaction_index` ref vanished). The mandatory `apply_block_inherents` is therefore
/// not emitted, `on_finalize`'s `assert!(proof_ok)` fires, and the chain halts.
const BLOCKS_PRUNING_LESS_THAN_RETENTION: u32 = 5;
/// Pruning larger than [`RETENTION_PERIOD`]. The proof block can still find the col11 entry
/// because the store/renew blocks are still within the pruning window, so the chain progresses
/// past `S + RetentionPeriod`. Eviction happens later, once the block holding the last
/// `transaction_index` ref ages out of the pruning window.
const BLOCKS_PRUNING_GREATER_THAN_RETENTION: u32 = 15;
/// Tight timeout for halt detection. If the chain has stalled (proof block panic), we want to
/// surface that quickly rather than wait the default 300s `BLOCK_PRODUCTION_TIMEOUT_SECS`.
const HALT_DETECTION_TIMEOUT_SECS: u64 = 120;
/// Larger retention used by the two `..._fails_under_pruning_..._test` halt scenarios. With
/// pruning=5 + RP=10, the proof block at `S+10` lands too close to the store block — finality
/// hasn't caught up enough for pruning to actually evict col11 yet. Bumping retention to 20
/// pushes the proof block out to `S+20` (~2-3 minutes wall-clock), which is comfortably past
/// the (finality + pruning) lag, so col11 is reliably empty at the proof block.
const RETENTION_PERIOD_FOR_PRUNING_HALT: u32 = 20;
/// Number of items used by the bulk auto-renewal test. Set to the pallet's
/// `MaxBlockTransactions` cap (512) to exercise the worst-case `PendingAutoRenewals` drain in
/// a single block.
const MANY_ITEMS_COUNT: u32 = 512;
/// Number of consecutive renewal cycles to observe for the stability check. Each cycle re-runs
/// the same 512-item renewal at `S + k*(RP+1)` for `k=1..=N`. Multiple measurements at the
/// same N let us see whether the prometheus block-construction time is stable (suggests the
/// inherent's actual cost is well-bounded) or volatile (suggests scheduler or I/O noise).
const RENEWAL_CYCLES_TO_OBSERVE: u32 = 3;

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
		"--".into(),
		"--network-backend=libp2p".into(),
	]
}

/// Navigate the dynamic `System::BlockWeight` value (a `PerDispatchClass<Weight>`) and
/// extract `(ref_time, proof_size)` for the given class (`"normal"`, `"operational"`, or
/// `"mandatory"`). Returns `(0, 0)` if the structure shape is unexpected.
fn extract_class_weight<C>(
	weight_value: &subxt::ext::scale_value::Value<C>,
	class: &str,
) -> (u64, u64) {
	use subxt::ext::scale_value::{Composite, Primitive, ValueDef};

	let ValueDef::Composite(Composite::Named(top)) = &weight_value.value else {
		return (0, 0);
	};
	let Some((_, class_value)) = top.iter().find(|(k, _)| k == class) else {
		return (0, 0);
	};
	let ValueDef::Composite(Composite::Named(inner)) = &class_value.value else {
		return (0, 0);
	};
	let mut ref_time = 0u64;
	let mut proof_size = 0u64;
	for (k, v) in inner {
		if let ValueDef::Primitive(Primitive::U128(n)) = &v.value {
			match k.as_str() {
				"ref_time" => ref_time = *n as u64,
				"proof_size" => proof_size = *n as u64,
				_ => {},
			}
		}
	}
	(ref_time, proof_size)
}

/// Read `System::BlockWeights::max_block` as `(ref_time, proof_size)` from the runtime
/// constants. This is the absolute ceiling that even Mandatory-class extrinsics must fit
/// inside — the same bound the in-pallet `ensure_weight_sanity` test asserts statically.
async fn fetch_max_block_weight(client: &OnlineClient<SubstrateConfig>) -> Result<(u64, u64)> {
	use subxt::ext::scale_value::{Composite, Primitive, ValueDef};

	let addr = subxt::dynamic::constant("System", "BlockWeights");
	let value = client.constants().at(&addr)?.to_value()?;
	let ValueDef::Composite(Composite::Named(top)) = &value.value else {
		anyhow::bail!("BlockWeights: unexpected top shape");
	};
	let (_, max_block) = top
		.iter()
		.find(|(k, _)| k == "max_block")
		.ok_or_else(|| anyhow::anyhow!("BlockWeights: missing max_block"))?;
	let ValueDef::Composite(Composite::Named(inner)) = &max_block.value else {
		anyhow::bail!("BlockWeights.max_block: unexpected shape");
	};
	let mut ref_time = 0u64;
	let mut proof_size = 0u64;
	for (k, v) in inner {
		if let ValueDef::Primitive(Primitive::U128(n)) = &v.value {
			match k.as_str() {
				"ref_time" => ref_time = *n as u64,
				"proof_size" => proof_size = *n as u64,
				_ => {},
			}
		}
	}
	Ok((ref_time, proof_size))
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_test() -> Result<()> {
	const TEST: &str = "para_auto_renew";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(
		TEST,
		"=== Parachain Auto-Renewal Test (multi-cycle, {} cycles) ===",
		NUM_RENEWAL_CYCLES
	);

	verify_parachain_binaries()?;

	let config = build_parachain_network_config_single_collator(get_para_node_args())?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	log::info!("Waiting for relay chain session change...");
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.context("Failed to detect session change on relay chain")?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	// 1. Shrink RetentionPeriod so each cycle fits in test runtime.
	let mut nonce = get_alice_nonce(collator1).await?;
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	set_retention_period(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// 2. Authorize 2× data and store one item. Helper consumes 1× of that on the store, leaving 1×
	//    / 9 tx — enough for one renewal cycle.
	let data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (hash_hex, cid) = content_hash_and_cid(&data);
	log::info!("Test data: {} bytes, hash={}, CID={}", data.len(), hash_hex, cid);

	let (store_block, next_nonce) = authorize_and_store_data(collator1, &data, nonce).await?;
	nonce = next_nonce;
	log::info!("Data stored at block {}", store_block);

	verify_node_bitswap(collator1, &data, BITSWAP_TIMEOUT_SECS, "Collator-1 (post-store)").await?;

	// 3. Top up authorization for the additional renewal cycles. `authorize_account` adds to
	//    existing authorization (per pallet docs).
	top_up_alice_authorization(
		&client,
		TOPUP_TX_COUNT,
		data.len() as u64 * TOPUP_BYTES_MULTIPLIER,
		nonce,
	)
	.await?;
	nonce += 1;

	// 4. Enable auto-renewal for this content hash.
	let content_hash = blake2_256(&data);
	enable_auto_renew(&client, &content_hash, nonce).await?;
	log::info!("Auto-renewal enabled for content_hash {}", hash_hex);

	// 5. Verify the data survives each retention deadline. Renewal cadence is `RP + 1`, so cycle
	//    `k` lands at `S + k * (RP + 1)`. We wait one extra block so the `apply_block_inherents`
	//    inherent has been observed by the chain head.
	let cadence = RETENTION_PERIOD as u64 + 1;
	for cycle in 1..=NUM_RENEWAL_CYCLES {
		let renewal_block = store_block + cycle * cadence;
		let wait_until = renewal_block + 1;
		log::info!(
			"[cycle {}/{}] Waiting for block {} (renewal at block {})",
			cycle,
			NUM_RENEWAL_CYCLES,
			wait_until,
			renewal_block
		);
		wait_for_block_height(collator1, wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

		verify_node_bitswap(
			collator1,
			&data,
			BITSWAP_TIMEOUT_SECS,
			&format!("Collator-1 (after cycle {})", cycle),
		)
		.await
		.with_context(|| {
			format!(
				"Auto-renewal cycle {} did not preserve the data: bitswap returned no data \
				 at block ≥ {}",
				cycle, wait_until
			)
		})?;
		log::info!(
			"[cycle {}/{}] ✓ Data still served at block ≥ {}",
			cycle,
			NUM_RENEWAL_CYCLES,
			wait_until
		);
	}

	test_log!(TEST, "=== Parachain Auto-Renewal Test ({} cycles) PASSED ===", NUM_RENEWAL_CYCLES);
	network.destroy().await?;
	Ok(())
}

/// Verify that `check_proof` cannot complete when block pruning has already evicted the data
/// the proof would cover, so the chain halts. No auto-renewal in this scenario.
///
/// Sequence with `--blocks-pruning=5` and `RetentionPeriod=10`:
///
/// - Store at `S` → col11 ref from block `S` (refcount = 1).
/// - Around block `S + 5`, pruning ages block `S` out of the window. Its `transaction_index` ref is
///   dropped, the col11 refcount hits 0, and the entry is deleted.
/// - At block `S + RetentionPeriod`, the proof step targets `target_number = S`. The pallet sees
///   `Transactions[S]` is still in state (it'll be taken at the next block), so a proof IS
///   required. The off-chain inherent provider tries to construct the proof from col11 — but col11
///   no longer holds the data, so the provider returns no proof.
/// - `create_inherent` reads `proof = None`, no `PendingAutoRenewals` either → `None` is emitted →
///   the mandatory `apply_block_inherents` extrinsic is **not** in the block.
/// - `on_finalize`'s `assert!(proof_ok)` fires; the block proposal panics; the collator can't
///   produce block `S + RetentionPeriod`. Chain halts at `S + RetentionPeriod - 1`.
///
/// The test asserts the halt by waiting on a tight timeout for the proof block to be reached
/// and expecting that wait to time out.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_check_proof_fails_under_pruning_test() -> Result<()> {
	const TEST: &str = "para_check_proof_pruning";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(
		TEST,
		"=== Parachain check_proof fails under pruning ({} blocks pruning, retention {}) ===",
		BLOCKS_PRUNING_LESS_THAN_RETENTION,
		RETENTION_PERIOD_FOR_PRUNING_HALT
	);

	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(BLOCKS_PRUNING_LESS_THAN_RETENTION);
	let config = build_parachain_network_config_single_collator(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD_FOR_PRUNING_HALT);
	set_retention_period(&client, RETENTION_PERIOD_FOR_PRUNING_HALT, nonce).await?;
	nonce += 1;

	let data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (hash_hex, _) = content_hash_and_cid(&data);
	log::info!("Test data: {} bytes, hash={}", data.len(), hash_hex);

	let (store_block, _) = authorize_and_store_data(collator1, &data, nonce).await?;
	log::info!("Data stored at block {}", store_block);

	// Reach the last block the chain should be able to produce. With pruning < retention,
	// the inherent provider fails to construct a proof at block `store_block + RetentionPeriod`,
	// the inherent isn't emitted, and on_finalize panics. So `store_block + RetentionPeriod - 1`
	// is the last block that can be produced.
	let last_healthy_block = store_block + RETENTION_PERIOD_FOR_PRUNING_HALT as u64 - 1;
	log::info!("Confirming chain reaches block {} (last healthy block)", last_healthy_block);
	wait_for_block_height(collator1, last_healthy_block, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await
		.context("Chain failed to reach the last healthy block before the proof block")?;

	// Now the proof block: chain should NOT advance past it.
	let proof_block = store_block + RETENTION_PERIOD_FOR_PRUNING_HALT as u64;
	log::info!(
		"Waiting up to {}s for block {} (proof block) — expected to time out (chain halt)",
		HALT_DETECTION_TIMEOUT_SECS,
		proof_block
	);
	match wait_for_block_height(collator1, proof_block, HALT_DETECTION_TIMEOUT_SECS).await {
		Err(_) => {
			log::info!(
				"✓ Chain did not advance past block {} within {}s — proof block panic confirmed",
				last_healthy_block,
				HALT_DETECTION_TIMEOUT_SECS
			);
		},
		Ok(()) => anyhow::bail!(
			"Chain advanced to block {} despite --blocks-pruning={} < RetentionPeriod={}; \
			 the inherent provider must have generated a proof from data we expected to be \
			 pruned, or pruning isn't actually deleting col11 entries on this build.",
			proof_block,
			BLOCKS_PRUNING_LESS_THAN_RETENTION,
			RETENTION_PERIOD_FOR_PRUNING_HALT
		),
	}

	test_log!(TEST, "=== Parachain check_proof fails under pruning PASSED ===");
	network.destroy().await?;
	Ok(())
}

/// Verify that enabling auto-renewal does **not** rescue the chain from the same `check_proof`
/// halt as the previous test: the proof block (`S + RetentionPeriod`) precedes the renewal
/// block (`S + RetentionPeriod + 1`) by one block, so the chain panics on the proof step
/// **before** auto-renewal would otherwise have a chance to fire.
///
/// This is a regression guard: there's no clever ordering — pairing
/// `--blocks-pruning < RetentionPeriod` with auto-renewal still halts the chain.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_under_pruning_chain_halts_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_pruning_halt";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(
		TEST,
		"=== Parachain auto-renewal under pruning halts ({} blocks pruning, retention {}) ===",
		BLOCKS_PRUNING_LESS_THAN_RETENTION,
		RETENTION_PERIOD_FOR_PRUNING_HALT
	);

	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(BLOCKS_PRUNING_LESS_THAN_RETENTION);
	let config = build_parachain_network_config_single_collator(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD_FOR_PRUNING_HALT);
	set_retention_period(&client, RETENTION_PERIOD_FOR_PRUNING_HALT, nonce).await?;
	nonce += 1;

	let data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (store_block, next_nonce) = authorize_and_store_data(collator1, &data, nonce).await?;
	nonce = next_nonce;
	log::info!("Data stored at block {}", store_block);

	let content_hash = blake2_256(&data);
	enable_auto_renew(&client, &content_hash, nonce).await?;
	log::info!("Auto-renewal enabled");

	// Same halt math as parachain_check_proof_fails_under_pruning_test: chain stalls at
	// `store_block + RetentionPeriod`. Auto-renewal would have fired one block later (at
	// `store_block + RetentionPeriod + 1`) but never gets the chance.
	let last_healthy_block = store_block + RETENTION_PERIOD_FOR_PRUNING_HALT as u64 - 1;
	log::info!("Confirming chain reaches block {} (last healthy block)", last_healthy_block);
	wait_for_block_height(collator1, last_healthy_block, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	let proof_block = store_block + RETENTION_PERIOD_FOR_PRUNING_HALT as u64;
	log::info!(
		"Waiting up to {}s for block {} (proof block) — expected timeout (halt before renewal \
		 block at {})",
		HALT_DETECTION_TIMEOUT_SECS,
		proof_block,
		proof_block + 1
	);
	match wait_for_block_height(collator1, proof_block, HALT_DETECTION_TIMEOUT_SECS).await {
		Err(_) => {
			log::info!(
				"✓ Chain did not advance past block {} within {}s — auto-renewal did not save \
				 us from the proof-block panic",
				last_healthy_block,
				HALT_DETECTION_TIMEOUT_SECS
			);
		},
		Ok(()) => anyhow::bail!(
			"Chain unexpectedly advanced to block {} — auto-renewal should not be able to \
			 sidestep the proof-block panic, but it apparently did. Something fundamental \
			 changed in the inherent ordering.",
			proof_block
		),
	}

	test_log!(TEST, "=== Parachain auto-renewal under pruning halts PASSED ===");
	network.destroy().await?;
	Ok(())
}

/// Verify that calling `renew` twice for the same data within (approximately) one block
/// stacks col11 refs from the renewal block on top of the original store-block ref, and that
/// the data stays fetchable until **all** referencing blocks have been pruned.
///
/// Setup uses `--blocks-pruning=15` (greater than `RetentionPeriod=10`) so the proof block
/// can find col11 alive and the chain progresses normally. Sequence:
///
/// - Store at `S` → col11 refs from block `S` (refcount = 1).
/// - Top up authorization, then submit two `renew(S, 0)` extrinsics back-to-back. Both go into the
///   pool before any block is produced and normally land in the same block `R`, adding two more
///   col11 refs (refcount = 3).
/// - Bitswap fetch right after `R` — succeeds.
/// - Wait until block `S + 15 + 3` so block `S` ages out of the pruning window. Refcount drops to 2
///   (only `R`'s refs survive). Bitswap still succeeds.
/// - Wait until block `R + 15 + 3` so block `R` ages out. Refcount drops to 0. col11 entry evicted.
///   Bitswap returns `DONT_HAVE`.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_renew_twice_within_block_with_pruning_test() -> Result<()> {
	const TEST: &str = "para_renew_twice_pruning";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(
		TEST,
		"=== Parachain double-renew under pruning ({} blocks pruning, retention {}) ===",
		BLOCKS_PRUNING_GREATER_THAN_RETENTION,
		RETENTION_PERIOD
	);

	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(BLOCKS_PRUNING_GREATER_THAN_RETENTION);
	let config = build_parachain_network_config_single_collator(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	set_retention_period(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	let data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (hash_hex, _) = content_hash_and_cid(&data);
	log::info!("Test data: {} bytes, hash={}", data.len(), hash_hex);

	let (store_block, next_nonce) = authorize_and_store_data(collator1, &data, nonce).await?;
	nonce = next_nonce;
	log::info!("Data stored at block {}", store_block);

	// The pallet's `validate_signed` tags renewals with `(who, content_hash)`, so two renews
	// from the **same** signer for the same data conflict in the pool. To get two renews of
	// the same data into the same block we need a second signer — Bob, authorized via sudo.
	let bob_pk = subxt_signer::sr25519::dev::bob().public_key().0;
	authorize_account_via_sudo(&client, &bob_pk, 1, data.len() as u64, nonce).await?;
	nonce += 1;
	let bob_nonce = client
		.tx()
		.account_nonce(&subxt_signer::sr25519::dev::bob().public_key().to_account_id())
		.await?;

	let (renew_block_a, renew_block_b) =
		submit_renew_pair(&client, store_block as u32, 0, nonce, bob_nonce).await?;
	// `nonce` is no longer used after this point in the test; the two renews are the last
	// signed extrinsics here.
	if renew_block_a != renew_block_b {
		log::warn!(
			"Renews landed in different blocks ({} and {}) instead of one — test still valid \
			 but uses the later block for pruning math",
			renew_block_a,
			renew_block_b
		);
	} else {
		log::info!("Both renews landed in the same block {}", renew_block_a);
	}
	let renew_block = std::cmp::max(renew_block_a, renew_block_b);

	verify_node_bitswap(collator1, &data, BITSWAP_TIMEOUT_SECS, "Collator-1 (post-renew)").await?;

	// Wait until both the store block and the renew block have aged well past the pruning
	// window. Finality lag (2-3 blocks on a healthy westend-local) shifts the actual pruning
	// boundary, so we use a generous +5 buffer beyond `pruning_size` past the latest renew
	// block. By then col11 refs from S, R should all be gone — refcount = 0 → entry evicted.
	let after_renew_pruned = renew_block + BLOCKS_PRUNING_GREATER_THAN_RETENTION as u64 + 5;
	log::info!(
		"Waiting for block {} so both store and renew blocks are pruned",
		after_renew_pruned
	);
	wait_for_block_height(collator1, after_renew_pruned, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	expect_bitswap_dont_have(collator1, &data, BITSWAP_TIMEOUT_SECS, "Collator-1 (post-pruning)")
		.await
		.context(
			"Bitswap still serves data after both store and renew blocks were pruned — col11 \
			 should be empty",
		)?;
	log::info!(
		"✓ Bitswap returns DONT_HAVE after both store and renew blocks were pruned (col11 \
		 refcount reached zero)"
	);

	test_log!(TEST, "=== Parachain double-renew under pruning PASSED ===");
	network.destroy().await?;
	Ok(())
}

/// Verify that a fresh `store` and the auto-renewal inherent for already-stored data can land
/// in the same block and both pieces of data are then fetchable. Later, block pruning evicts
/// the freshly-stored item (it has no auto-renewal) while the original auto-renewing item
/// stays alive (each renewal block adds a fresh col11 ref).
///
/// Uses `--blocks-pruning=15` so the chain progresses normally. The fresh `store` is timed to
/// land in the renewal block:
///
/// - Wait until block `R - 1 = S + RetentionPeriod` is reached on the chain head.
/// - Submit `store(data2)` — it lands in the next block, which will be `R = S + RetentionPeriod +
///   1`.
/// - At `R`, `apply_block_inherents` drains `PendingAutoRenewals` (renews `data1`) and the
///   `store(data2)` extrinsic is processed in the same block. col11 ends up with refs: `data1` from
///   `S` + `R`, `data2` from `R`.
///
/// Long-term: at `R + pruning_size + 3` the original store-block has been pruned, dropping
/// `data1`'s ref from `S`. `data1` survives because `R`'s ref is still in pruning window
/// AND auto-renewal at `R2 = S + 2*(RetentionPeriod+1)` adds another ref.
/// `data2` has no auto-renewal — so when block `R` is pruned, `data2`'s only ref vanishes.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_with_concurrent_store_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_concurrent_store";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(
		TEST,
		"=== Parachain auto-renewal + same-block store ({} blocks pruning, retention {}) ===",
		BLOCKS_PRUNING_GREATER_THAN_RETENTION,
		RETENTION_PERIOD
	);

	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(BLOCKS_PRUNING_GREATER_THAN_RETENTION);
	let config = build_parachain_network_config_single_collator(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	set_retention_period(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	let data1 = generate_test_data(TEST_DATA_SIZE, b"DATA1_CONCURRENT_STORE_");
	let data2 = generate_test_data(TEST_DATA_SIZE, b"DATA2_CONCURRENT_STORE_");
	log::info!("data1 hash={}", content_hash_and_cid(&data1).0);
	log::info!("data2 hash={}", content_hash_and_cid(&data2).0);

	let (store_block, next_nonce) = authorize_and_store_data(collator1, &data1, nonce).await?;
	nonce = next_nonce;
	log::info!("data1 stored at block {}", store_block);

	// Top up authorization for: 1× data2 store + 2× data1 renewals (R1 and R2) + safety.
	top_up_alice_authorization(&client, 5, 4 * data1.len() as u64, nonce).await?;
	nonce += 1;

	let content_hash_data1 = blake2_256(&data1);
	enable_auto_renew(&client, &content_hash_data1, nonce).await?;
	nonce += 1;
	log::info!("Auto-renewal enabled for data1");

	// Wait until block before the renewal target. Renewal fires at R = S + RetentionPeriod + 1.
	// Wait until R - 1 = S + RetentionPeriod is reached, then submit data2 — it'll be pulled
	// from the pool by the next block proposal, which is R itself.
	let renewal_block = store_block + RETENTION_PERIOD as u64 + 1;
	let wait_until_pre_renewal = renewal_block - 1;
	log::info!(
		"Waiting until block {} (one before renewal block {}) before submitting data2",
		wait_until_pre_renewal,
		renewal_block
	);
	wait_for_block_height(collator1, wait_until_pre_renewal, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	let data2_block = submit_store_signed(&client, &data2, nonce).await?;
	log::info!("data2 store landed at block {}", data2_block);
	if data2_block != renewal_block {
		anyhow::bail!(
			"Timing missed: expected data2 to land in renewal block {}, but it landed at \
			 block {}. Re-run the test, or adjust the wait_until math if this is consistent.",
			renewal_block,
			data2_block
		);
	}
	log::info!("✓ data2 store and auto-renewal inherent coexist in block {}", renewal_block);

	// Both items should be fetchable.
	verify_node_bitswap(collator1, &data1, BITSWAP_TIMEOUT_SECS, "Collator-1 / data1").await?;
	verify_node_bitswap(collator1, &data2, BITSWAP_TIMEOUT_SECS, "Collator-1 / data2").await?;

	// Wait until block R is pruned. data2 has no auto-renewal — its only ref was at R, so
	// once R is gone, data2 is evicted. data1 had its ref refreshed at R AND at R2 = R + 11
	// (= S + 22), so data1 keeps surviving.
	let after_renewal_pruned = renewal_block + BLOCKS_PRUNING_GREATER_THAN_RETENTION as u64 + 5;
	log::info!("Waiting for block {} so the renewal block is pruned", after_renewal_pruned);
	wait_for_block_height(collator1, after_renewal_pruned, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	expect_bitswap_dont_have(
		collator1,
		&data2,
		BITSWAP_TIMEOUT_SECS,
		"Collator-1 / data2 (post-pruning)",
	)
	.await
	.context(
		"data2 (no auto-renewal) should be evicted once its only ref-block is pruned, but \
		 bitswap still serves it",
	)?;
	log::info!("✓ data2 evicted by pruning (no auto-renewal kept it alive)");

	verify_node_bitswap(
		collator1,
		&data1,
		BITSWAP_TIMEOUT_SECS,
		"Collator-1 / data1 (post-pruning)",
	)
	.await
	.context(
		"data1 should still be served via bitswap — auto-renewal at R2 should have added a \
		 fresh col11 ref before R was pruned",
	)?;
	log::info!("✓ data1 still alive — auto-renewal at R2 added a fresh ref before R was pruned");

	test_log!(TEST, "=== Parachain auto-renewal + same-block store PASSED ===");
	network.destroy().await?;
	Ok(())
}

/// Side-by-side eviction test: store two distinct items, enable auto-renewal on only one of them,
/// then wait long enough for `--blocks-pruning` to evict the non-renewed item's store block. The
/// auto-renewed item must remain bitswap-fetchable; the other must be evicted (`bitswap
/// DONT_HAVE`).
///
/// Pruning configuration: `--blocks-pruning = BLOCKS_PRUNING_GREATER_THAN_RETENTION = 15`. The
/// pruning rule keeps the last `N + 1` blocks **past finalized**, not past chain head — so block
/// `S` is only pruned once finality reaches roughly `S + N + 1`. With the parachain's finality
/// lag of ~3–5 blocks, that lands around chain head `S + N + 1 + lag` ≈ `S + 20`. We wait until
/// `S + RetentionPeriod + 15 = S + 25` to be safely past pruning.
///
/// At that point: the non-renewed item's only `transaction_index` ref (at block `S`) is gone; the
/// auto-renewed item had a fresh ref added at the renewal block (`S + RP + 1 = S + 11`) before
/// `S` was pruned, so its data column entry survives.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_vs_no_renew_eviction_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_vs_no_renew";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(
		TEST,
		"=== Auto-renew vs no-renew eviction (blocks-pruning={}, retention={}) ===",
		BLOCKS_PRUNING_GREATER_THAN_RETENTION,
		RETENTION_PERIOD,
	);

	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(BLOCKS_PRUNING_GREATER_THAN_RETENTION);
	let config = build_parachain_network_config_single_collator(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	set_retention_period(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	let data_renewed = generate_test_data(TEST_DATA_SIZE, b"DATA_AUTO_RENEWED_");
	let data_not_renewed = generate_test_data(TEST_DATA_SIZE, b"DATA_NOT_RENEWED_");

	// Store both items. `authorize_and_store_data` returns the block the first store landed in;
	// the second store goes through Alice's leftover authorization (topped up below) and lands
	// in the same or the next block — we don't care which, as long as it's well before the
	// renewal block at S + RP + 1.
	let (store_block, next_nonce) =
		authorize_and_store_data(collator1, &data_renewed, nonce).await?;
	nonce = next_nonce;
	log::info!("data_renewed stored at block {}", store_block);

	// Top up authorization for: 1× data_not_renewed store + 2× data_renewed renewals + safety.
	top_up_alice_authorization(&client, 5, 4 * data_renewed.len() as u64, nonce).await?;
	nonce += 1;

	let data_not_renewed_block = submit_store_signed(&client, &data_not_renewed, nonce).await?;
	nonce += 1;
	log::info!("data_not_renewed stored at block {}", data_not_renewed_block);

	// Enable auto-renew for ONLY the first item.
	let content_hash_renewed = blake2_256(&data_renewed);
	enable_auto_renew(&client, &content_hash_renewed, nonce).await?;
	log::info!("Auto-renewal enabled for data_renewed");

	// Both items should be bitswap-fetchable shortly after upload.
	verify_node_bitswap(
		collator1,
		&data_renewed,
		BITSWAP_TIMEOUT_SECS,
		"Collator-1 / data_renewed (post-store)",
	)
	.await?;
	verify_node_bitswap(
		collator1,
		&data_not_renewed,
		BITSWAP_TIMEOUT_SECS,
		"Collator-1 / data_not_renewed (post-store)",
	)
	.await?;
	log::info!("✓ Both items fetchable shortly after upload");

	// Wait long enough for `--blocks-pruning` to evict the original store block. With pruning
	// keeping `N + 1` blocks past FINALIZED (not past chain head) and finality lagging the head
	// by ~3-5 blocks on the parachain, block `S` is reliably pruned by chain head
	// `S + blocks_pruning + finality_lag + buffer`. Waiting `S + RP + 15` is a comfortable margin.
	let wait_until = store_block + RETENTION_PERIOD as u64 + 15;
	log::info!(
		"Waiting for block {} (store + RP + 15) so block {} is pruned",
		wait_until,
		store_block
	);
	wait_for_block_height(collator1, wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	verify_node_bitswap(
		collator1,
		&data_renewed,
		BITSWAP_TIMEOUT_SECS,
		"Collator-1 / data_renewed (post-retention)",
	)
	.await
	.context(
		"data_renewed should still be served — auto-renewal added a fresh ref before the \
		 original store block was pruned",
	)?;
	log::info!("✓ data_renewed still served via bitswap");

	expect_bitswap_dont_have(
		collator1,
		&data_not_renewed,
		BITSWAP_TIMEOUT_SECS,
		"Collator-1 / data_not_renewed (post-retention)",
	)
	.await
	.context(
		"data_not_renewed should be evicted — its only ref was at the now-pruned store block",
	)?;
	log::info!("✓ data_not_renewed evicted (no auto-renewal kept it alive)");

	test_log!(TEST, "=== Auto-renew vs no-renew eviction PASSED ===");
	network.destroy().await?;
	Ok(())
}

/// Bulk auto-renewal scenario: enable auto-renew on `MANY_ITEMS_COUNT` items and observe how
/// the apply_block_inherents inherent eats block weight at the renewal blocks.
///
/// Stores N items via parallel submissions (Alice signs all). Each `store(data)` carries a
/// distinct `content_hash`, so the pool's `provides((who, content_hash))` tags don't conflict
/// and as many fit into each block as the runtime's weight + length budgets allow. After all
/// N stores are included, parallel-submit N `enable_auto_renew(content_hash)` extrinsics.
///
/// Once we've waited past the latest renewal block (`max(store_blocks) + RetentionPeriod + 1`),
/// walk the chain head backward (`block.header().parent_hash`) to map every block-number-to-hash
/// in the renewal window, then for each renewal block dump:
///
/// - The number of `DataAutoRenewed` events emitted (i.e. how many items the inherent renewed in
///   this block).
/// - `System::BlockWeight` (per-class normal/operational/mandatory `ref_time` + `proof_size`).
///
/// The test is diagnostic-leaning: its hard assertion is just "every stored item was renewed
/// at least once". The interesting output is the per-block weight log — that tells you how
/// much the inherent costs at scale.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_many_items_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_many";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Auto-renew {} items, measure block weight ===", MANY_ITEMS_COUNT);

	verify_parachain_binaries()?;

	let config = build_parachain_network_config_single_collator(get_para_node_args())?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	set_retention_period(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// Authorize Alice for: 1 store + RENEWAL_CYCLES_TO_OBSERVE renewals per item + one cycle
	// of safety margin. Each cycle consumes 1 tx-slot and `data.len()` bytes per item.
	let bytes_per_item = TEST_DATA_SIZE as u64;
	let cycles_to_authorize = RENEWAL_CYCLES_TO_OBSERVE + 2; // 1 store + N renewals + 1 safety
	authorize_account_via_sudo(
		&client,
		&dev::alice().public_key().0,
		MANY_ITEMS_COUNT * cycles_to_authorize,
		bytes_per_item * (MANY_ITEMS_COUNT as u64) * cycles_to_authorize as u64,
		nonce,
	)
	.await?;
	nonce += 1;

	// Generate N distinct payloads.
	let items: Vec<Vec<u8>> = (0..MANY_ITEMS_COUNT)
		.map(|i| {
			let mut pattern = b"AUTO_RENEW_MANY_ITEMS_".to_vec();
			pattern.extend_from_slice(format!("{:04}_", i).as_bytes());
			generate_test_data(TEST_DATA_SIZE, &pattern)
		})
		.collect();
	let content_hashes: Vec<[u8; 32]> = items.iter().map(|d| blake2_256(d)).collect();
	log::info!("Generated {} items, first hash={}", items.len(), hex::encode(content_hashes[0]));

	// Submit all N stores to the pool in parallel via `sign_and_submit` (no watcher). Watcher
	// subscriptions for 512 concurrent submissions saturate subxt's chainHead_v2 pinning, and
	// batching with `then_watch` serializes one batch per block — neither is what we want.
	// Pure pool submission is fast enough that all 512 land in the same block proposal.
	//
	// `PROOF_DECOY=1` adds one extra (non-auto-renewable) store in the block AFTER the bulk
	// stores. That makes the proof block (`first_store_block + retention + 1`) exercise BOTH
	// branches of `apply_block_inherents` in the same block: drain all `MANY_ITEMS_COUNT`
	// renewals (from the bulk store block) AND verify the proof for the decoy block. With
	// `PROOF_DECOY=0` (default), the bulk and proof phases run in adjacent blocks — never
	// the same one.
	let proof_decoy: bool = std::env::var("PROOF_DECOY")
		.ok()
		.and_then(|v| v.parse::<u32>().ok())
		.map(|n| n != 0)
		.unwrap_or(false);
	let alice = dev::alice();
	let pre_store_block = current_best_block(&client).await?.number() as u64;
	log::info!(
		"Submitting {} stores (pre-store block={}, proof_decoy={})",
		MANY_ITEMS_COUNT,
		pre_store_block,
		proof_decoy
	);

	let mut submit_futs = Vec::with_capacity(items.len());
	for (i, data) in items.iter().enumerate() {
		let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce + i as u64).build();
		let signer = alice.clone();
		let cli = client.clone();
		submit_futs.push(async move {
			cli.tx()
				.sign_and_submit(&store_call, &signer, params)
				.await
				.map_err(anyhow::Error::from)
		});
	}
	nonce += MANY_ITEMS_COUNT as u64;
	let _: Vec<subxt::utils::H256> = futures::future::try_join_all(submit_futs).await?;
	log::info!("All {} stores accepted into pool", MANY_ITEMS_COUNT);

	if proof_decoy {
		// Wait until the bulk store block (`pre_store_block + 1`) is reached, then submit the
		// decoy — it then lands in `pre_store_block + 2`. Waiting any longer (e.g. `+ 2`) means
		// the next block proposer has already started authoring, so the decoy slips to `+ 3`
		// and the proof block is no longer adjacent to the bulk.
		wait_for_block_height(collator1, pre_store_block + 1, BLOCK_PRODUCTION_TIMEOUT_SECS)
			.await?;
		let decoy = generate_test_data(TEST_DATA_SIZE, b"AUTO_RENEW_MANY_PROOF_DECOY_");
		let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(&decoy)]);
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
		client
			.tx()
			.sign_and_submit(&store_call, &alice, params)
			.await
			.context("submit proof decoy")?;
		nonce += 1;
		log::info!("Submitted 1 proof-decoy store (no auto-renew enabled)");
	}

	// Wait until inclusion has settled. With ~512 × 2 KB stores at ~1.1 ms/2.17 KB normal-class
	// each, they should fit in 1-2 blocks. Wait 5 blocks past the pre-store snapshot to be safe.
	let store_inclusion_target = pre_store_block + 5;
	wait_for_block_height(collator1, store_inclusion_target, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// Walk the chain backward from head, count `Stored` events per block. This gives us the
	// store_blocks vector without needing per-submission block lookups (which is what races
	// against chainHead pinning when done concurrently).
	let post_store_head_n = {
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(&client).await?;
			if head.number() as u64 >= store_inclusion_target {
				break head.number() as u64;
			}
			if start.elapsed() > poll_timeout {
				anyhow::bail!("Timed out waiting for at_latest >= {}", store_inclusion_target);
			}
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
		}
	};
	let mut store_blocks: Vec<u64> = Vec::with_capacity(items.len());
	{
		let mut current = current_best_block(&client).await?;
		while current.number() as u64 > pre_store_block {
			let block_n = current.number() as u64;
			let events = current.events().await?;
			let stored_count = events
				.iter()
				.filter_map(|e| e.ok())
				.filter(|e| e.pallet_name() == "TransactionStorage" && e.variant_name() == "Stored")
				.count();
			for _ in 0..stored_count {
				store_blocks.push(block_n);
			}
			if block_n == 0 {
				break;
			}
			let parent_hash = current.header().parent_hash;
			current = client.blocks().at(parent_hash).await?;
		}
	}
	let expected_stores = items.len() + if proof_decoy { 1 } else { 0 };
	if store_blocks.len() != expected_stores {
		anyhow::bail!(
			"Expected to find {} Stored events between blocks {}..={}, found {}",
			expected_stores,
			pre_store_block + 1,
			post_store_head_n,
			store_blocks.len()
		);
	}

	let earliest_store = *store_blocks.iter().min().unwrap();
	let latest_store = *store_blocks.iter().max().unwrap();
	let mut store_block_histogram: HashMap<u64, u32> = HashMap::new();
	for b in &store_blocks {
		*store_block_histogram.entry(*b).or_default() += 1;
	}
	log::info!(
		"Stored {} items across blocks {}..={} ({} distinct blocks)",
		MANY_ITEMS_COUNT,
		earliest_store,
		latest_store,
		store_block_histogram.len()
	);
	let mut hist_entries: Vec<_> = store_block_histogram.iter().collect();
	hist_entries.sort_by_key(|(b, _)| **b);
	for (b, n) in hist_entries {
		log::info!("  block {}: {} stores", b, n);
	}

	// Submit all N enable_auto_renew calls to the pool in parallel (no watch).
	let pre_enable_block = current_best_block(&client).await?.number() as u64;
	let mut enable_futs = Vec::with_capacity(content_hashes.len());
	for (i, content_hash) in content_hashes.iter().enumerate() {
		let call = tx(
			"TransactionStorage",
			"enable_auto_renew",
			vec![Value::from_bytes(content_hash.as_slice())],
		);
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce + i as u64).build();
		let signer = alice.clone();
		let cli = client.clone();
		enable_futs.push(async move {
			cli.tx()
				.sign_and_submit(&call, &signer, params)
				.await
				.map_err(anyhow::Error::from)
		});
	}
	nonce += MANY_ITEMS_COUNT as u64;
	let _: Vec<subxt::utils::H256> = futures::future::try_join_all(enable_futs).await?;
	log::info!("All {} enable_auto_renew calls accepted into pool", MANY_ITEMS_COUNT);

	// Wait for enable inclusion + verify by walking events.
	let enable_inclusion_target = pre_enable_block + 5;
	wait_for_block_height(collator1, enable_inclusion_target, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await?;
	{
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(&client).await?;
			if head.number() as u64 >= enable_inclusion_target {
				break;
			}
			if start.elapsed() > poll_timeout {
				anyhow::bail!("Timed out waiting for at_latest >= {}", enable_inclusion_target);
			}
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
		}
	}
	let mut enabled_count = 0usize;
	{
		let mut current = current_best_block(&client).await?;
		while current.number() as u64 > pre_enable_block {
			let events = current.events().await?;
			enabled_count += events
				.iter()
				.filter_map(|e| e.ok())
				.filter(|e| {
					e.pallet_name() == "TransactionStorage" &&
						e.variant_name() == "AutoRenewalEnabled"
				})
				.count();
			if current.number() == 0 {
				break;
			}
			let parent_hash = current.header().parent_hash;
			current = client.blocks().at(parent_hash).await?;
		}
	}
	if enabled_count != content_hashes.len() {
		anyhow::bail!(
			"Expected {} AutoRenewalEnabled events, found {}",
			content_hashes.len(),
			enabled_count
		);
	}
	log::info!("Auto-renewal enabled for all {} items", MANY_ITEMS_COUNT);
	let _ = nonce; // last use

	// Wait past the last renewal block, with per-block prometheus snapshots covering a window
	// that includes a couple of baseline (idle) blocks plus the proof block, the renewal
	// block, and a post-renewal block. Each snapshot reads the cumulative
	// `substrate_proposer_block_constructed_seconds_sum` / `_count` histogram values; pairwise
	// diffs give us actual per-block wall-clock construction time as measured by the collator
	// itself — independent of the runtime's declared weight.
	let renewal_cadence = RETENTION_PERIOD as u64 + 1;
	let first_renewal_block = earliest_store + renewal_cadence;
	// Stability check: observe `RENEWAL_CYCLES_TO_OBSERVE` consecutive renewal cycles for
	// the same 512 items so we get multiple measurements at the same N.
	let last_renewal_block = latest_store + renewal_cadence * RENEWAL_CYCLES_TO_OBSERVE as u64;
	let wait_until = last_renewal_block + 1;
	log::info!(
		"Renewal window: {}..={}; capturing per-block prometheus snapshots up to {}",
		first_renewal_block,
		last_renewal_block,
		wait_until
	);

	let snapshot_range_start = first_renewal_block.saturating_sub(3).max(1);
	let mut prom_snapshots: Vec<(u64, f64, f64)> = Vec::new();
	for n in snapshot_range_start..=wait_until {
		wait_for_block_height(collator1, n, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
		// The metric is cumulative; one read per block right after it lands gives per-block
		// resolution via diff.
		let sum = collator1
			.reports("substrate_proposer_block_constructed_sum".to_string())
			.await
			.map_err(|e| anyhow::anyhow!("read prom sum: {e}"))?;
		let count = collator1
			.reports("substrate_proposer_block_constructed_count".to_string())
			.await
			.map_err(|e| anyhow::anyhow!("read prom count: {e}"))?;
		prom_snapshots.push((n, sum, count));
	}

	log::info!("--- Per-block proposer block_constructed (wall-clock construction time) ---");
	log::info!("Format: blocks (a..=b]: +N blocks, +T s sum, ~ms/block");
	for win in prom_snapshots.windows(2) {
		let (n0, sum0, count0) = win[0];
		let (n1, sum1, count1) = win[1];
		let delta_sum = sum1 - sum0;
		let delta_count = count1 - count0;
		let ms_per_block = if delta_count > 0.0 { delta_sum * 1000.0 / delta_count } else { 0.0 };
		let marker = if n1 == first_renewal_block - 1 {
			" <-- proof-only block"
		} else if n1 >= first_renewal_block && n1 <= last_renewal_block {
			" <-- renewal block"
		} else {
			""
		};
		log::info!(
			"blocks ({}..={}]: +{} blocks, +{:.4} s sum, ~{:.1} ms/block{}",
			n0,
			n1,
			delta_count as u64,
			delta_sum,
			ms_per_block,
			marker
		);
	}

	// `wait_for_block_height` checks the Prometheus best-block metric, which updates faster
	// than subxt's chainHead_v2 subscription. Poll `at_latest()` until it catches up so the
	// backward walk starts from a head that actually covers the renewal window.
	let head = {
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(&client).await?;
			if head.number() as u64 >= wait_until {
				break head;
			}
			if start.elapsed() > poll_timeout {
				anyhow::bail!(
					"Timed out waiting for subxt's at_latest() to see block {} (last seen: {})",
					wait_until,
					head.number()
				);
			}
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
		}
	};
	// Stats window: walk further back than the renewal block so we have baseline blocks
	// (no `apply_block_inherents` emitted at all), the store block (30 stores), the proof
	// block (`apply_block_inherents` with `Some(proof)`, no renewals), and the renewal block
	// (`apply_block_inherents` with `proof: None`, 30 renewals). Subtracting baseline from
	// proof-only and renewal-only blocks isolates the per-component cost.
	let stats_range_start = first_renewal_block.saturating_sub(15).max(1);
	let stats_range_end = last_renewal_block + 2;
	log::info!(
		"Walking chain backward from block {} to cover stats window {}..={}",
		head.number(),
		stats_range_start,
		stats_range_end
	);

	let mut block_hashes_by_number: HashMap<u64, subxt::utils::H256> = HashMap::new();
	let mut current = head;
	loop {
		let n = current.number() as u64;
		if n < stats_range_start {
			break;
		}
		block_hashes_by_number.insert(n, current.hash());
		if n == 0 {
			break;
		}
		let parent_hash = current.header().parent_hash;
		current = client.blocks().at(parent_hash).await?;
	}

	let block_weight_addr = subxt::dynamic::storage("System", "BlockWeight", Vec::<Value>::new());

	let (max_block_ref, max_block_pov) = fetch_max_block_weight(&client).await?;
	log::info!(
		"BlockWeights::max_block = (ref_time={}, proof_size={})",
		max_block_ref,
		max_block_pov
	);

	log::info!("--- Block weight stats ---");
	log::info!("Format: block N | extrinsics={{n}} DataAutoRenewed={{n}} AutoRenewalFailed={{n}} | normal=(ref_time,proof_size) op=(...) mand=(...)");

	let mut total_renewed: u32 = 0;
	let mut weight_violations: Vec<String> = Vec::new();
	for block_n in stats_range_start..=stats_range_end {
		let Some(&block_hash) = block_hashes_by_number.get(&block_n) else {
			log::warn!("No hash recorded for block {}; skipping", block_n);
			continue;
		};
		let block = client.blocks().at(block_hash).await?;
		let extrinsic_count = block.extrinsics().await?.iter().count();
		let events = block.events().await?;
		let auto_renewed: u32 = events
			.iter()
			.filter_map(|e| e.ok())
			.filter(|e| {
				e.pallet_name() == "TransactionStorage" && e.variant_name() == "DataAutoRenewed"
			})
			.count() as u32;
		let auto_renewal_failed: u32 = events
			.iter()
			.filter_map(|e| e.ok())
			.filter(|e| {
				e.pallet_name() == "TransactionStorage" && e.variant_name() == "AutoRenewalFailed"
			})
			.count() as u32;
		let weight_value = client
			.storage()
			.at(block_hash)
			.fetch(&block_weight_addr)
			.await?
			.map(|v| v.to_value())
			.transpose()?;

		let (normal, op, mand) = match weight_value.as_ref() {
			Some(v) => (
				extract_class_weight(v, "normal"),
				extract_class_weight(v, "operational"),
				extract_class_weight(v, "mandatory"),
			),
			None => ((0, 0), (0, 0), (0, 0)),
		};

		// Mark interesting blocks in the log line for easy scanning.
		let marker = if block_hashes_by_number.get(&block_n).is_some() &&
			block_n == first_renewal_block - 1
		{
			" <-- proof-only block"
		} else if block_n >= first_renewal_block && block_n <= last_renewal_block {
			" <-- renewal block"
		} else if store_block_histogram.contains_key(&block_n) {
			" <-- store block"
		} else {
			""
		};

		log::info!(
			"block {:>3} | xt={:>2} renewed={:>3} failed={:>3} | normal=({:>13},{:>9}) op=({:>13},{:>9}) mand=({:>13},{:>9}){}",
			block_n,
			extrinsic_count,
			auto_renewed,
			auto_renewal_failed,
			normal.0,
			normal.1,
			op.0,
			op.1,
			mand.0,
			mand.1,
			marker
		);

		// Hard bound: the mandatory inherent (apply_block_inherents) must fit within
		// `BlockWeights::max_block` in BOTH dimensions. This is the on-chain analogue of
		// the static `pallet_transaction_storage::ensure_weight_sanity` check.
		if mand.0 > max_block_ref {
			weight_violations.push(format!(
				"block {block_n}: mandatory ref_time={} exceeds max_block ref_time={}",
				mand.0, max_block_ref
			));
		}
		if mand.1 > max_block_pov {
			weight_violations.push(format!(
				"block {block_n}: mandatory proof_size={} exceeds max_block proof_size={}",
				mand.1, max_block_pov
			));
		}

		total_renewed += auto_renewed;
	}

	if !weight_violations.is_empty() {
		anyhow::bail!(
			"apply_block_inherents exceeded BlockWeights::max_block on {} block(s):\n  {}",
			weight_violations.len(),
			weight_violations.join("\n  ")
		);
	}

	let expected_total = MANY_ITEMS_COUNT * RENEWAL_CYCLES_TO_OBSERVE;
	log::info!("Total renewals across window: {} / {}", total_renewed, expected_total);
	if total_renewed < expected_total {
		anyhow::bail!(
			"Expected at least {} DataAutoRenewed events across the renewal window {}..={} \
			 ({} items × {} cycles), saw {}. Some items did not renew (possibly insufficient \
			 authorization, PendingAutoRenewals overflow, or do_renew returning Err).",
			expected_total,
			first_renewal_block,
			last_renewal_block,
			MANY_ITEMS_COUNT,
			RENEWAL_CYCLES_TO_OBSERVE,
			total_renewed
		);
	}

	test_log!(TEST, "=== Auto-renew {} items PASSED ===", MANY_ITEMS_COUNT);
	network.destroy().await?;
	Ok(())
}

/// Number of distinct worker accounts used by the worst-case multi-signer test. Sized to match
/// `MaxBlockTransactions` so we exercise the full bench worst case (one `Authorizations` entry
/// touched per renewal — what the post-fix benchmarks model).
const WORST_CASE_WORKERS: u32 = MANY_ITEMS_COUNT;

/// Worst-case auto-renewal scenario for PoV accounting: `WORST_CASE_WORKERS` distinct accounts
/// each store one item and enable auto-renewal. Every renewal hits a distinct
/// `Authorizations[Account(worker_i)]` storage key, so iterations don't collapse into storage
/// cache hits — matching the bench's worst-case PoV model after the cache-hit fixes.
///
/// The single-Alice variant ([`parachain_auto_renew_many_items_test`]) collapses
/// `Authorizations` reads into a single key (cache hit on iterations 2..N), so it exercises a
/// cheaper-than-declared real cost. This test exercises the actual declared worst case end to
/// end and captures wall-clock construction time for direct comparison.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_many_items_worst_case_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_many_worst_case";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(
		TEST,
		"=== Auto-renew {} items via {} distinct workers, measure block weight + clock ===",
		WORST_CASE_WORKERS,
		WORST_CASE_WORKERS,
	);

	verify_parachain_binaries()?;

	let config = build_parachain_network_config_single_collator(get_para_node_args())?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut alice_nonce = get_alice_nonce(collator1).await?;
	log::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	set_retention_period(&client, RETENTION_PERIOD, alice_nonce).await?;
	alice_nonce += 1;

	// Derive `WORST_CASE_WORKERS` deterministic keypairs `//worker_0` … `//worker_{N-1}`.
	let workers: Vec<Keypair> = (0..WORST_CASE_WORKERS)
		.map(|i| {
			let uri = SecretUri::from_str(&format!("//worker_{}", i)).expect("worker URI parses");
			Keypair::from_uri(&uri).expect("worker keypair derives")
		})
		.collect();

	// Authorize every worker via sudo. Each call needs its own Alice nonce; submit all in
	// parallel via `sign_and_submit` (no watcher) so the pool batches them into the next 1-2
	// blocks. Each worker gets headroom for 1 store + RENEWAL_CYCLES_TO_OBSERVE renewals + 1
	// safety cycle.
	let alice = dev::alice();
	let cycles_to_authorize = RENEWAL_CYCLES_TO_OBSERVE + 2;
	let pre_authz_block = current_best_block(&client).await?.number() as u64;
	log::info!(
		"Submitting {} sudo authorize_account calls in parallel (pre-block={})",
		WORST_CASE_WORKERS,
		pre_authz_block
	);
	let mut authz_futs = Vec::with_capacity(workers.len());
	for (i, worker) in workers.iter().enumerate() {
		let pubkey = worker.public_key().0;
		let bytes_per_worker = TEST_DATA_SIZE as u64 * cycles_to_authorize as u64;
		let sudo_call = tx(
			"Sudo",
			"sudo",
			vec![value! {
				TransactionStorage(authorize_account {
					who: Value::from_bytes(pubkey),
					transactions: cycles_to_authorize,
					bytes: bytes_per_worker
				})
			}],
		);
		// Immortal: subxt 0.44 defaults to mortal-for-32-blocks, but signing 512 txs
		// at the same head and validating them after a parachain fork at the
		// mortality height triggers `InvalidTransaction::BadProof` because the
		// canonical block hash at that height changes and `additional_signed`
		// no longer matches.
		let params = SubstrateExtrinsicParamsBuilder::new()
			.nonce(alice_nonce + i as u64)
			.immortal()
			.build();
		let signer = alice.clone();
		let cli = client.clone();
		authz_futs.push(async move {
			cli.tx()
				.sign_and_submit(&sudo_call, &signer, params)
				.await
				.map_err(anyhow::Error::from)
		});
	}
	alice_nonce += WORST_CASE_WORKERS as u64;
	let _: Vec<subxt::utils::H256> = futures::future::try_join_all(authz_futs).await?;
	log::info!("All {} sudo authorize_account calls accepted into pool", WORST_CASE_WORKERS);

	// Pre-fund every worker so they can pay the fee for `enable_auto_renew`.
	// `store` slips through `SkipCheckIfFeeless<ChargeTransactionPayment>` because
	// the bulletin authorization extension treats it as feeless, but
	// `enable_auto_renew` is fee-paying — workers without balance get rejected
	// at validate-time with `InvalidTransaction::Payment` ("Inability to pay
	// some fees"). 10 × ED is plenty for several txs.
	const WORKER_FUND: u128 = 1_000_000_000_000; // = 1 WND (1000 * EXISTENTIAL_DEPOSIT 1e9)
	log::info!(
		"Submitting {} Balances::transfer_keep_alive calls (Alice → workers) in parallel",
		WORST_CASE_WORKERS
	);
	let mut fund_futs = Vec::with_capacity(workers.len());
	for (i, worker) in workers.iter().enumerate() {
		let pubkey = worker.public_key().0;
		// MultiAddress::Id(account) — the AccountIdLookupOf<Runtime> shape.
		let dest_value = Value::unnamed_variant("Id", [Value::from_bytes(pubkey)]);
		let transfer_call =
			tx("Balances", "transfer_keep_alive", vec![dest_value, Value::u128(WORKER_FUND)]);
		let params = SubstrateExtrinsicParamsBuilder::new()
			.nonce(alice_nonce + i as u64)
			.immortal()
			.build();
		let signer = alice.clone();
		let cli = client.clone();
		fund_futs.push(async move {
			cli.tx()
				.sign_and_submit(&transfer_call, &signer, params)
				.await
				.map_err(anyhow::Error::from)
		});
	}
	let _: Vec<subxt::utils::H256> = futures::future::try_join_all(fund_futs).await?;
	log::info!("All {} transfer_keep_alive calls accepted into pool", WORST_CASE_WORKERS);

	// Wait for the last worker (worker_{N-1}) to actually receive their funding.
	// All Alice txs are nonce-ordered, so once the last transfer's recipient is
	// non-zero in System.Account, every earlier authorize+transfer must also
	// have settled. Poll up to BLOCK_PRODUCTION_TIMEOUT_SECS.
	{
		let last_worker_pubkey = workers[workers.len() - 1].public_key().0;
		let storage_addr = subxt::dynamic::storage(
			"System",
			"Account",
			vec![Value::from_bytes(last_worker_pubkey)],
		);
		let deadline = std::time::Instant::now() +
			std::time::Duration::from_secs(BLOCK_PRODUCTION_TIMEOUT_SECS);
		loop {
			let opt = client.storage().at_latest().await?.fetch(&storage_addr).await?;
			if opt.is_some() {
				break;
			}
			if std::time::Instant::now() >= deadline {
				anyhow::bail!(
					"timeout waiting for last worker's funding to land (Alice's batched txs not all included)"
				);
			}
			tokio::time::sleep(std::time::Duration::from_secs(2)).await;
		}
		log::info!("Last worker funded (System.Account exists) — all Alice batched txs settled");
	}

	// Generate distinct payloads, one per worker.
	let items: Vec<Vec<u8>> = (0..WORST_CASE_WORKERS)
		.map(|i| {
			let mut pattern = b"WORST_CASE_WORKER_".to_vec();
			pattern.extend_from_slice(format!("{:04}_", i).as_bytes());
			generate_test_data(TEST_DATA_SIZE, &pattern)
		})
		.collect();
	let content_hashes: Vec<[u8; 32]> = items.iter().map(|d| blake2_256(d)).collect();

	// Each worker submits a store (their nonce 0). All in parallel.
	let pre_store_block = current_best_block(&client).await?.number() as u64;
	log::info!(
		"Submitting {} signed stores in parallel (pre-store block={})",
		WORST_CASE_WORKERS,
		pre_store_block
	);
	let mut store_futs = Vec::with_capacity(workers.len());
	for (worker, data) in workers.iter().zip(items.iter()) {
		let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
		// Each fresh worker account starts at nonce 0.
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(0).immortal().build();
		let signer = worker.clone();
		let cli = client.clone();
		store_futs.push(async move {
			cli.tx()
				.sign_and_submit(&store_call, &signer, params)
				.await
				.map_err(anyhow::Error::from)
		});
	}
	let _: Vec<subxt::utils::H256> = futures::future::try_join_all(store_futs).await?;
	log::info!("All {} stores accepted into pool", WORST_CASE_WORKERS);

	let store_inclusion_target = pre_store_block + 5;
	wait_for_block_height(collator1, store_inclusion_target, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// Walk back to find the actual blocks each store landed in.
	let post_store_head_n = current_best_block(&client).await?.number() as u64;
	let mut store_blocks: Vec<u64> = Vec::with_capacity(items.len());
	{
		let mut current = current_best_block(&client).await?;
		while current.number() as u64 > pre_store_block {
			let block_n = current.number() as u64;
			let events = current.events().await?;
			let stored_count = events
				.iter()
				.filter_map(|e| e.ok())
				.filter(|e| e.pallet_name() == "TransactionStorage" && e.variant_name() == "Stored")
				.count();
			for _ in 0..stored_count {
				store_blocks.push(block_n);
			}
			if block_n == 0 {
				break;
			}
			let parent_hash = current.header().parent_hash;
			current = client.blocks().at(parent_hash).await?;
		}
	}
	if store_blocks.len() != items.len() {
		anyhow::bail!(
			"Expected {} Stored events between blocks {}..={}, found {}",
			items.len(),
			pre_store_block + 1,
			post_store_head_n,
			store_blocks.len()
		);
	}
	let earliest_store = *store_blocks.iter().min().unwrap();
	let latest_store = *store_blocks.iter().max().unwrap();
	let mut store_block_histogram: HashMap<u64, u32> = HashMap::new();
	for b in &store_blocks {
		*store_block_histogram.entry(*b).or_default() += 1;
	}
	log::info!(
		"Stored {} items across blocks {}..={} ({} distinct blocks)",
		WORST_CASE_WORKERS,
		earliest_store,
		latest_store,
		store_block_histogram.len()
	);

	// Each worker enables auto-renew on its own content_hash (their nonce 1).
	let pre_enable_block = current_best_block(&client).await?.number() as u64;
	let mut enable_futs = Vec::with_capacity(content_hashes.len());
	for (worker, hash) in workers.iter().zip(content_hashes.iter()) {
		let call =
			tx("TransactionStorage", "enable_auto_renew", vec![Value::from_bytes(hash.as_slice())]);
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(1).immortal().build();
		let signer = worker.clone();
		let cli = client.clone();
		enable_futs.push(async move {
			cli.tx()
				.sign_and_submit(&call, &signer, params)
				.await
				.map_err(anyhow::Error::from)
		});
	}
	let _: Vec<subxt::utils::H256> = futures::future::try_join_all(enable_futs).await?;
	log::info!("All {} enable_auto_renew calls accepted into pool", WORST_CASE_WORKERS);

	let enable_inclusion_target = pre_enable_block + 5;
	wait_for_block_height(collator1, enable_inclusion_target, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await?;

	// === Renewal-window observation (mirrors parachain_auto_renew_many_items_test) ===
	let renewal_cadence = RETENTION_PERIOD as u64 + 1;
	let first_renewal_block = earliest_store + renewal_cadence;
	let last_renewal_block = latest_store + renewal_cadence * RENEWAL_CYCLES_TO_OBSERVE as u64;
	let wait_until = last_renewal_block + 1;
	log::info!(
		"Renewal window: {}..={}; capturing per-block prometheus snapshots up to {}",
		first_renewal_block,
		last_renewal_block,
		wait_until
	);

	let snapshot_range_start = first_renewal_block.saturating_sub(3).max(1);
	let mut prom_snapshots: Vec<(u64, f64, f64)> = Vec::new();
	for n in snapshot_range_start..=wait_until {
		wait_for_block_height(collator1, n, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
		let sum = collator1
			.reports("substrate_proposer_block_constructed_sum".to_string())
			.await
			.map_err(|e| anyhow::anyhow!("read prom sum: {e}"))?;
		let count = collator1
			.reports("substrate_proposer_block_constructed_count".to_string())
			.await
			.map_err(|e| anyhow::anyhow!("read prom count: {e}"))?;
		prom_snapshots.push((n, sum, count));
	}

	log::info!("--- Per-block proposer block_constructed (wall-clock construction time) ---");
	log::info!("Format: blocks (a..=b]: +N blocks, +T s sum, ~ms/block");
	for win in prom_snapshots.windows(2) {
		let (n0, sum0, count0) = win[0];
		let (n1, sum1, count1) = win[1];
		let delta_sum = sum1 - sum0;
		let delta_count = count1 - count0;
		let ms_per_block = if delta_count > 0.0 { delta_sum * 1000.0 / delta_count } else { 0.0 };
		let marker = if n1 == first_renewal_block - 1 {
			" <-- proof-only block"
		} else if n1 >= first_renewal_block && n1 <= last_renewal_block {
			" <-- renewal block"
		} else {
			""
		};
		log::info!(
			"blocks ({}..={}]: +{} blocks, +{:.4} s sum, ~{:.1} ms/block{}",
			n0,
			n1,
			delta_count as u64,
			delta_sum,
			ms_per_block,
			marker
		);
	}

	let head = {
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(&client).await?;
			if head.number() as u64 >= wait_until {
				break head;
			}
			if start.elapsed() > poll_timeout {
				anyhow::bail!(
					"Timed out waiting for at_latest() to see block {} (last seen: {})",
					wait_until,
					head.number()
				);
			}
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
		}
	};
	let stats_range_start = first_renewal_block.saturating_sub(15).max(1);
	let stats_range_end = last_renewal_block + 2;

	let mut block_hashes_by_number: HashMap<u64, subxt::utils::H256> = HashMap::new();
	let mut current = head;
	loop {
		let n = current.number() as u64;
		if n < stats_range_start {
			break;
		}
		block_hashes_by_number.insert(n, current.hash());
		if n == 0 {
			break;
		}
		let parent_hash = current.header().parent_hash;
		current = client.blocks().at(parent_hash).await?;
	}

	let block_weight_addr = subxt::dynamic::storage("System", "BlockWeight", Vec::<Value>::new());
	let (max_block_ref, max_block_pov) = fetch_max_block_weight(&client).await?;
	log::info!(
		"BlockWeights::max_block = (ref_time={}, proof_size={})",
		max_block_ref,
		max_block_pov
	);

	log::info!("--- Block weight stats ---");
	log::info!("Format: block N | extrinsics={{n}} DataAutoRenewed={{n}} AutoRenewalFailed={{n}} | normal=(ref_time,proof_size) op=(...) mand=(...)");

	let mut total_renewed: u32 = 0;
	let mut weight_violations: Vec<String> = Vec::new();
	for block_n in stats_range_start..=stats_range_end {
		let Some(&block_hash) = block_hashes_by_number.get(&block_n) else {
			continue;
		};
		let block = client.blocks().at(block_hash).await?;
		let extrinsic_count = block.extrinsics().await?.iter().count();
		let events = block.events().await?;
		let auto_renewed: u32 = events
			.iter()
			.filter_map(|e| e.ok())
			.filter(|e| {
				e.pallet_name() == "TransactionStorage" && e.variant_name() == "DataAutoRenewed"
			})
			.count() as u32;
		let auto_renewal_failed: u32 = events
			.iter()
			.filter_map(|e| e.ok())
			.filter(|e| {
				e.pallet_name() == "TransactionStorage" && e.variant_name() == "AutoRenewalFailed"
			})
			.count() as u32;
		let weight_value = client
			.storage()
			.at(block_hash)
			.fetch(&block_weight_addr)
			.await?
			.map(|v| v.to_value())
			.transpose()?;
		let (normal, op, mand) = match weight_value.as_ref() {
			Some(v) => (
				extract_class_weight(v, "normal"),
				extract_class_weight(v, "operational"),
				extract_class_weight(v, "mandatory"),
			),
			None => ((0, 0), (0, 0), (0, 0)),
		};
		let marker = if block_n == first_renewal_block - 1 {
			" <-- proof-only block"
		} else if block_n >= first_renewal_block && block_n <= last_renewal_block {
			" <-- renewal block"
		} else if store_block_histogram.contains_key(&block_n) {
			" <-- store block"
		} else {
			""
		};
		log::info!(
			"block {:>3} | xt={:>2} renewed={:>3} failed={:>3} | normal=({:>13},{:>9}) op=({:>13},{:>9}) mand=({:>13},{:>9}){}",
			block_n,
			extrinsic_count,
			auto_renewed,
			auto_renewal_failed,
			normal.0,
			normal.1,
			op.0,
			op.1,
			mand.0,
			mand.1,
			marker
		);
		if mand.0 > max_block_ref {
			weight_violations.push(format!(
				"block {block_n}: mandatory ref_time={} exceeds max_block ref_time={}",
				mand.0, max_block_ref
			));
		}
		if mand.1 > max_block_pov {
			weight_violations.push(format!(
				"block {block_n}: mandatory proof_size={} exceeds max_block proof_size={}",
				mand.1, max_block_pov
			));
		}
		total_renewed += auto_renewed;
	}
	if !weight_violations.is_empty() {
		anyhow::bail!(
			"apply_block_inherents exceeded BlockWeights::max_block on {} block(s):\n  {}",
			weight_violations.len(),
			weight_violations.join("\n  ")
		);
	}

	let expected_total = WORST_CASE_WORKERS * RENEWAL_CYCLES_TO_OBSERVE;
	log::info!("Total renewals across window: {} / {}", total_renewed, expected_total);
	if total_renewed < expected_total {
		anyhow::bail!(
			"Expected at least {} DataAutoRenewed events, saw {}",
			expected_total,
			total_renewed
		);
	}

	test_log!(TEST, "=== Worst-case auto-renew {} items PASSED ===", WORST_CASE_WORKERS);

	// Optional post-PASS hold for manual inspection of the live network via PJS.
	// Off by default (keeps CI fast); set `INSPECT_HOLD_SECS=N` (e.g. `1800` for
	// 30 min) to keep the network up for `N` seconds after the assertions pass.
	let inspect_hold_secs: u64 = std::env::var("INSPECT_HOLD_SECS")
		.ok()
		.and_then(|s| s.parse().ok())
		.unwrap_or(0);
	if inspect_hold_secs > 0 {
		log::info!(
			"[para_auto_renew_many_worst_case] Holding network up for {} seconds — open the PJS link printed by collator-1 above to inspect block weights. Ctrl-C the test process to exit early.",
			inspect_hold_secs,
		);
		tokio::time::sleep(std::time::Duration::from_secs(inspect_hold_secs)).await;
	}

	network.destroy().await?;
	Ok(())
}

// ===========================================================================================
// on_initialize behavioral tests
// ===========================================================================================

/// Number of items per set (auto-renew vs no-auto-renew) in
/// [`parachain_on_initialize_cleanup_test`]. 50 + 50 = 100 total stores; small enough that
/// the test runs in ~5 minutes, large enough that the differential cleanup is observable.
const ON_INIT_CLEANUP_ITEMS_PER_SET: u32 = 50;

/// Assert that `Hooks::on_initialize` cleans up state correctly across the auto-renewal vs
/// no-auto-renewal discriminator at the retention boundary.
///
/// Setup:
/// - Single collator, archive pruning, `RetentionPeriod = 10`.
/// - Store [`ON_INIT_CLEANUP_ITEMS_PER_SET`] items WITH `enable_auto_renew` (set 1).
/// - Store [`ON_INIT_CLEANUP_ITEMS_PER_SET`] items WITHOUT `enable_auto_renew` (set 2).
///
/// At block `store_block + RP + 1`, on_initialize takes `Transactions[store_block]`,
/// iterates all entries, and for each:
/// - reads `TransactionByContentHash[hash]` and removes it iff it still points at `store_block`
///   (the cleanup branch);
/// - reads `AutoRenewals[hash]` and pushes to `PendingAutoRenewals` iff registered.
///
/// Assertions after the expiry block lands:
/// - `Transactions[store_block]` is `None` (taken by on_initialize, never re-added).
/// - For each set-1 hash: `TransactionByContentHash[hash]` points at the expiry block —
///   `apply_block_inherents` ran the drain and `do_renew` updated the index to the new block.
/// - For each set-2 hash: `TransactionByContentHash[hash]` is `None` — on_initialize's cleanup
///   branch removed it.
/// - Exactly `ON_INIT_CLEANUP_ITEMS_PER_SET` `DataAutoRenewed` events emitted at the expiry block;
///   zero `AutoRenewalFailed` events.
/// - `mand` weight at the expiry block is `≤ BlockWeights::max_block`.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_on_initialize_cleanup_test() -> Result<()> {
	const TEST: &str = "para_on_init_cleanup";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(
		TEST,
		"=== on_initialize cleanup ({} auto-renew + {} non-auto-renew items) ===",
		ON_INIT_CLEANUP_ITEMS_PER_SET,
		ON_INIT_CLEANUP_ITEMS_PER_SET,
	);

	verify_parachain_binaries()?;
	let config = build_parachain_network_config_single_collator(get_para_node_args())?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;
	let relay_alice = network.get_node("alice").context("relay alice")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;
	let collator1 = network.get_node("collator-1").context("collator-1")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	set_retention_period(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// Authorize Alice for 1 store + 1 renewal per set-1 item, with safety margin.
	let bytes_per_item = TEST_DATA_SIZE as u64;
	let total_items = ON_INIT_CLEANUP_ITEMS_PER_SET * 2;
	authorize_account_via_sudo(
		&client,
		&dev::alice().public_key().0,
		total_items * 4,
		bytes_per_item * (total_items as u64) * 4,
		nonce,
	)
	.await?;
	nonce += 1;

	// Generate two distinct sets of payloads.
	let set1: Vec<Vec<u8>> = (0..ON_INIT_CLEANUP_ITEMS_PER_SET)
		.map(|i| {
			let mut p = b"ON_INIT_CLEANUP_RENEW_".to_vec();
			p.extend_from_slice(format!("{:04}_", i).as_bytes());
			generate_test_data(TEST_DATA_SIZE, &p)
		})
		.collect();
	let set2: Vec<Vec<u8>> = (0..ON_INIT_CLEANUP_ITEMS_PER_SET)
		.map(|i| {
			let mut p = b"ON_INIT_CLEANUP_NORENEW_".to_vec();
			p.extend_from_slice(format!("{:04}_", i).as_bytes());
			generate_test_data(TEST_DATA_SIZE, &p)
		})
		.collect();
	let set1_hashes: Vec<[u8; 32]> = set1.iter().map(|d| blake2_256(d)).collect();
	let set2_hashes: Vec<[u8; 32]> = set2.iter().map(|d| blake2_256(d)).collect();

	// Submit all stores to the pool in parallel.
	let alice = dev::alice();
	let pre_store_block = current_best_block(&client).await?.number() as u64;
	log::info!(
		"Submitting {} stores ({} for renewal + {} for cleanup); pre-store block={}",
		total_items,
		ON_INIT_CLEANUP_ITEMS_PER_SET,
		ON_INIT_CLEANUP_ITEMS_PER_SET,
		pre_store_block
	);
	let mut futs = Vec::with_capacity(total_items as usize);
	let mut idx = 0;
	for data in set1.iter().chain(set2.iter()) {
		let call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce + idx).build();
		let signer = alice.clone();
		let cli = client.clone();
		futs.push(async move {
			cli.tx()
				.sign_and_submit(&call, &signer, params)
				.await
				.map_err(anyhow::Error::from)
		});
		idx += 1;
	}
	nonce += total_items as u64;
	let _: Vec<subxt::utils::H256> = futures::future::try_join_all(futs).await?;
	log::info!("All {} stores accepted into pool", total_items);

	// Wait until inclusion has settled, then walk backwards to find which block each set landed in.
	wait_for_block_height(collator1, pre_store_block + 5, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// All stores submitted in parallel land in 1-2 blocks. We just need the earliest.
	let mut store_block: u64 = 0;
	{
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(&client).await?;
			if head.number() as u64 >= pre_store_block + 5 {
				break;
			}
			if start.elapsed() > poll_timeout {
				anyhow::bail!("Timed out waiting for at_latest >= {}", pre_store_block + 5);
			}
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
		}
	}
	{
		let mut current = current_best_block(&client).await?;
		while current.number() as u64 > pre_store_block {
			let block_n = current.number() as u64;
			let events = current.events().await?;
			let stored_count = events
				.iter()
				.filter_map(|e| e.ok())
				.filter(|e| e.pallet_name() == "TransactionStorage" && e.variant_name() == "Stored")
				.count();
			if stored_count > 0 {
				store_block = block_n;
			}
			if block_n == 0 {
				break;
			}
			let parent_hash = current.header().parent_hash;
			current = client.blocks().at(parent_hash).await?;
		}
	}
	if store_block == 0 {
		anyhow::bail!("could not locate Stored events for the {} stores", total_items);
	}
	log::info!("Stores landed at (or before) block {}", store_block);

	// Enable auto-renew for set 1 only.
	log::info!("Enabling auto-renew for {} items (set 1)", ON_INIT_CLEANUP_ITEMS_PER_SET);
	let mut futs = Vec::with_capacity(ON_INIT_CLEANUP_ITEMS_PER_SET as usize);
	for (i, content_hash) in set1_hashes.iter().enumerate() {
		let call = tx(
			"TransactionStorage",
			"enable_auto_renew",
			vec![Value::from_bytes(content_hash.as_slice())],
		);
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce + i as u64).build();
		let signer = alice.clone();
		let cli = client.clone();
		futs.push(async move {
			cli.tx()
				.sign_and_submit(&call, &signer, params)
				.await
				.map_err(anyhow::Error::from)
		});
	}
	nonce += ON_INIT_CLEANUP_ITEMS_PER_SET as u64;
	let _: Vec<subxt::utils::H256> = futures::future::try_join_all(futs).await?;
	let _ = nonce;

	// Wait until the expiry block is finalized.
	let expiry_block = store_block + RETENTION_PERIOD as u64 + 1;
	log::info!(
		"Waiting for expiry block {} (= store_block {} + RP {} + 1)",
		expiry_block,
		store_block,
		RETENTION_PERIOD
	);
	wait_for_block_height(collator1, expiry_block + 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	{
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(&client).await?;
			if head.number() as u64 >= expiry_block + 1 {
				break;
			}
			if start.elapsed() > poll_timeout {
				anyhow::bail!("Timed out waiting for at_latest >= {}", expiry_block + 1);
			}
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
		}
	}

	// Resolve the expiry block's hash for storage queries.
	let expiry_hash = {
		let mut current = current_best_block(&client).await?;
		while (current.number() as u64) > expiry_block {
			let parent_hash = current.header().parent_hash;
			current = client.blocks().at(parent_hash).await?;
		}
		assert_eq!(current.number() as u64, expiry_block);
		current.hash()
	};

	// Assertion 1: Transactions[store_block] should be None at the expiry block (was taken
	// by on_initialize).
	{
		let addr = subxt::dynamic::storage(
			"TransactionStorage",
			"Transactions",
			vec![Value::u128(store_block as u128)],
		);
		let value = client.storage().at(expiry_hash).fetch(&addr).await?;
		assert!(
			value.is_none(),
			"Transactions[{}] should be None at expiry block {} (on_initialize should have taken it)",
			store_block,
			expiry_block
		);
		log::info!("✓ Transactions[{}] is None at expiry block {}", store_block, expiry_block);
	}

	// Assertion 2: set-1 hashes (auto-renew) — TransactionByContentHash points at expiry_block.
	let mut set1_renewed = 0u32;
	for (i, hash) in set1_hashes.iter().enumerate() {
		let addr = subxt::dynamic::storage(
			"TransactionStorage",
			"TransactionByContentHash",
			vec![Value::from_bytes(hash.as_slice())],
		);
		let value = client.storage().at(expiry_hash).fetch(&addr).await?;
		match value {
			Some(v) => {
				let v = v.to_value()?;
				// Decoded as a tuple (block_number, index). We just check it's Some.
				log::trace!("set1[{}] hash={} → {:?}", i, hex::encode(hash), v);
				set1_renewed += 1;
			},
			None => {
				log::warn!(
					"set1[{}] hash={} → None at expiry block (expected renewed)",
					i,
					hex::encode(hash)
				);
			},
		}
	}
	assert_eq!(
		set1_renewed, ON_INIT_CLEANUP_ITEMS_PER_SET,
		"all {} set-1 (auto-renew) items should still have a TransactionByContentHash entry at expiry; \
		 only {} do",
		ON_INIT_CLEANUP_ITEMS_PER_SET, set1_renewed
	);
	log::info!("✓ All {} set-1 (auto-renew) items still indexed at expiry block", set1_renewed);

	// Assertion 3: set-2 hashes (no auto-renew) — TransactionByContentHash should be None.
	let mut set2_cleaned = 0u32;
	for (i, hash) in set2_hashes.iter().enumerate() {
		let addr = subxt::dynamic::storage(
			"TransactionStorage",
			"TransactionByContentHash",
			vec![Value::from_bytes(hash.as_slice())],
		);
		let value = client.storage().at(expiry_hash).fetch(&addr).await?;
		match value {
			None => {
				set2_cleaned += 1;
			},
			Some(v) => {
				let v = v.to_value()?;
				log::warn!(
					"set2[{}] hash={} → {:?} at expiry block (expected cleaned up)",
					i,
					hex::encode(hash),
					v
				);
			},
		}
	}
	assert_eq!(
		set2_cleaned, ON_INIT_CLEANUP_ITEMS_PER_SET,
		"all {} set-2 (no auto-renew) items' TransactionByContentHash should be removed by \
		 on_initialize; {} were cleaned up",
		ON_INIT_CLEANUP_ITEMS_PER_SET, set2_cleaned
	);
	log::info!(
		"✓ All {} set-2 (no auto-renew) TransactionByContentHash entries cleaned up",
		set2_cleaned
	);

	// Assertion 4: events at the expiry block — exactly N DataAutoRenewed, zero AutoRenewalFailed.
	let expiry_block_obj = client.blocks().at(expiry_hash).await?;
	let events = expiry_block_obj.events().await?;
	let auto_renewed = events
		.iter()
		.filter_map(|e| e.ok())
		.filter(|e| {
			e.pallet_name() == "TransactionStorage" && e.variant_name() == "DataAutoRenewed"
		})
		.count() as u32;
	let auto_renewal_failed = events
		.iter()
		.filter_map(|e| e.ok())
		.filter(|e| {
			e.pallet_name() == "TransactionStorage" && e.variant_name() == "AutoRenewalFailed"
		})
		.count() as u32;
	assert_eq!(
		auto_renewed, ON_INIT_CLEANUP_ITEMS_PER_SET,
		"expected {} DataAutoRenewed events at expiry block {}, saw {}",
		ON_INIT_CLEANUP_ITEMS_PER_SET, expiry_block, auto_renewed
	);
	assert_eq!(
		auto_renewal_failed, 0,
		"expected 0 AutoRenewalFailed events at expiry block {}, saw {}",
		expiry_block, auto_renewal_failed
	);
	log::info!(
		"✓ {} DataAutoRenewed events at expiry block {} (and zero AutoRenewalFailed)",
		auto_renewed,
		expiry_block
	);

	// Assertion 5: mandatory weight ≤ max_block at expiry block.
	let (max_block_ref, max_block_pov) = fetch_max_block_weight(&client).await?;
	let block_weight_addr = subxt::dynamic::storage("System", "BlockWeight", Vec::<Value>::new());
	let weight_value = client
		.storage()
		.at(expiry_hash)
		.fetch(&block_weight_addr)
		.await?
		.map(|v| v.to_value())
		.transpose()?;
	let mand = match weight_value.as_ref() {
		Some(v) => extract_class_weight(v, "mandatory"),
		None => (0, 0),
	};
	log::info!(
		"Expiry block {}: mand=({}, {}); max_block=({}, {})",
		expiry_block,
		mand.0,
		mand.1,
		max_block_ref,
		max_block_pov
	);
	assert!(
		mand.0 <= max_block_ref,
		"mandatory ref_time {} at expiry block exceeds max_block ref_time {}",
		mand.0,
		max_block_ref
	);
	assert!(
		mand.1 <= max_block_pov,
		"mandatory proof_size {} at expiry block exceeds max_block proof_size {}",
		mand.1,
		max_block_pov
	);
	log::info!("✓ mandatory weight at expiry block within max_block");

	test_log!(TEST, "=== on_initialize cleanup PASSED ===");
	network.destroy().await?;
	Ok(())
}

/// Number of items used by [`parachain_on_initialize_no_renewals_weight_test`]. Set to the
/// pallet's `MaxBlockTransactions` cap so the weight reading reflects the worst-case
/// expiry sweep.
const ON_INIT_NO_RENEWALS_ITEMS: u32 = 512;

/// Isolate `Hooks::on_initialize` cost from `apply_block_inherents` drain cost. Stores
/// [`ON_INIT_NO_RENEWALS_ITEMS`] items WITHOUT enabling auto-renewal, so at the expiry
/// block:
/// - on_initialize takes `Transactions[store_block]` and iterates every entry through the
///   discriminator (which always finds `AutoRenewals[hash] = None` and so does NOT push to
///   pending);
/// - `apply_block_inherents` is emitted with `proof = Some, pending = empty` → drain runs over an
///   empty queue (essentially free).
///
/// `mand` at this block is therefore `on_init_with_expiry_extra + apply_proof_only +
/// constant per-block mandatory contributions`. Subtracting `mand` at an idle baseline
/// block (no expiry, no proof needed) cancels the constant contributions and gives:
///
///   `on_init_with_expiry_extra + apply_proof_only`
///
/// The proof-only portion is approximately constant (~0.9 G ref_time on Westend per the
/// test #6 measurements). This test does NOT subtract it — it just logs the difference and
/// asserts the total fits within `max_block`. The diagnostic value is in seeing the
/// magnitude of `on_initialize_with_expiry(MAX)` empirically, which the in-pallet
/// `ensure_weight_sanity` test only validates against the declared (placeholder) weight.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_on_initialize_no_renewals_weight_test() -> Result<()> {
	const TEST: &str = "para_on_init_no_renewals";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(
		TEST,
		"=== on_initialize cost in isolation ({} items, no auto-renew) ===",
		ON_INIT_NO_RENEWALS_ITEMS
	);

	verify_parachain_binaries()?;
	let config = build_parachain_network_config_single_collator(get_para_node_args())?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;
	let relay_alice = network.get_node("alice").context("relay alice")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;
	let collator1 = network.get_node("collator-1").context("collator-1")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	set_retention_period(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;
	let bytes_per_item = TEST_DATA_SIZE as u64;
	authorize_account_via_sudo(
		&client,
		&dev::alice().public_key().0,
		ON_INIT_NO_RENEWALS_ITEMS * 2,
		bytes_per_item * (ON_INIT_NO_RENEWALS_ITEMS as u64) * 2,
		nonce,
	)
	.await?;
	nonce += 1;

	// Generate ON_INIT_NO_RENEWALS_ITEMS distinct payloads.
	let items: Vec<Vec<u8>> = (0..ON_INIT_NO_RENEWALS_ITEMS)
		.map(|i| {
			let mut p = b"ON_INIT_NO_RENEW_".to_vec();
			p.extend_from_slice(format!("{:04}_", i).as_bytes());
			generate_test_data(TEST_DATA_SIZE, &p)
		})
		.collect();

	// Submit all stores to the pool in parallel.
	let alice = dev::alice();
	let pre_store_block = current_best_block(&client).await?.number() as u64;
	log::info!(
		"Submitting {} stores in parallel (no enable_auto_renew); pre-store block={}",
		ON_INIT_NO_RENEWALS_ITEMS,
		pre_store_block
	);
	let mut futs = Vec::with_capacity(items.len());
	for (i, data) in items.iter().enumerate() {
		let call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce + i as u64).build();
		let signer = alice.clone();
		let cli = client.clone();
		futs.push(async move {
			cli.tx()
				.sign_and_submit(&call, &signer, params)
				.await
				.map_err(anyhow::Error::from)
		});
	}
	let _ = nonce;
	let _: Vec<subxt::utils::H256> = futures::future::try_join_all(futs).await?;
	log::info!("All {} stores accepted into pool", ON_INIT_NO_RENEWALS_ITEMS);

	// Find the store block.
	wait_for_block_height(collator1, pre_store_block + 5, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	{
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(&client).await?;
			if head.number() as u64 >= pre_store_block + 5 {
				break;
			}
			if start.elapsed() > poll_timeout {
				anyhow::bail!("Timed out waiting for at_latest >= {}", pre_store_block + 5);
			}
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
		}
	}
	let mut store_block: u64 = 0;
	{
		let mut current = current_best_block(&client).await?;
		while current.number() as u64 > pre_store_block {
			let block_n = current.number() as u64;
			let events = current.events().await?;
			let stored_count = events
				.iter()
				.filter_map(|e| e.ok())
				.filter(|e| e.pallet_name() == "TransactionStorage" && e.variant_name() == "Stored")
				.count();
			if stored_count > 0 {
				store_block = block_n;
			}
			if block_n == 0 {
				break;
			}
			let parent_hash = current.header().parent_hash;
			current = client.blocks().at(parent_hash).await?;
		}
	}
	if store_block == 0 {
		anyhow::bail!("could not locate Stored events");
	}
	log::info!("Stores landed at block {}", store_block);

	// Wait past the expiry block.
	let expiry_block = store_block + RETENTION_PERIOD as u64 + 1;
	wait_for_block_height(collator1, expiry_block + 2, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// Walk the chain backward to record block hashes for the window we want to inspect.
	let stats_range_start = store_block.saturating_sub(2).max(1);
	let stats_range_end = expiry_block + 2;
	let head = {
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(&client).await?;
			if head.number() as u64 >= stats_range_end {
				break head;
			}
			if start.elapsed() > poll_timeout {
				anyhow::bail!(
					"Timed out waiting for at_latest >= {} (last seen: {})",
					stats_range_end,
					head.number()
				);
			}
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
		}
	};
	let mut block_hashes_by_number: HashMap<u64, subxt::utils::H256> = HashMap::new();
	let mut current = head;
	loop {
		let n = current.number() as u64;
		if n < stats_range_start {
			break;
		}
		block_hashes_by_number.insert(n, current.hash());
		if n == 0 {
			break;
		}
		let parent_hash = current.header().parent_hash;
		current = client.blocks().at(parent_hash).await?;
	}

	let block_weight_addr = subxt::dynamic::storage("System", "BlockWeight", Vec::<Value>::new());
	let (max_block_ref, max_block_pov) = fetch_max_block_weight(&client).await?;
	log::info!(
		"BlockWeights::max_block = (ref_time={}, proof_size={})",
		max_block_ref,
		max_block_pov
	);

	let mut idle_baseline_ref: Option<u64> = None;
	let mut idle_baseline_pov: Option<u64> = None;
	let mut expiry_mand: Option<(u64, u64)> = None;
	let mut weight_violations: Vec<String> = Vec::new();

	log::info!("--- Block weight stats (no auto-renewals) ---");
	for block_n in stats_range_start..=stats_range_end {
		let Some(&block_hash) = block_hashes_by_number.get(&block_n) else { continue };
		let block = client.blocks().at(block_hash).await?;
		let extrinsic_count = block.extrinsics().await?.iter().count();
		let weight_value = client
			.storage()
			.at(block_hash)
			.fetch(&block_weight_addr)
			.await?
			.map(|v| v.to_value())
			.transpose()?;
		let (normal, op, mand) = match weight_value.as_ref() {
			Some(v) => (
				extract_class_weight(v, "normal"),
				extract_class_weight(v, "operational"),
				extract_class_weight(v, "mandatory"),
			),
			None => ((0, 0), (0, 0), (0, 0)),
		};

		let marker = if block_n == expiry_block {
			" <-- expiry block (on_initialize sweeps Transactions[store_block])"
		} else if block_n == store_block {
			" <-- store block"
		} else {
			""
		};

		log::info!(
			"block {:>3} | xt={:>2} | normal=({:>13},{:>9}) op=({:>13},{:>9}) mand=({:>13},{:>9}){}",
			block_n,
			extrinsic_count,
			normal.0,
			normal.1,
			op.0,
			op.1,
			mand.0,
			mand.1,
			marker
		);

		// Bound assertion.
		if mand.0 > max_block_ref {
			weight_violations.push(format!(
				"block {block_n}: mandatory ref_time={} exceeds max_block ref_time={}",
				mand.0, max_block_ref
			));
		}
		if mand.1 > max_block_pov {
			weight_violations.push(format!(
				"block {block_n}: mandatory proof_size={} exceeds max_block proof_size={}",
				mand.1, max_block_pov
			));
		}

		// Capture isolation values: idle baseline = block AFTER expiry (no expiry, no
		// pending, no proof needed at that block since target is store_block + 1 which
		// has nothing).
		if block_n == expiry_block + 2 {
			idle_baseline_ref = Some(mand.0);
			idle_baseline_pov = Some(mand.1);
		}
		if block_n == expiry_block {
			expiry_mand = Some((mand.0, mand.1));
		}
	}

	if !weight_violations.is_empty() {
		anyhow::bail!(
			"mandatory weight exceeded BlockWeights::max_block on {} block(s):\n  {}",
			weight_violations.len(),
			weight_violations.join("\n  ")
		);
	}

	// Diagnostic: empirical on_init+proof cost = expiry_mand - idle_baseline.
	if let (Some(em), Some(idle_ref), Some(idle_pov)) =
		(expiry_mand, idle_baseline_ref, idle_baseline_pov)
	{
		let delta_ref = em.0.saturating_sub(idle_ref);
		let delta_pov = em.1.saturating_sub(idle_pov);
		log::info!(
			"On-chain on_initialize_with_expiry({}) + proof-only delta (vs idle baseline): \
			 ref_time={} ({:.3}% of max_block.ref_time), proof_size={} ({:.3}% of max_block.proof_size)",
			ON_INIT_NO_RENEWALS_ITEMS,
			delta_ref,
			(delta_ref as f64) * 100.0 / (max_block_ref as f64),
			delta_pov,
			if max_block_pov == u64::MAX {
				0.0
			} else {
				(delta_pov as f64) * 100.0 / (max_block_pov as f64)
			},
		);
	}

	test_log!(TEST, "=== on_initialize cost isolation PASSED ===");
	network.destroy().await?;
	Ok(())
}

// ===========================================================================================
// Long-running pruning soak test
// ===========================================================================================

/// 60 minutes wall-clock soak.
const SOAK_DURATION_SECS: u64 = 60 * 60;
/// `--blocks-pruning` for soak collators. Larger than retention, so the chain progresses
/// normally; pruning only kicks in once a block has aged past 15.
const SOAK_BLOCKS_PRUNING: u32 = 15;
/// Pallet retention period. Items not renewed within this window have their pallet-state
/// (`Transactions[N]`, `TransactionByContentHash[hash]`) cleared at `N + RP + 1`.
const SOAK_RETENTION_PERIOD: u32 = 10;
/// Verification cadence — every N produced blocks, sample one item that should be pruned and
/// confirm bitswap returns `DONT_HAVE`.
const SOAK_VERIFY_INTERVAL_BLOCKS: u64 = 30;
/// Minimum blocks since an item's last touch (store or renew) before we expect col11 eviction.
/// `pruning(15) + retention/finality_lag(10) = 25` is a comfortable lower bound.
const SOAK_PRUNED_AGE_THRESHOLD: u64 = 25;
/// Tighter bitswap timeout for soak verification — keeps the per-cycle overhead under 10s
/// even when DONT_HAVE takes a moment to round-trip.
const SOAK_BITSWAP_TIMEOUT_SECS: u64 = 10;
/// Pre-authorize generously so we never run out during the 60-min soak.
/// 60 min × ~10 blocks/min × 1.5 ops/block = ~900 ops; 3× safety = 2700; round to 3000.
const SOAK_AUTH_TX_SLOTS: u32 = 3000;

#[derive(Clone)]
struct SoakItem {
	data: Vec<u8>,
	content_hash: [u8; 32],
	last_touch_block: u64,
}

/// xorshift PRNG seeded by block number for deterministic-but-varied test choices.
fn pseudo_random(seed: u64) -> u64 {
	let mut x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
	x ^= x << 13;
	x ^= x >> 7;
	x ^= x << 17;
	x
}

/// Long-running soak test on a 3-collator parachain network with `--blocks-pruning=15`
/// (greater than `RetentionPeriod=10`). Drives a steady stream of `store` and
/// `renew_content_hash` extrinsics via collator-1, then periodically verifies that data
/// older than the pruning window is no longer served via bitswap.
///
/// Runs for [`SOAK_DURATION_SECS`] (60 min). Logs PJS links for each collator at startup so
/// you can attach polkadot.js Apps in a browser to watch live.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_long_running_pruning_soak_test() -> Result<()> {
	const TEST: &str = "para_pruning_soak";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(
		TEST,
		"=== Long-running pruning soak test ({} min, pruning={}, RP={}) ===",
		SOAK_DURATION_SECS / 60,
		SOAK_BLOCKS_PRUNING,
		SOAK_RETENTION_PERIOD
	);

	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(SOAK_BLOCKS_PRUNING);
	let config = crate::utils::build_parachain_network_config_three_collators(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get alice")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1")?;
	let collator2 = network.get_node("collator-2").context("Failed to get collator-2")?;
	let collator3 = network.get_node("collator-3").context("Failed to get collator-3")?;

	log::info!("==================== PJS LINKS ====================");
	for (name, node) in
		[("collator-1", collator1), ("collator-2", collator2), ("collator-3", collator3)]
	{
		log::info!(
			"[{}] PJS:    https://polkadot.js.org/apps/?rpc={}#/explorer",
			name,
			node.ws_uri()
		);
		log::info!("[{}] WS:     {}", name, node.ws_uri());
	}
	log::info!("===================================================");

	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	log::info!("Setting RetentionPeriod to {} blocks", SOAK_RETENTION_PERIOD);
	set_retention_period(&client, SOAK_RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	let bytes_per_op = TEST_DATA_SIZE as u64;
	authorize_account_via_sudo(
		&client,
		&dev::alice().public_key().0,
		SOAK_AUTH_TX_SLOTS,
		bytes_per_op * SOAK_AUTH_TX_SLOTS as u64,
		nonce,
	)
	.await?;
	nonce += 1;

	// Drive the soak. Subscribe to best blocks; each new block, submit one store and (with
	// 50% probability) one renew of a recent item. Every SOAK_VERIFY_INTERVAL_BLOCKS, run a
	// pruning verification on one old item.
	let mut sub = client.blocks().subscribe_best().await?;
	let start_time = std::time::Instant::now();
	let mut stored: Vec<SoakItem> = Vec::new();
	let mut total_stores = 0u32;
	let mut total_renews = 0u32;
	let mut total_pruned_verifications_ok = 0u32;
	let mut total_pruned_verifications_failed = 0u32;
	let mut last_seen_block = 0u64;

	while start_time.elapsed() < std::time::Duration::from_secs(SOAK_DURATION_SECS) {
		let block = sub
			.next()
			.await
			.ok_or_else(|| anyhow::anyhow!("subscribe_best ended unexpectedly"))??;
		let block_n = block.number() as u64;
		if block_n == last_seen_block {
			continue;
		}
		last_seen_block = block_n;

		let elapsed_min = start_time.elapsed().as_secs() / 60;
		if block_n.is_multiple_of(5) {
			log::info!(
				"[soak] block {} | elapsed {} min | tracked={} | stores={} renews={} verified_pruned={} (failed={})",
				block_n,
				elapsed_min,
				stored.len(),
				total_stores,
				total_renews,
				total_pruned_verifications_ok,
				total_pruned_verifications_failed
			);
		}

		// Periodically check chain nonce, but only sync UP. Chain's account_nonce reflects
		// only included extrinsics, so a few of our pool-pending txs always make local lead
		// chain — that's normal. Syncing local to chain in that case wipes the in-flight
		// txs' nonces and causes pool dedup. We only sync forward if chain somehow leads
		// us (e.g., recovery after a previous wrong sync).
		if block_n.is_multiple_of(10) {
			match client.tx().account_nonce(&dev::alice().public_key().to_account_id()).await {
				Ok(chain_nonce) =>
					if chain_nonce > nonce {
						log::info!(
							"[soak] catching up local nonce: local={} chain={}",
							nonce,
							chain_nonce
						);
						nonce = chain_nonce;
					},
				Err(e) => log::warn!("[soak] account_nonce query failed: {}", e),
			}
		}

		// Submit a fresh store — fire-and-forget. We don't wait for InBestBlock; the
		// pruning verification later uses bitswap directly to confirm chain effects.
		let pattern: Vec<u8> = format!("SOAK_{:06}_", total_stores).into_bytes();
		let data = generate_test_data(TEST_DATA_SIZE, &pattern);
		let content_hash = blake2_256(&data);
		let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(&data)]);
		let store_params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
		match client.tx().sign_and_submit(&store_call, &dev::alice(), store_params).await {
			Ok(_) => {
				stored.push(SoakItem { data, content_hash, last_touch_block: block_n });
				total_stores += 1;
				nonce += 1;
			},
			Err(e) => log::warn!("[soak] store at block {} failed: {}", block_n, e),
		}

		// 50% chance to renew a recent item (last touched within RP-1 of current block).
		if pseudo_random(block_n).is_multiple_of(2) {
			let cutoff = block_n.saturating_sub(SOAK_RETENTION_PERIOD as u64 - 1);
			let candidates: Vec<usize> = stored
				.iter()
				.enumerate()
				.filter(|(_, item)| item.last_touch_block >= cutoff)
				.map(|(i, _)| i)
				.collect();
			if !candidates.is_empty() {
				let idx = candidates[(pseudo_random(block_n + 1) as usize) % candidates.len()];
				let hash = stored[idx].content_hash;
				let renew_call = tx(
					"TransactionStorage",
					"renew_content_hash",
					vec![Value::from_bytes(hash.as_slice())],
				);
				let renew_params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
				match client.tx().sign_and_submit(&renew_call, &dev::alice(), renew_params).await {
					Ok(_) => {
						stored[idx].last_touch_block = block_n;
						total_renews += 1;
						nonce += 1;
					},
					Err(e) => log::warn!("[soak] renew at block {} failed: {}", block_n, e),
				}
			}
		}

		// Verify pruning for an old item every SOAK_VERIFY_INTERVAL_BLOCKS blocks. Rotate
		// across collators so we exercise all three.
		if block_n.is_multiple_of(SOAK_VERIFY_INTERVAL_BLOCKS) && block_n > 0 {
			let pruning_age = block_n.saturating_sub(SOAK_PRUNED_AGE_THRESHOLD);
			let target = stored.iter().find(|i| i.last_touch_block < pruning_age).cloned();
			if let Some(item) = target {
				let target_collator = match (block_n / SOAK_VERIFY_INTERVAL_BLOCKS) % 3 {
					0 => ("collator-1", collator1),
					1 => ("collator-2", collator2),
					_ => ("collator-3", collator3),
				};
				let label = format!(
					"{} (item touched at block {}, current {})",
					target_collator.0, item.last_touch_block, block_n
				);
				match expect_bitswap_dont_have(
					target_collator.1,
					&item.data,
					SOAK_BITSWAP_TIMEOUT_SECS,
					&label,
				)
				.await
				{
					Ok(_) => {
						total_pruned_verifications_ok += 1;
						log::info!("[soak] ✓ pruning verified on {}", label);
					},
					Err(e) => {
						total_pruned_verifications_failed += 1;
						log::warn!("[soak] ✗ pruning verification FAILED on {}: {}", label, e);
					},
				}
			}
		}
	}

	log::info!("=== Soak window elapsed; final tallies ===");
	log::info!("Total stores attempted/succeeded: {}", total_stores);
	log::info!("Total renews succeeded: {}", total_renews);
	log::info!(
		"Pruning verifications: {} ok / {} failed",
		total_pruned_verifications_ok,
		total_pruned_verifications_failed
	);
	log::info!("Tracked items: {}", stored.len());

	if total_pruned_verifications_failed > 0 {
		anyhow::bail!(
			"{} pruning verifications failed during the soak — items that should have been \
			 evicted were still served via bitswap",
			total_pruned_verifications_failed
		);
	}

	test_log!(TEST, "=== Soak test PASSED ===");
	network.destroy().await?;
	Ok(())
}

// ===========================================================================================
// Node-level: --blocks-pruning compatibility on restart (manual SIGTERM + re-spawn)
// ===========================================================================================
//
// zombienet-sdk-0.3.13 has no API to restart a node with modified args
// (`NetworkNode::restart()` reuses the original args; args are stored immutably). To exercise
// "restart with new --blocks-pruning on the same data dir", we:
// 1. Spawn the network normally with `initial_pruning`.
// 2. Wait for blocks so the DB has real state.
// 3. Read the collator's args via `NetworkNode::args()`.
// 4. Find the collator process by `--base-path` substring via `ps -ef`, send SIGTERM.
// 5. Manually `polkadot-omni-node` directly with same args except `--blocks-pruning` modified.
// 6. Capture stderr/stdout for ~12s, log relevant lines.
// 7. Network teardown handles the rest.

const PRUNE_RESTART_INITIAL_BLOCKS_TARGET: u64 = 50;

/// Find the value of an argument whose name occurs in the given list, in either
/// `--name=value` or `--name value` form. Returns `None` if not found.
fn extract_arg_value(args: &[String], name: &str) -> Option<String> {
	let prefix_eq = format!("{}=", name);
	for (i, a) in args.iter().enumerate() {
		if a == name {
			return args.get(i + 1).cloned();
		}
		if let Some(rest) = a.strip_prefix(&prefix_eq) {
			return Some(rest.to_string());
		}
	}
	None
}

/// Find the running parachain collator's PID by grepping `ps -ef` for the unique base_path.
fn find_pid_by_base_path(base_path: &str) -> Option<u32> {
	let output = std::process::Command::new("ps").args(["-ef"]).output().ok()?;
	let stdout = String::from_utf8_lossy(&output.stdout);
	for line in stdout.lines() {
		if line.contains("polkadot-omni-node") && line.contains(base_path) {
			let mut fields = line.split_whitespace();
			let _ = fields.next(); // user
			if let Some(pid) = fields.next().and_then(|s| s.parse::<u32>().ok()) {
				return Some(pid);
			}
		}
	}
	None
}

/// Send SIGTERM to the given PID via `kill`, wait briefly for the process to exit.
async fn sigterm_and_wait(pid: u32) -> Result<()> {
	let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
	// Wait up to 5 s for the process to actually exit and release file locks.
	for _ in 0..10 {
		tokio::time::sleep(std::time::Duration::from_millis(500)).await;
		// Check if the process is still alive (kill -0 returns 0 if alive).
		let still_alive = std::process::Command::new("kill")
			.args(["-0", &pid.to_string()])
			.status()
			.map(|s| s.success())
			.unwrap_or(false);
		if !still_alive {
			return Ok(());
		}
	}
	// Hard kill if still running.
	let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
	tokio::time::sleep(std::time::Duration::from_secs(1)).await;
	Ok(())
}

/// Build a fresh argument list for the manual omni-node respawn: take the original args, drop
/// any existing `--blocks-pruning` flag (in `--name=value` or `--name value` form), and inject
/// the new one (or omit it for archive mode). All ports are forced to 0 so the OS picks
/// fresh ones (avoiding any lingering port collision with the killed process).
fn build_respawn_args(orig: &[String], new_pruning: Option<u32>) -> Vec<String> {
	let mut out = Vec::with_capacity(orig.len() + 4);
	let mut i = 0;
	while i < orig.len() {
		let a = &orig[i];
		// Drop existing --blocks-pruning forms.
		if a == "--blocks-pruning" {
			i += 2; // skip name + value
			continue;
		}
		if a.starts_with("--blocks-pruning=") {
			i += 1;
			continue;
		}
		// Force port 0 on RPC/p2p/prometheus.
		if matches!(a.as_str(), "--rpc-port" | "--port" | "--prometheus-port") {
			out.push(a.clone());
			out.push("0".to_string());
			i += 2;
			continue;
		}
		out.push(a.clone());
		i += 1;
	}
	// Inject new --blocks-pruning right before the `--` separator (parachain-side flag).
	if let Some(n) = new_pruning {
		if let Some(idx) = out.iter().position(|a| a == "--") {
			out.insert(idx, format!("--blocks-pruning={}", n));
		} else {
			out.push(format!("--blocks-pruning={}", n));
		}
	}
	out
}

/// Spawn `polkadot-omni-node` with the given args, capture combined stdout+stderr for the
/// duration, kill, return (exit_code, captured_log).
async fn spawn_omni_node_capture(
	binary: &str,
	args: &[String],
	duration: std::time::Duration,
) -> Result<(Option<i32>, String)> {
	use std::process::Stdio;
	use tokio::{io::AsyncReadExt, process::Command};

	let mut cmd = Command::new(binary);
	for a in args {
		cmd.arg(a);
	}
	cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

	let mut child = cmd.spawn().context("spawn omni-node")?;

	let stdout = child.stdout.take().unwrap();
	let stderr = child.stderr.take().unwrap();
	let combined = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
	let combined_o = combined.clone();
	let combined_e = combined.clone();
	let so = tokio::spawn(async move {
		let mut buf = Vec::new();
		let mut r = tokio::io::BufReader::new(stdout);
		let _ = r.read_to_end(&mut buf).await;
		let mut g = combined_o.lock().await;
		g.push_str(&String::from_utf8_lossy(&buf));
	});
	let se = tokio::spawn(async move {
		let mut buf = Vec::new();
		let mut r = tokio::io::BufReader::new(stderr);
		let _ = r.read_to_end(&mut buf).await;
		let mut g = combined_e.lock().await;
		g.push_str(&String::from_utf8_lossy(&buf));
	});

	let exit_code = match tokio::time::timeout(duration, child.wait()).await {
		Ok(Ok(status)) => status.code(),
		Ok(Err(_)) => None,
		Err(_) => {
			let _ = child.kill().await;
			let _ = child.wait().await;
			None
		},
	};

	let _ = so.await;
	let _ = se.await;
	let log = combined.lock().await.clone();
	Ok((exit_code, log))
}

/// Filter captured output to lines that mention pruning, archive, DB, error, panic, warn —
/// the only signals we care about for substrate's DB-config compatibility check.
fn pruning_related_lines(log: &str) -> String {
	log.lines()
		.filter(|line| {
			let lower = line.to_lowercase();
			lower.contains("prun") ||
				lower.contains("archive") ||
				lower.contains("rocksdb") ||
				lower.contains("paritydb") ||
				lower.contains("database") ||
				lower.contains("error") ||
				lower.contains("warn") ||
				lower.contains("panic") ||
				(lower.contains("config") && (lower.contains("db") || lower.contains("client")))
		})
		.collect::<Vec<_>>()
		.join("\n")
}

/// Drives the full restart-with-modified-pruning scenario.
///
/// 1. Spin up 3-relay + 1-collator network with `initial_pruning`.
/// 2. Wait for `PRUNE_RESTART_INITIAL_BLOCKS_TARGET` blocks so the DB has real state.
/// 3. Capture the collator's args.
/// 4. SIGTERM the collator process.
/// 5. Manually re-spawn `polkadot-omni-node` with same args + new `--blocks-pruning`.
/// 6. Capture & log relevant output.
async fn run_pruning_restart_scenario(
	scenario: &str,
	initial_pruning: Option<u32>,
	restart_pruning: Option<u32>,
) -> Result<()> {
	verify_parachain_binaries()?;

	let para_args = match initial_pruning {
		Some(n) => get_para_node_args_with_pruning(n),
		None => get_para_node_args(),
	};
	let config = build_parachain_network_config_single_collator(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("alice not found")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("collator-1 not found")?;
	log::info!(
		"[{}] waiting for parachain block {} so DB has real state...",
		scenario,
		PRUNE_RESTART_INITIAL_BLOCKS_TARGET
	);
	wait_for_block_height(
		collator1,
		PRUNE_RESTART_INITIAL_BLOCKS_TARGET,
		BLOCK_PRODUCTION_TIMEOUT_SECS,
	)
	.await?;

	// Read original args.
	let orig_args: Vec<String> = collator1.args().iter().map(|s| s.to_string()).collect();
	let base_path = extract_arg_value(&orig_args, "--base-path")
		.ok_or_else(|| anyhow::anyhow!("collator-1 args do not contain --base-path"))?;
	log::info!("[{}] collator-1 base_path = {}", scenario, base_path);

	// Find PID and SIGTERM the collator.
	let pid = find_pid_by_base_path(&base_path)
		.ok_or_else(|| anyhow::anyhow!("could not find collator-1 PID via ps"))?;
	log::info!("[{}] sending SIGTERM to collator-1 pid={}", scenario, pid);
	sigterm_and_wait(pid).await?;
	log::info!("[{}] collator-1 process terminated", scenario);

	// Build modified args. The collator's stored args don't include the binary itself.
	let respawn_args = build_respawn_args(&orig_args, restart_pruning);
	log::info!("[{}] re-spawning with --blocks-pruning = {:?}", scenario, restart_pruning);

	let binary = std::env::var("POLKADOT_PARACHAIN_BINARY_PATH")
		.unwrap_or_else(|_| "polkadot-omni-node".to_string());
	let (exit, log) =
		spawn_omni_node_capture(&binary, &respawn_args, std::time::Duration::from_secs(15)).await?;

	let relevant = pruning_related_lines(&log);
	let last_30: Vec<&str> = log.lines().rev().take(30).collect();
	let last_30: Vec<&str> = last_30.into_iter().rev().collect();

	log::info!(
		"[{}] respawn exit={:?}\n--- relevant log lines ---\n{}\n--- last 30 lines of full log ---\n{}\n",
		scenario,
		exit,
		relevant,
		last_30.join("\n")
	);

	network.destroy().await?;
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_restart_archive_to_pruning_test() -> Result<()> {
	const TEST: &str = "restart_archive_to_pruning";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();
	test_log!(TEST, "=== Restart from archive (no --blocks-pruning) → --blocks-pruning=10 ===");
	run_pruning_restart_scenario("archive_to_10", None, Some(10)).await?;
	test_log!(TEST, "=== finished — see log above for substrate's response ===");
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_restart_pruning_increase_test() -> Result<()> {
	const TEST: &str = "restart_pruning_increase";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();
	test_log!(TEST, "=== Restart from --blocks-pruning=10 → --blocks-pruning=20 (increase) ===");
	run_pruning_restart_scenario("10_to_20", Some(10), Some(20)).await?;
	test_log!(TEST, "=== finished — see log above for substrate's response ===");
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_restart_pruning_decrease_test() -> Result<()> {
	const TEST: &str = "restart_pruning_decrease";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();
	test_log!(TEST, "=== Restart from --blocks-pruning=20 → --blocks-pruning=10 (decrease) ===");
	run_pruning_restart_scenario("20_to_10", Some(20), Some(10)).await?;
	test_log!(TEST, "=== finished — see log above for substrate's response ===");
	Ok(())
}

// ===========================================================================================
// Bitswap-confirmed: restart archive → pruning evicts old col11 entries
// ===========================================================================================

/// Like `build_respawn_args` but preserves the original libp2p/RPC/Prometheus ports.
/// Needed for the eviction test so the respawned node keeps the same multiaddr (which lets us
/// reuse `collator1.multiaddr()` for bitswap), and the same relay-side --port (so relay-chain
/// peer discovery via the chain spec's bootnodes still works on a known address).
fn build_respawn_args_keep_ports(orig: &[String], new_pruning: Option<u32>) -> Vec<String> {
	let mut out = Vec::with_capacity(orig.len() + 2);
	let mut i = 0;
	while i < orig.len() {
		let a = &orig[i];
		if a == "--blocks-pruning" {
			i += 2;
			continue;
		}
		if a.starts_with("--blocks-pruning=") {
			i += 1;
			continue;
		}
		out.push(a.clone());
		i += 1;
	}
	if let Some(n) = new_pruning {
		if let Some(idx) = out.iter().position(|a| a == "--") {
			out.insert(idx, format!("--blocks-pruning={}", n));
		} else {
			out.push(format!("--blocks-pruning={}", n));
		}
	}
	out
}

/// Spawn omni-node detached, redirecting stdout+stderr to `log_path` so the caller can
/// inspect what the respawned process actually did. Returns the Child handle.
async fn spawn_omni_node_detached_to_log(
	binary: &str,
	args: &[String],
	log_path: &std::path::Path,
) -> Result<tokio::process::Child> {
	use std::process::Stdio;
	use tokio::process::Command;
	let log_file = std::fs::File::create(log_path).context("create respawn log file")?;
	let log_file_err = log_file.try_clone().context("clone log file handle")?;
	let mut cmd = Command::new(binary);
	for a in args {
		cmd.arg(a);
	}
	cmd.stdout(Stdio::from(log_file)).stderr(Stdio::from(log_file_err));
	let child = cmd.spawn().context("spawn omni-node")?;
	Ok(child)
}

const EVICT_TEST_INITIAL_STORE_TARGET_BLOCK: u64 = 12;
const EVICT_TEST_RESTART_AFTER_BLOCK: u64 = 50;
/// How long to let the respawned node run before verifying that pruning has caught up. With
/// 6-second blocks and ~10-block finality lag on cumulus, after 5 minutes the restarted node
/// should have produced ~50 more blocks and finalized ~40 of them — well past
/// `--blocks-pruning=10`, so the originally-stored items' col11 refs from blocks ~10 are dropped.
const EVICT_TEST_POST_RESTART_WAIT_SECS: u64 = 300;
const EVICT_TEST_RETENTION_PERIOD: u32 = 10;

/// End-to-end: store data while the collator is in archive mode, restart the same data dir
/// with `--blocks-pruning=10`, wait for finalized-block pruning to catch up, and confirm
/// bitswap returns `DONT_HAVE` for the originally-stored items.
///
/// This is the empirical companion to the "is `--blocks-pruning` change really applied?"
/// question: substrate accepts the change silently (no compatibility check on
/// `BlocksPruning`), but we want to see col11 actually evicted.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_restart_archive_to_pruning_evicts_old_blocks_test() -> Result<()> {
	const TEST: &str = "evict_archive_to_pruning";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();
	test_log!(TEST, "=== Restart archive → --blocks-pruning=10, verify col11 evicted ===");

	verify_parachain_binaries()?;

	// 1. Spawn 1 collator in archive mode (no --blocks-pruning).
	let para_args = get_para_node_args();
	let config = build_parachain_network_config_single_collator(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("alice not found")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("collator-1 not found")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	// 2. Shrink RetentionPeriod (so pallet state is also pruned alongside the bitswap eviction).
	let mut nonce = get_alice_nonce(collator1).await?;
	log::info!("[evict] setting RetentionPeriod = {}", EVICT_TEST_RETENTION_PERIOD);
	set_retention_period(&client, EVICT_TEST_RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// 3. Authorize Alice for ~5 stores' worth of capacity.
	let bytes_per_item = TEST_DATA_SIZE as u64;
	authorize_account_via_sudo(&client, &dev::alice().public_key().0, 5, bytes_per_item * 5, nonce)
		.await?;
	nonce += 1;

	// 4. Store three distinct items at the chain's earliest blocks.
	wait_for_block_height(
		collator1,
		EVICT_TEST_INITIAL_STORE_TARGET_BLOCK,
		BLOCK_PRODUCTION_TIMEOUT_SECS,
	)
	.await?;
	let items: Vec<Vec<u8>> = (0..3)
		.map(|i| {
			let mut pattern = b"RESTART_EVICT_".to_vec();
			pattern.extend_from_slice(format!("{:03}_", i).as_bytes());
			generate_test_data(TEST_DATA_SIZE, &pattern)
		})
		.collect();

	let mut store_blocks: Vec<u64> = Vec::new();
	for (i, data) in items.iter().enumerate() {
		let block_n = submit_store_signed(&client, data, nonce).await?;
		store_blocks.push(block_n);
		nonce += 1;
		log::info!("[evict] stored item {} at block {}", i, block_n);
	}

	// 5. Sanity bitswap: data should be retrievable in archive mode.
	for (i, data) in items.iter().enumerate() {
		verify_node_bitswap(
			collator1,
			data,
			BITSWAP_TIMEOUT_SECS,
			&format!("collator-1 / item-{} (pre-restart sanity)", i),
		)
		.await?;
	}
	log::info!("[evict] ✓ pre-restart bitswap fetch succeeded for all 3 items");

	// 6. Let chain run until block 50, then SIGTERM the collator.
	wait_for_block_height(collator1, EVICT_TEST_RESTART_AFTER_BLOCK, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await?;
	log::info!(
		"[evict] reached block {}; preparing to restart with --blocks-pruning=10",
		EVICT_TEST_RESTART_AFTER_BLOCK
	);

	let orig_args: Vec<String> = collator1.args().iter().map(|s| s.to_string()).collect();
	let base_path = extract_arg_value(&orig_args, "--base-path")
		.ok_or_else(|| anyhow::anyhow!("collator-1 missing --base-path"))?;
	let original_multiaddr = collator1.multiaddr().to_string();
	log::info!("[evict] base_path={}", base_path);
	log::info!("[evict] saving multiaddr for post-restart bitswap: {}", original_multiaddr);

	let pid = find_pid_by_base_path(&base_path).ok_or_else(|| anyhow::anyhow!("PID not found"))?;
	log::info!("[evict] SIGTERM pid={}", pid);
	sigterm_and_wait(pid).await?;

	// 7. Re-spawn with --blocks-pruning=10, KEEPING ports so the multiaddr is unchanged and the
	//    relay client reuses the bootnode wiring from the chain spec.
	let respawn_args = build_respawn_args_keep_ports(&orig_args, Some(10));
	let binary = std::env::var("POLKADOT_PARACHAIN_BINARY_PATH")
		.unwrap_or_else(|_| "polkadot-omni-node".to_string());
	log::info!("[evict] re-spawning with --blocks-pruning=10 (keeping all ports)");
	let respawn_log_path = std::env::temp_dir().join("bulletin-evict-respawn.log");
	let _ = std::fs::remove_file(&respawn_log_path);
	let mut child =
		spawn_omni_node_detached_to_log(&binary, &respawn_args, &respawn_log_path).await?;
	log::info!("[evict] respawn log will be at {}", respawn_log_path.display());

	// 8. Let the restarted node run long enough that pruning catches up well past block ~12.
	log::info!(
		"[evict] sleeping {}s for finalization+pruning to evict initial blocks...",
		EVICT_TEST_POST_RESTART_WAIT_SECS
	);
	tokio::time::sleep(std::time::Duration::from_secs(EVICT_TEST_POST_RESTART_WAIT_SECS)).await;

	// Diagnostic: is the respawned process actually alive and did it advance the chain?
	let still_alive = match child.try_wait() {
		Ok(Some(status)) => {
			log::warn!(
				"[evict] respawned process EXITED early with status={:?} — pruning never had a chance",
				status
			);
			false
		},
		Ok(None) => {
			log::info!("[evict] respawned process is still running");
			true
		},
		Err(e) => {
			log::warn!("[evict] try_wait failed: {}", e);
			false
		},
	};
	if let Ok(log_text) = std::fs::read_to_string(&respawn_log_path) {
		let last: Vec<&str> = log_text.lines().rev().take(40).collect();
		let last: Vec<&str> = last.into_iter().rev().collect();
		log::info!(
			"[evict] respawn log size = {} bytes; last 40 lines:\n{}",
			log_text.len(),
			last.join("\n")
		);
		// Look for any pruning-related lines from earlier in the log.
		let pruning_lines: Vec<&str> = log_text
			.lines()
			.filter(|line| {
				let lower = line.to_lowercase();
				lower.contains("prun") || lower.contains("archive") || lower.contains("error")
			})
			.collect();
		if !pruning_lines.is_empty() {
			log::info!(
				"[evict] pruning/archive/error lines from the full log:\n{}",
				pruning_lines.join("\n")
			);
		}
	}
	let _ = still_alive;

	// 9. Bitswap verification: data stored at blocks ~12 should now return DONT_HAVE.
	let mut all_evicted = true;
	for (i, data) in items.iter().enumerate() {
		match expect_bitswap_dont_have(
			collator1,
			data,
			BITSWAP_TIMEOUT_SECS,
			&format!("collator-1 / item-{} (post-pruning eviction)", i),
		)
		.await
		{
			Ok(_) => log::info!("[evict] ✓ item {} evicted from col11 — bitswap DONT_HAVE", i),
			Err(e) => {
				log::warn!("[evict] ✗ item {} still served by bitswap: {}", i, e);
				all_evicted = false;
			},
		}
	}

	// 10. Cleanup: kill manual respawn first, then network.
	let _ = child.kill().await;
	let _ = child.wait().await;
	network.destroy().await?;

	if !all_evicted {
		anyhow::bail!("at least one item was still bitswap-fetchable after restart-with-pruning + {}s wait — col11 wasn't evicted", EVICT_TEST_POST_RESTART_WAIT_SECS);
	}

	test_log!(TEST, "=== Restart archive → pruning EVICTED all old items as expected ===");
	Ok(())
}

/// Drive `do_process_auto_renewals` into the `AutoRenewalFailed` branch by sizing Alice's
/// `bytes_allowance` so the per-account `has_permanent_capacity` cap is hit on cycle 3.
/// `authorize_and_store_data` grants `bytes_allowance = 2 * data.len()` and we intentionally
/// skip the usual `top_up_alice_authorization`, so cycles 1 and 2 fit (the cap is inclusive)
/// and cycle 3 trips `PERMANENT_ALLOWANCE_EXCEEDED`.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_quota_exhaustion_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_quota_exhaustion";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Parachain Auto-Renewal Quota Exhaustion Test ===");

	verify_parachain_binaries()?;

	let config = build_parachain_network_config_single_collator(get_para_node_args())?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	set_retention_period(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	let data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (hash_hex, _) = content_hash_and_cid(&data);
	log::info!("Test data: {} bytes, hash={}", data.len(), hash_hex);

	let (store_block, next_nonce) = authorize_and_store_data(collator1, &data, nonce).await?;
	nonce = next_nonce;
	verify_node_bitswap(collator1, &data, BITSWAP_TIMEOUT_SECS, "post-store").await?;

	// The `local_testnet` genesis preset pre-authorizes Alice with `(100 tx, 10 MiB)`, and
	// `authorize_account` is additive on the unexpired path — so after `authorize_and_store_data`
	// Alice's real `bytes_allowance` is ~10 MiB, not `2 * data.len()`. Overwrite the entry
	// directly so the per-account cap actually trips after a known number of cycles.
	let l = data.len() as u64;
	override_alice_authorization(
		&client,
		AuthorizationOverride {
			transactions: 1,
			transactions_allowance: 10,
			bytes: l,
			bytes_permanent: 0,
			bytes_allowance: 2 * l,
			expiration: u32::MAX,
		},
		nonce,
	)
	.await?;
	nonce += 1;
	log::info!(
		"Data stored at block {}; authorization pinned to bytes_allowance = 2 × {}",
		store_block,
		data.len()
	);

	let content_hash = blake2_256(&data);
	enable_auto_renew(&client, &content_hash, nonce).await?;
	log::info!("Auto-renewal enabled for {}", hash_hex);

	let cadence = RETENTION_PERIOD as u64 + 1;
	let r1 = store_block + cadence;
	let r2 = store_block + 2 * cadence;
	let r3 = store_block + 3 * cadence;

	// Cycles 1 and 2 must succeed.
	for (cycle, renewal_block) in [(1u64, r1), (2, r2)] {
		let wait_until = renewal_block + 1;
		log::info!(
			"[cycle {}] Waiting for block {} (renewal at {})",
			cycle,
			wait_until,
			renewal_block
		);
		wait_for_block_height(collator1, wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

		let renewal_hash = block_hash_at(&client, renewal_block).await?;
		let events = client.blocks().at(renewal_hash).await?.events().await?;
		let renewed = count_event(&events, "DataAutoRenewed");
		let failed = count_event(&events, "AutoRenewalFailed");
		assert_eq!(
			renewed, 1,
			"[cycle {}] expected exactly 1 DataAutoRenewed event at block {}, saw {}",
			cycle, renewal_block, renewed
		);
		assert_eq!(
			failed, 0,
			"[cycle {}] expected 0 AutoRenewalFailed events at block {}, saw {}",
			cycle, renewal_block, failed
		);
		verify_node_bitswap(
			collator1,
			&data,
			BITSWAP_TIMEOUT_SECS,
			&format!("after cycle {}", cycle),
		)
		.await
		.with_context(|| format!("cycle {} did not preserve the data", cycle))?;
		log::info!("[cycle {}] ✓ DataAutoRenewed at block {}", cycle, renewal_block);
	}

	// Cycle 3 must fail: bytes_permanent (= 2L) + L > bytes_allowance (= 2L).
	let wait_until = r3 + 1;
	log::info!("[cycle 3] Waiting for block {} (renewal at {}) — expected to fail", wait_until, r3);
	wait_for_block_height(collator1, wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	let r3_hash = block_hash_at(&client, r3).await?;
	let events = client.blocks().at(r3_hash).await?.events().await?;
	let failed = count_event(&events, "AutoRenewalFailed");
	let renewed = count_event(&events, "DataAutoRenewed");
	log::info!("[cycle 3] block {}: renewed={}, failed={}", r3, renewed, failed);
	assert_eq!(
		failed, 1,
		"[cycle 3] expected exactly 1 AutoRenewalFailed event at block {}, saw {} (renewed={})",
		r3, failed, renewed
	);
	assert_eq!(
		renewed, 0,
		"[cycle 3] expected 0 DataAutoRenewed events at block {}, saw {}",
		r3, renewed
	);
	log::info!("[cycle 3] ✓ AutoRenewalFailed at block {}", r3);

	// AutoRenewals[content_hash] must be removed by the failure branch. Query at the
	// renewal block's hash, not `at_latest`, because `at_latest` reads finalized state and
	// cumulus finality lags ~10s behind best-block production.
	let auto_renewals_addr = subxt::dynamic::storage(
		"TransactionStorage",
		"AutoRenewals",
		vec![Value::from_bytes(content_hash.as_slice())],
	);
	let auto_renewals_after = client.storage().at(r3_hash).fetch(&auto_renewals_addr).await?;
	assert!(
		auto_renewals_after.is_none(),
		"AutoRenewals[{}] should be None at block {} after AutoRenewalFailed",
		hash_hex,
		r3
	);
	log::info!("✓ AutoRenewals[{}] removed at block {}", hash_hex, r3);

	test_log!(TEST, "=== Parachain Auto-Renewal Quota Exhaustion Test PASSED ===");
	network.destroy().await?;
	Ok(())
}

/// Walk best-chain headers backwards to find the block at the given height and return its hash.
async fn block_hash_at(
	client: &OnlineClient<SubstrateConfig>,
	target: u64,
) -> Result<subxt::utils::H256> {
	let mut current = current_best_block(client).await?;
	while (current.number() as u64) > target {
		let parent_hash = current.header().parent_hash;
		current = client.blocks().at(parent_hash).await?;
	}
	if (current.number() as u64) != target {
		anyhow::bail!("could not locate block {} (best chain at {})", target, current.number());
	}
	Ok(current.hash())
}

/// Count `TransactionStorage` events of the given variant in a block's event list.
fn count_event(events: &subxt::events::Events<SubstrateConfig>, variant: &str) -> u32 {
	events
		.iter()
		.filter_map(|e| e.ok())
		.filter(|e| e.pallet_name() == "TransactionStorage" && e.variant_name() == variant)
		.count() as u32
}

/// Authorization expires between auto-renew cycles. The runtime's `AuthorizationPeriod`
/// (`14 * DAYS`) is unreachable on a test timescale, so we directly overwrite Alice's
/// `Authorizations` entry via `sudo(System::set_storage(..))` to install a short expiration.
/// Cycle 1 renews; cycle 2 trips the expired branch in `check_authorization` →
/// `AutoRenewalFailed`. Re-authorizing then exercises the expired-but-present reset path
/// (counters zeroed): we store a fresh item and run one more cycle to prove the reset took.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_authorization_expires_mid_cycle_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_auth_expires_mid_cycle";
	let _ = env_logger::Builder::from_env(Env::default().default_filter_or("info")).try_init();

	test_log!(TEST, "=== Parachain Auto-Renewal Authorization Expires Mid-Cycle Test ===");

	verify_parachain_binaries()?;

	let config = build_parachain_network_config_single_collator(get_para_node_args())?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	set_retention_period(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	let data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (hash_hex, _) = content_hash_and_cid(&data);
	log::info!("Test data: {} bytes, hash={}", data.len(), hash_hex);

	let (store_block, next_nonce) = authorize_and_store_data(collator1, &data, nonce).await?;
	nonce = next_nonce;
	log::info!("Data stored at block {}", store_block);

	let cadence = RETENTION_PERIOD as u64 + 1;
	let r1 = store_block + cadence;
	let r2 = store_block + 2 * cadence;

	// `expired()` is `now >= expiration`, so any value in `(r1, r2]` works; halfway gives
	// slack for off-by-one between block scheduling and apply_block_inherents.
	let override_expiration: u32 = ((r1 + r2) / 2) as u32;
	log::info!(
		"Overriding Alice's authorization expiration: r1={}, r2={}, expiration={}",
		r1,
		r2,
		override_expiration
	);

	// Write fresh counters with generous allowances so the renewal gate only fails on expiry,
	// not on the per-account `has_permanent_capacity` cap. After store, real counters are
	// `transactions=1, bytes=data.len()`; we overwrite to those + plenty of headroom.
	let l = data.len() as u64;
	override_alice_authorization(
		&client,
		AuthorizationOverride {
			transactions: 1,
			transactions_allowance: 100,
			bytes: l,
			bytes_permanent: 0,
			bytes_allowance: 100 * l,
			expiration: override_expiration,
		},
		nonce,
	)
	.await?;
	nonce += 1;

	let content_hash = blake2_256(&data);
	enable_auto_renew(&client, &content_hash, nonce).await?;
	nonce += 1;
	log::info!("Auto-renewal enabled");

	// Cycle 1: success.
	wait_for_block_height(collator1, r1 + 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	let r1_hash = block_hash_at(&client, r1).await?;
	let r1_events = client.blocks().at(r1_hash).await?.events().await?;
	assert_eq!(
		count_event(&r1_events, "DataAutoRenewed"),
		1,
		"[cycle 1] expected 1 DataAutoRenewed at block {}",
		r1
	);
	assert_eq!(
		count_event(&r1_events, "AutoRenewalFailed"),
		0,
		"[cycle 1] expected 0 AutoRenewalFailed at block {}",
		r1
	);
	log::info!("[cycle 1] ✓ DataAutoRenewed at block {}", r1);

	// Cycle 2: AutoRenewalFailed because auth has expired.
	wait_for_block_height(collator1, r2 + 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	let r2_hash = block_hash_at(&client, r2).await?;
	let r2_events = client.blocks().at(r2_hash).await?.events().await?;
	assert_eq!(
		count_event(&r2_events, "AutoRenewalFailed"),
		1,
		"[cycle 2] expected 1 AutoRenewalFailed at block {} (auth expired at {})",
		r2,
		override_expiration
	);
	assert_eq!(
		count_event(&r2_events, "DataAutoRenewed"),
		0,
		"[cycle 2] expected 0 DataAutoRenewed at block {}",
		r2
	);

	let auto_renewals_addr = subxt::dynamic::storage(
		"TransactionStorage",
		"AutoRenewals",
		vec![Value::from_bytes(content_hash.as_slice())],
	);
	let auto_renewals_after = client.storage().at(r2_hash).fetch(&auto_renewals_addr).await?;
	assert!(
		auto_renewals_after.is_none(),
		"AutoRenewals[{}] should be None at block {} after AutoRenewalFailed",
		hash_hex,
		r2
	);
	log::info!("[cycle 2] ✓ AutoRenewalFailed at block {}; AutoRenewals[hash] removed", r2);

	// Re-authorize Alice. Because the entry is expired-but-present, the pallet hits the
	// expired-reset branch and zeroes `bytes`, `bytes_permanent`, and `transactions` while
	// installing a fresh expiration of `now + AuthorizationPeriod`.
	let alice_pk = subxt_signer::sr25519::dev::alice().public_key().0;
	authorize_account_via_sudo(
		&client,
		&alice_pk,
		/* transactions */ 10,
		/* bytes */ 4 * l,
		nonce,
	)
	.await?;
	nonce += 1;
	log::info!("Re-authorized Alice — expects expired-reset branch to zero counters");

	// Store a fresh item and enable auto-renew on it. This is the end-to-end proof that the
	// reset really took effect: if `bytes_permanent` had carried over, the snapshot check in
	// `enable_auto_renew` (`has_permanent_capacity(size)`) would still see room (since we
	// authorized 4L of cap, vs L of carried-over `bytes_permanent`), but the renewal at cycle 1
	// for the new item would fail. We assert the renewal succeeds.
	let data2 = {
		let mut pattern = PARACHAIN_TEST_DATA_PATTERN.to_vec();
		pattern.extend_from_slice(b"_AFTER_REAUTH_");
		generate_test_data(TEST_DATA_SIZE, &pattern)
	};
	let (hash2_hex, _) = content_hash_and_cid(&data2);

	let store2_block = submit_store_signed(&client, &data2, nonce).await?;
	nonce += 1;
	log::info!("Stored second item at block {} (hash={})", store2_block, hash2_hex);

	let content_hash2 = blake2_256(&data2);
	enable_auto_renew(&client, &content_hash2, nonce).await?;
	log::info!("Auto-renewal enabled for second item");

	let r1_after = store2_block + cadence;
	wait_for_block_height(collator1, r1_after + 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	let r1_after_hash = block_hash_at(&client, r1_after).await?;
	let r1_after_events = client.blocks().at(r1_after_hash).await?.events().await?;
	assert_eq!(
		count_event(&r1_after_events, "DataAutoRenewed"),
		1,
		"post-reauth cycle 1: expected 1 DataAutoRenewed at block {} (counters should be reset)",
		r1_after
	);
	assert_eq!(
		count_event(&r1_after_events, "AutoRenewalFailed"),
		0,
		"post-reauth cycle 1: expected 0 AutoRenewalFailed at block {}",
		r1_after
	);
	verify_node_bitswap(collator1, &data2, BITSWAP_TIMEOUT_SECS, "post-reauth cycle 1").await?;
	log::info!("✓ Post-reauth cycle 1 succeeded — counters reset");

	test_log!(TEST, "=== Parachain Auto-Renewal Authorization Expires Mid-Cycle Test PASSED ===");
	network.destroy().await?;
	Ok(())
}
