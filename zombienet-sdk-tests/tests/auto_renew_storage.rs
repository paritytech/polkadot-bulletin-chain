// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests for the auto-renewal lifecycle, on-init cleanup, and `--blocks-pruning`
//! interactions on a single-collator parachain network.

use crate::{
	test_log,
	utils::{
		authorize_account_via_sudo, authorize_account_via_sudo_finalized, authorize_and_store_data,
		blake2_256, build_parachain_network_config_three_relay_validators, content_hash_and_cid,
		count_event, disable_auto_renew, enable_auto_renew, expect_bitswap_dont_have,
		generate_test_data, get_alice_nonce, initialize_network, override_alice_authorization,
		set_retention_period, set_retention_period_finalized, submit_renew_pair,
		submit_store_signed, top_up_alice_authorization, verify_node_bitswap,
		verify_parachain_binaries, wait_for_block_height, wait_for_finalized_height,
		wait_for_finalized_quiescence, wait_for_session_change_on_node, AuthorizationOverride,
		BLOCK_PRODUCTION_TIMEOUT_SECS, NETWORK_READY_TIMEOUT_SECS, NODE_LOG_CONFIG,
		PARACHAIN_TEST_DATA_PATTERN, TEST_DATA_SIZE,
	},
};
use anyhow::{Context, Result};
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

/// Fetch the latest best block. `at_latest()` returns the latest finalized block via
/// chainHead_v2 — on cumulus, finality lags ~10s behind production, so it can be stuck at
/// block 0 well after the chain is producing.
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

/// Fetch the latest finalized block. Use this when event/storage reads must be stable —
/// best-view can briefly follow a non-canonical branch as chainHead_v2 resolves.
async fn current_finalized_block(
	client: &OnlineClient<SubstrateConfig>,
) -> Result<subxt::blocks::Block<SubstrateConfig, OnlineClient<SubstrateConfig>>> {
	Ok(client.blocks().at_latest().await?)
}

const SESSION_CHANGE_TIMEOUT_SECS: u64 = 300;
const RETENTION_PERIOD: u32 = 10;
const BITSWAP_TIMEOUT_SECS: u64 = 30;
/// `--blocks-pruning` deletion + col11 refcount-zero cleanup runs asynchronously after the
/// finalized-height metric crosses; under CI load that lag can be tens of seconds.
const BITSWAP_EVICTION_TIMEOUT_SECS: u64 = 180;

const NUM_RENEWAL_CYCLES: u64 = 2;
const TOPUP_TX_COUNT: u32 = 5;
/// `+1` is safety: without it, a single byte short flips auto-renewal into `AutoRenewalFailed`.
const TOPUP_BYTES_MULTIPLIER: u64 = NUM_RENEWAL_CYCLES + 1;

/// Pruning smaller than retention: the store block ages out of the pruning window before
/// the proof block, the inherent provider can't construct `TransactionStorageProof`, and
/// `on_finalize`'s `assert!(proof_ok)` halts the chain.
const BLOCKS_PRUNING_LESS_THAN_RETENTION: u32 = 5;
/// Pruning larger than retention: the proof block still finds col11 alive, chain progresses.
const BLOCKS_PRUNING_GREATER_THAN_RETENTION: u32 = 15;
const HALT_DETECTION_TIMEOUT_SECS: u64 = 120;
/// With pruning=5 + RP=10, the proof block at `S+10` lands before finality has caught up
/// enough for pruning to actually evict col11. Bumping retention to 20 pushes the proof
/// block out past the (finality + pruning) lag so col11 is reliably empty.
const RETENTION_PERIOD_FOR_PRUNING_HALT: u32 = 20;
const MANY_ITEMS_COUNT: u32 = pallet_bulletin_transaction_storage::DEFAULT_MAX_BLOCK_TRANSACTIONS;
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

// Tests in the same group share one zombienet network via OnceCell. The Network is never
// dropped — its processes stay alive until the test binary exits and the OS reaps them.
// Group invariants (e.g. `RetentionPeriod`) are set once on first spawn; tests must tolerate
// state left behind by previously-run sibling tests (salt data patterns, capture own
// `store_block`, don't assume nonce 0).

/// We don't cache a long-lived `OnlineClient`: its WebSocket drops during inter-test
/// quiescence, surfacing as opaque RPC errors in the next test. Tests fetch a fresh client
/// per run via `collator1.wait_client()`.
struct SharedHarness {
	/// Held to keep spawned processes alive; never dropped.
	_network: zombienet_sdk::Network<zombienet_sdk::LocalFileSystem>,
	collator1: zombienet_sdk::NetworkNode,
}

static ARCHIVE_HARNESS: tokio::sync::OnceCell<std::sync::Arc<SharedHarness>> =
	tokio::sync::OnceCell::const_new();

static PRUNING_HARNESS: tokio::sync::OnceCell<std::sync::Arc<SharedHarness>> =
	tokio::sync::OnceCell::const_new();

async fn archive_harness() -> Result<std::sync::Arc<SharedHarness>> {
	let harness = ARCHIVE_HARNESS
		.get_or_try_init(|| async { spawn_shared_harness("archive", get_para_node_args()).await })
		.await?
		.clone();
	wait_for_finalized_quiescence(&harness.collator1, QUIESCENCE_TIMEOUT_SECS).await?;
	Ok(harness)
}

async fn pruning_harness() -> Result<std::sync::Arc<SharedHarness>> {
	let harness = PRUNING_HARNESS
		.get_or_try_init(|| async {
			spawn_shared_harness(
				"pruning",
				get_para_node_args_with_pruning(BLOCKS_PRUNING_GREATER_THAN_RETENTION),
			)
			.await
		})
		.await?
		.clone();
	wait_for_finalized_quiescence(&harness.collator1, QUIESCENCE_TIMEOUT_SECS).await?;
	Ok(harness)
}

const QUIESCENCE_TIMEOUT_SECS: u64 = 120;

async fn spawn_shared_harness(
	label: &str,
	para_node_args: Vec<String>,
) -> Result<std::sync::Arc<SharedHarness>> {
	tracing::info!("[{}] spawning shared zombienet network", label);
	verify_parachain_binaries()?;
	// 3 relay validators give a fault-tolerant GRANDPA quorum; with only 2, any brief stall
	// halts finality and widens the best-vs-finalized window, which leaks into event-reading
	// race conditions.
	let config = build_parachain_network_config_three_relay_validators(para_node_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;
	let relay_alice = network
		.get_node("alice")
		.with_context(|| format!("[{}] failed to get relay alice node", label))?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS)
		.await
		.with_context(|| format!("[{}] failed to detect first session change", label))?;
	let collator1 = network
		.get_node("collator-1")
		.with_context(|| format!("[{}] failed to get collator-1", label))?
		.clone();
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	// Wait for finalization so tests can rely on `get_alice_nonce` reflecting the bump.
	let nonce = get_alice_nonce(&collator1).await?;
	set_retention_period_finalized(&client, RETENTION_PERIOD, nonce).await?;
	tracing::info!("[{}] harness ready (RetentionPeriod={})", label, RETENTION_PERIOD);

	Ok(std::sync::Arc::new(SharedHarness { _network: network, collator1 }))
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

/// Extract `(ref_time, proof_size)` for the given dispatch class from a dynamic
/// `System::BlockWeight` value. Returns `(0, 0)` if the shape is unexpected.
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

/// Read `System::BlockWeights::max_block` as `(ref_time, proof_size)` — the absolute ceiling
/// that even Mandatory-class extrinsics must fit inside.
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
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== Parachain Auto-Renewal Test (multi-cycle, {} cycles) ===",
		NUM_RENEWAL_CYCLES
	);

	let harness = archive_harness().await?;
	let collator1 = &harness.collator1;
	let client_owned = collator1.wait_client().await?;
	let client = &client_owned;

	// Salt the pattern so we don't collide with sibling tests on the shared chain.
	let mut data_pattern = PARACHAIN_TEST_DATA_PATTERN.to_vec();
	data_pattern.extend_from_slice(b"_basic_");
	let data = generate_test_data(TEST_DATA_SIZE, &data_pattern);
	let (hash_hex, cid) = content_hash_and_cid(&data);
	tracing::info!("Test data: {} bytes, hash={}, CID={}", data.len(), hash_hex, cid);

	let nonce = get_alice_nonce(collator1).await?;
	let (store_block, mut nonce) = authorize_and_store_data(collator1, &data, nonce).await?;
	tracing::info!("Data stored at block {}", store_block);

	verify_node_bitswap(collator1, &data, BITSWAP_TIMEOUT_SECS, "Collator-1 (post-store)").await?;

	top_up_alice_authorization(
		client,
		TOPUP_TX_COUNT,
		data.len() as u64 * TOPUP_BYTES_MULTIPLIER,
		nonce,
	)
	.await?;
	nonce += 1;

	let content_hash = blake2_256(&data);
	enable_auto_renew(client, &content_hash, nonce).await?;
	nonce += 1;
	tracing::info!("Auto-renewal enabled for content_hash {}", hash_hex);

	// Renewal cadence is `RP + 1`; wait one extra block so the inherent is observable.
	let cadence = RETENTION_PERIOD as u64 + 1;
	for cycle in 1..=NUM_RENEWAL_CYCLES {
		let renewal_block = store_block + cycle * cadence;
		let wait_until = renewal_block + 1;
		tracing::info!(
			"[cycle {}/{}] Waiting for block {} (renewal at block {})",
			cycle,
			NUM_RENEWAL_CYCLES,
			wait_until,
			renewal_block
		);
		wait_for_block_height(collator1, wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

		// Proof for cycle k's source tx_info lands at `(store_block + (k-1)*cadence) + RP`,
		// one block before the renewal. Cycle 1's source is the original store; cycle k>1's
		// source is the previous cycle's renewal.
		let proof_block = store_block + (cycle - 1) * cadence + RETENTION_PERIOD as u64;
		assert_proof_checked_at(client, proof_block, &format!("cycle {}", cycle)).await?;

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
		tracing::info!(
			"[cycle {}/{}] ✓ Data still served at block ≥ {}",
			cycle,
			NUM_RENEWAL_CYCLES,
			wait_until
		);
	}

	// Shared-harness cleanup: stop renewing this item so it doesn't keep consuming Alice's
	// authorization for the rest of the harness lifetime.
	disable_auto_renew(client, &content_hash, nonce).await?;
	tracing::info!("✓ Disabled auto-renew for content_hash — chain idle for the next test");

	test_log!(TEST, "=== Parachain Auto-Renewal Test ({} cycles) PASSED ===", NUM_RENEWAL_CYCLES);
	Ok(())
}

/// `--blocks-pruning < RetentionPeriod` evicts the proof target's col11 entry before the
/// proof block; `apply_block_inherents` is not emitted and `on_finalize`'s `assert!(proof_ok)`
/// halts the chain at `S + RetentionPeriod - 1`.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_check_proof_fails_under_pruning_test() -> Result<()> {
	const TEST: &str = "para_check_proof_pruning";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== Parachain check_proof fails under pruning ({} blocks pruning, retention {}) ===",
		BLOCKS_PRUNING_LESS_THAN_RETENTION,
		RETENTION_PERIOD_FOR_PRUNING_HALT
	);

	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(BLOCKS_PRUNING_LESS_THAN_RETENTION);
	let config = build_parachain_network_config_three_relay_validators(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	tracing::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD_FOR_PRUNING_HALT);
	set_retention_period(&client, RETENTION_PERIOD_FOR_PRUNING_HALT, nonce).await?;
	nonce += 1;

	let data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (hash_hex, _) = content_hash_and_cid(&data);
	tracing::info!("Test data: {} bytes, hash={}", data.len(), hash_hex);

	let (store_block, _) = authorize_and_store_data(collator1, &data, nonce).await?;
	tracing::info!("Data stored at block {}", store_block);

	let last_healthy_block = store_block + RETENTION_PERIOD_FOR_PRUNING_HALT as u64 - 1;
	tracing::info!("Confirming chain reaches block {} (last healthy block)", last_healthy_block);
	wait_for_block_height(collator1, last_healthy_block, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await
		.context("Chain failed to reach the last healthy block before the proof block")?;

	let proof_block = store_block + RETENTION_PERIOD_FOR_PRUNING_HALT as u64;
	tracing::info!(
		"Waiting up to {}s for block {} (proof block) — expected to time out (chain halt)",
		HALT_DETECTION_TIMEOUT_SECS,
		proof_block
	);
	match wait_for_block_height(collator1, proof_block, HALT_DETECTION_TIMEOUT_SECS).await {
		Err(_) => {
			tracing::info!(
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

/// Auto-renewal does not rescue the chain from the `check_proof` halt: the proof block
/// (`S + RP`) precedes the renewal block (`S + RP + 1`) by one block, so the chain panics
/// before auto-renewal fires.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_under_pruning_chain_halts_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_pruning_halt";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== Parachain auto-renewal under pruning halts ({} blocks pruning, retention {}) ===",
		BLOCKS_PRUNING_LESS_THAN_RETENTION,
		RETENTION_PERIOD_FOR_PRUNING_HALT
	);

	verify_parachain_binaries()?;

	let para_args = get_para_node_args_with_pruning(BLOCKS_PRUNING_LESS_THAN_RETENTION);
	let config = build_parachain_network_config_three_relay_validators(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	tracing::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD_FOR_PRUNING_HALT);
	set_retention_period(&client, RETENTION_PERIOD_FOR_PRUNING_HALT, nonce).await?;
	nonce += 1;

	let data = generate_test_data(TEST_DATA_SIZE, PARACHAIN_TEST_DATA_PATTERN);
	let (store_block, next_nonce) = authorize_and_store_data(collator1, &data, nonce).await?;
	nonce = next_nonce;
	tracing::info!("Data stored at block {}", store_block);

	let content_hash = blake2_256(&data);
	enable_auto_renew(&client, &content_hash, nonce).await?;
	tracing::info!("Auto-renewal enabled");

	let last_healthy_block = store_block + RETENTION_PERIOD_FOR_PRUNING_HALT as u64 - 1;
	tracing::info!("Confirming chain reaches block {} (last healthy block)", last_healthy_block);
	wait_for_block_height(collator1, last_healthy_block, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	let proof_block = store_block + RETENTION_PERIOD_FOR_PRUNING_HALT as u64;
	tracing::info!(
		"Waiting up to {}s for block {} (proof block) — expected timeout (halt before renewal \
		 block at {})",
		HALT_DETECTION_TIMEOUT_SECS,
		proof_block,
		proof_block + 1
	);
	match wait_for_block_height(collator1, proof_block, HALT_DETECTION_TIMEOUT_SECS).await {
		Err(_) => {
			tracing::info!(
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

/// Two `renew` calls for the same data within one block stack col11 refs; bitswap stays
/// available until all referencing blocks age out of the `--blocks-pruning` window.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_renew_twice_within_block_with_pruning_test() -> Result<()> {
	const TEST: &str = "para_renew_twice_pruning";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== Parachain double-renew under pruning ({} blocks pruning, retention {}) ===",
		BLOCKS_PRUNING_GREATER_THAN_RETENTION,
		RETENTION_PERIOD
	);

	let harness = pruning_harness().await?;
	let collator1 = &harness.collator1;
	let client_owned = collator1.wait_client().await?;
	let client = &client_owned;

	let mut nonce = get_alice_nonce(collator1).await?;

	let data = generate_test_data(TEST_DATA_SIZE, b"DATA_RENEW_TWICE_");
	let (hash_hex, _) = content_hash_and_cid(&data);
	tracing::info!("Test data: {} bytes, hash={}", data.len(), hash_hex);

	let (store_block, next_nonce) = authorize_and_store_data(collator1, &data, nonce).await?;
	nonce = next_nonce;
	tracing::info!("Data stored at block {}", store_block);

	// `validate_signed` tags renewals with `(who, content_hash)`, so two renews from the
	// same signer would conflict in the pool — use Bob as a second signer. Bob has no prior
	// authorization, and the pool reads `Authorizations` from finalized state, so the
	// authorize must be finalized before Bob's renew can validate.
	let bob_pk = subxt_signer::sr25519::dev::bob().public_key().0;
	authorize_account_via_sudo_finalized(client, &bob_pk, 1, data.len() as u64, nonce).await?;
	nonce += 1;
	let bob_nonce = client
		.tx()
		.account_nonce(&subxt_signer::sr25519::dev::bob().public_key().to_account_id())
		.await?;

	let content_hash = blake2_256(&data);
	let (renew_block_a, renew_block_b) =
		submit_renew_pair(client, store_block as u32, 0, &content_hash, nonce, bob_nonce).await?;
	if renew_block_a != renew_block_b {
		tracing::warn!(
			"Renews landed in different blocks ({} and {}) instead of one — test still valid \
			 but uses the later block for pruning math",
			renew_block_a,
			renew_block_b
		);
	} else {
		tracing::info!("Both renews landed in the same block {}", renew_block_a);
	}
	let renew_block = std::cmp::max(renew_block_a, renew_block_b);

	// Proof for the original store lands at `store_block + RP` (one block before pruning
	// could evict). At this point col11 still has the chunks and the proof can be built.
	let proof_block = store_block + RETENTION_PERIOD as u64;
	wait_for_block_height(collator1, proof_block + 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	assert_proof_checked_at(client, proof_block, "post-store").await?;

	verify_node_bitswap(collator1, &data, BITSWAP_TIMEOUT_SECS, "Collator-1 (post-renew)").await?;

	// `--blocks-pruning=N` prunes blocks N behind FINALIZED head, not best head. Waiting
	// on best-block + fudge is flaky under finality lag.
	let after_renew_pruned_finalized =
		renew_block + BLOCKS_PRUNING_GREATER_THAN_RETENTION as u64 + 1;
	tracing::info!(
		"Waiting for FINALIZED block {} so both store and renew blocks are past the pruning boundary",
		after_renew_pruned_finalized
	);
	wait_for_finalized_height(
		collator1,
		after_renew_pruned_finalized,
		BLOCK_PRODUCTION_TIMEOUT_SECS,
	)
	.await?;

	expect_bitswap_dont_have(
		collator1,
		&data,
		BITSWAP_EVICTION_TIMEOUT_SECS,
		"Collator-1 (post-pruning)",
	)
	.await
	.context(
		"Bitswap still serves data after both store and renew blocks were pruned — col11 \
			 should be empty",
	)?;
	tracing::info!(
		"✓ Bitswap returns DONT_HAVE after both store and renew blocks were pruned (col11 \
		 refcount reached zero)"
	);

	test_log!(TEST, "=== Parachain double-renew under pruning PASSED ===");
	Ok(())
}

/// A fresh `store` and an auto-renewal inherent can land in the same block; both items are
/// fetchable, and later pruning evicts only the item without auto-renewal.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_with_concurrent_store_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_concurrent_store";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== Parachain auto-renewal + same-block store ({} blocks pruning, retention {}) ===",
		BLOCKS_PRUNING_GREATER_THAN_RETENTION,
		RETENTION_PERIOD
	);

	let harness = pruning_harness().await?;
	let collator1 = &harness.collator1;
	let client_owned = collator1.wait_client().await?;
	let client = &client_owned;

	let mut nonce = get_alice_nonce(collator1).await?;

	let data1 = generate_test_data(TEST_DATA_SIZE, b"DATA1_CONCURRENT_STORE_");
	let data2 = generate_test_data(TEST_DATA_SIZE, b"DATA2_CONCURRENT_STORE_");
	tracing::info!("data1 hash={}", content_hash_and_cid(&data1).0);
	tracing::info!("data2 hash={}", content_hash_and_cid(&data2).0);

	let (store_block, next_nonce) = authorize_and_store_data(collator1, &data1, nonce).await?;
	nonce = next_nonce;
	tracing::info!("data1 stored at block {}", store_block);

	top_up_alice_authorization(client, 5, 4 * data1.len() as u64, nonce).await?;
	nonce += 1;

	let content_hash_data1 = blake2_256(&data1);
	enable_auto_renew(client, &content_hash_data1, nonce).await?;
	nonce += 1;
	tracing::info!("Auto-renewal enabled for data1");

	// Submit data2 at R-1 so it lands in the renewal block R = S + RetentionPeriod + 1.
	let renewal_block = store_block + RETENTION_PERIOD as u64 + 1;
	let wait_until_pre_renewal = renewal_block - 1;
	tracing::info!(
		"Waiting until block {} (one before renewal block {}) before submitting data2",
		wait_until_pre_renewal,
		renewal_block
	);
	wait_for_block_height(collator1, wait_until_pre_renewal, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	let data2_block = submit_store_signed(client, &data2, nonce).await?;
	tracing::info!("data2 store landed at block {}", data2_block);
	if data2_block != renewal_block {
		anyhow::bail!(
			"Timing missed: expected data2 to land in renewal block {}, but it landed at \
			 block {}. Re-run the test, or adjust the wait_until math if this is consistent.",
			renewal_block,
			data2_block
		);
	}
	tracing::info!("✓ data2 store and auto-renewal inherent coexist in block {}", renewal_block);

	// Proof for data1's store-block lands at `store_block + RP` = `renewal_block - 1`. data2
	// landed in the renewal block itself, so its proof block is `renewal_block + RP` (asserted
	// after the wait_for_finalized_height below).
	assert_proof_checked_at(client, store_block + RETENTION_PERIOD as u64, "post-store data1")
		.await?;

	verify_node_bitswap(collator1, &data1, BITSWAP_TIMEOUT_SECS, "Collator-1 / data1").await?;
	verify_node_bitswap(collator1, &data2, BITSWAP_TIMEOUT_SECS, "Collator-1 / data2").await?;

	// Pruning fires off FINALIZED head — wait on finalized to cross the boundary directly.
	let after_renewal_pruned_finalized =
		renewal_block + BLOCKS_PRUNING_GREATER_THAN_RETENTION as u64 + 1;
	tracing::info!(
		"Waiting for FINALIZED block {} so the renewal block is past the pruning boundary",
		after_renewal_pruned_finalized
	);
	wait_for_finalized_height(
		collator1,
		after_renewal_pruned_finalized,
		BLOCK_PRODUCTION_TIMEOUT_SECS,
	)
	.await?;

	// Proof for the renewal-block's contents (data1's renewal + data2's store) lands at
	// `renewal_block + RP`. Both items were indexed in Transactions[renewal_block].
	assert_proof_checked_at(client, renewal_block + RETENTION_PERIOD as u64, "post-renewal-block")
		.await?;

	expect_bitswap_dont_have(
		collator1,
		&data2,
		BITSWAP_EVICTION_TIMEOUT_SECS,
		"Collator-1 / data2 (post-pruning)",
	)
	.await
	.context(
		"data2 (no auto-renewal) should be evicted once its only ref-block is pruned, but \
		 bitswap still serves it",
	)?;
	tracing::info!("✓ data2 evicted by pruning (no auto-renewal kept it alive)");

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
	tracing::info!(
		"✓ data1 still alive — auto-renewal at R2 added a fresh ref before R was pruned"
	);

	test_log!(TEST, "=== Parachain auto-renewal + same-block store PASSED ===");
	Ok(())
}

/// Auto-renew preserves an item across pruning of its original store block; an
/// otherwise-identical item without auto-renew is evicted.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_vs_no_renew_eviction_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_vs_no_renew";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== Auto-renew vs no-renew eviction (blocks-pruning={}, retention={}) ===",
		BLOCKS_PRUNING_GREATER_THAN_RETENTION,
		RETENTION_PERIOD,
	);

	let harness = pruning_harness().await?;
	let collator1 = &harness.collator1;
	let client_owned = collator1.wait_client().await?;
	let client = &client_owned;

	let mut nonce = get_alice_nonce(collator1).await?;

	let data_renewed = generate_test_data(TEST_DATA_SIZE, b"DATA_VS_NO_RENEW_RENEWED_");
	let data_not_renewed = generate_test_data(TEST_DATA_SIZE, b"DATA_VS_NO_RENEW_NOT_RENEWED_");

	let (store_block, next_nonce) =
		authorize_and_store_data(collator1, &data_renewed, nonce).await?;
	nonce = next_nonce;
	tracing::info!("data_renewed stored at block {}", store_block);

	top_up_alice_authorization(client, 5, 4 * data_renewed.len() as u64, nonce).await?;
	nonce += 1;

	let data_not_renewed_block = submit_store_signed(client, &data_not_renewed, nonce).await?;
	nonce += 1;
	tracing::info!("data_not_renewed stored at block {}", data_not_renewed_block);

	let content_hash_renewed = blake2_256(&data_renewed);
	enable_auto_renew(client, &content_hash_renewed, nonce).await?;
	tracing::info!("Auto-renewal enabled for data_renewed");

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
	tracing::info!("✓ Both items fetchable shortly after upload");

	// Pruning fires off FINALIZED head; with ~3-5 block finality lag, S + RP + 15 is past it.
	let wait_until = store_block + RETENTION_PERIOD as u64 + 15;
	tracing::info!(
		"Waiting for block {} (store + RP + 15) so block {} is pruned",
		wait_until,
		store_block
	);
	wait_for_block_height(collator1, wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// Proof for each store-block (one per store, since they landed in different blocks)
	// fires at `block + RP`. Single ProofChecked event per source-block.
	assert_proof_checked_at(
		client,
		store_block + RETENTION_PERIOD as u64,
		"post-store data_renewed",
	)
	.await?;
	assert_proof_checked_at(
		client,
		data_not_renewed_block + RETENTION_PERIOD as u64,
		"post-store data_not_renewed",
	)
	.await?;

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
	tracing::info!("✓ data_renewed still served via bitswap");

	expect_bitswap_dont_have(
		collator1,
		&data_not_renewed,
		BITSWAP_EVICTION_TIMEOUT_SECS,
		"Collator-1 / data_not_renewed (post-retention)",
	)
	.await
	.context(
		"data_not_renewed should be evicted — its only ref was at the now-pruned store block",
	)?;
	tracing::info!("✓ data_not_renewed evicted (no auto-renewal kept it alive)");

	test_log!(TEST, "=== Auto-renew vs no-renew eviction PASSED ===");
	Ok(())
}

/// Bulk auto-renewal: enable auto-renew on `MANY_ITEMS_COUNT` items signed by a single account
/// (Alice), then log per-block weights / event counts across the renewal window. Hard
/// assertion: every item is renewed at least once.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_many_items_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_many";
	crate::utils::init_logging();

	test_log!(TEST, "=== Auto-renew {} items, measure block weight ===", MANY_ITEMS_COUNT);

	let harness = archive_harness().await?;
	let collator1 = &harness.collator1;
	let client_owned = collator1.wait_client().await?;
	let client = &client_owned;

	let mut nonce = get_alice_nonce(collator1).await?;

	// Overwrite the Authorizations entry so cycle N+1 reliably trips
	// `PERMANENT_ALLOWANCE_EXCEEDED` (drains `AutoRenewals`, leaving the shared harness idle).
	// `authorize_account` is additive on the unexpired path, so it can't shrink the existing
	// genesis entry.
	let bytes_per_item = TEST_DATA_SIZE as u64;
	let bytes_allowance =
		bytes_per_item * MANY_ITEMS_COUNT as u64 * RENEWAL_CYCLES_TO_OBSERVE as u64;
	let transactions_allowance = MANY_ITEMS_COUNT * (RENEWAL_CYCLES_TO_OBSERVE + 1);
	override_alice_authorization(
		client,
		AuthorizationOverride {
			transactions: 0,
			transactions_allowance,
			bytes: 0,
			bytes_permanent: 0,
			bytes_allowance,
			expiration: u32::MAX,
		},
		nonce,
	)
	.await?;
	nonce += 1;

	let items: Vec<Vec<u8>> = (0..MANY_ITEMS_COUNT)
		.map(|i| {
			let mut pattern = b"AUTO_RENEW_MANY_ITEMS_".to_vec();
			pattern.extend_from_slice(format!("{:04}_", i).as_bytes());
			generate_test_data(TEST_DATA_SIZE, &pattern)
		})
		.collect();
	let content_hashes: Vec<[u8; 32]> = items.iter().map(|d| blake2_256(d)).collect();
	tracing::info!(
		"Generated {} items, first hash={}",
		items.len(),
		hex::encode(content_hashes[0])
	);

	// Plain `sign_and_submit` — watcher subscriptions for hundreds of submissions saturate
	// subxt's chainHead_v2 pinning; pure pool submission lets everything land in 1-2 blocks.
	//
	// `PROOF_DECOY=1` adds one extra store after the bulk so the proof block exercises both
	// branches of `apply_block_inherents` (drain renewals AND verify a proof) in one block.
	let proof_decoy: bool = std::env::var("PROOF_DECOY")
		.ok()
		.and_then(|v| v.parse::<u32>().ok())
		.map(|n| n != 0)
		.unwrap_or(false);
	let alice = dev::alice();
	let pre_store_block = current_best_block(client).await?.number() as u64;
	tracing::info!(
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
	tracing::info!("All {} stores accepted into pool", MANY_ITEMS_COUNT);

	if proof_decoy {
		// Submit the decoy right after the bulk block lands so it goes into the next block;
		// any longer wait and the next proposer is already authoring, pushing the decoy out.
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
		tracing::info!("Submitted 1 proof-decoy store (no auto-renew enabled)");
	}

	let store_inclusion_target = pre_store_block + 5;
	wait_for_block_height(collator1, store_inclusion_target, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// Walk chain backward counting `Stored` events — avoids per-submission block lookups
	// which race against chainHead pinning when done concurrently.
	let post_store_head_n = {
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(client).await?;
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
		let mut current = current_best_block(client).await?;
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
	tracing::info!(
		"Stored {} items across blocks {}..={} ({} distinct blocks)",
		MANY_ITEMS_COUNT,
		earliest_store,
		latest_store,
		store_block_histogram.len()
	);
	let mut hist_entries: Vec<_> = store_block_histogram.iter().collect();
	hist_entries.sort_by_key(|(b, _)| **b);
	for (b, n) in hist_entries {
		tracing::info!("  block {}: {} stores", b, n);
	}

	let pre_enable_block = current_best_block(client).await?.number() as u64;
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
	tracing::info!("All {} enable_auto_renew calls accepted into pool", MANY_ITEMS_COUNT);

	let enable_inclusion_target = pre_enable_block + 5;
	wait_for_block_height(collator1, enable_inclusion_target, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await?;
	{
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(client).await?;
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
		let mut current = current_best_block(client).await?;
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
	tracing::info!("Auto-renewal enabled for all {} items", MANY_ITEMS_COUNT);
	let _ = nonce; // last use

	// Pairwise diffs of `substrate_proposer_block_constructed_{sum,count}` give per-block
	// wall-clock construction time, independent of the runtime's declared weight.
	let renewal_cadence = RETENTION_PERIOD as u64 + 1;
	let first_renewal_block = earliest_store + renewal_cadence;
	let last_renewal_block = latest_store + renewal_cadence * RENEWAL_CYCLES_TO_OBSERVE as u64;
	let wait_until = last_renewal_block + 1;
	tracing::info!(
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

	tracing::info!("--- Per-block proposer block_constructed (wall-clock construction time) ---");
	tracing::info!("Format: blocks (a..=b]: +N blocks, +T s sum, ~ms/block");
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
		tracing::info!(
			"blocks ({}..={}]: +{} blocks, +{:.4} s sum, ~{:.1} ms/block{}",
			n0,
			n1,
			delta_count as u64,
			delta_sum,
			ms_per_block,
			marker
		);
	}

	// `wait_for_block_height` reads the Prometheus best-block metric, which leads subxt's
	// chainHead_v2 subscription. Poll subxt until it catches up so the backward walk covers
	// the renewal window.
	let head = {
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(client).await?;
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
	// Stats window covers baseline (no inherent), store, proof-only, and renewal blocks so
	// subtracting baseline from proof-only / renewal-only isolates per-component cost.
	let stats_range_start = first_renewal_block.saturating_sub(15).max(1);
	let stats_range_end = last_renewal_block + 2;
	tracing::info!(
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

	let (max_block_ref, max_block_pov) = fetch_max_block_weight(client).await?;
	tracing::info!(
		"BlockWeights::max_block = (ref_time={}, proof_size={})",
		max_block_ref,
		max_block_pov
	);

	tracing::info!("--- Block weight stats ---");
	tracing::info!("Format: block N | extrinsics={{n}} DataAutoRenewed={{n}} AutoRenewalFailed={{n}} | normal=(ref_time,proof_size) op=(...) mand=(...)");

	let mut total_renewed: u32 = 0;
	let mut weight_violations: Vec<String> = Vec::new();
	for block_n in stats_range_start..=stats_range_end {
		let Some(&block_hash) = block_hashes_by_number.get(&block_n) else {
			tracing::warn!("No hash recorded for block {}; skipping", block_n);
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

		let marker = if block_hashes_by_number.contains_key(&block_n) &&
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

		tracing::info!(
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

		// Mandatory inherent must fit within max_block in both dimensions.
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
	tracing::info!("Total renewals across window: {} / {}", total_renewed, expected_total);
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

	// Proof fires at each renewal-source block + RP. Cycle k's source is the previous cycle's
	// renewal at `first_renewal_block + (k-1)*cadence`; cycle 1's source is the original store
	// at `first_renewal_block - cadence`. All map to proof blocks one before each renewal.
	for cycle in 1..=RENEWAL_CYCLES_TO_OBSERVE as u64 {
		let proof_block = first_renewal_block + (cycle - 1) * renewal_cadence - 1;
		assert_proof_checked_at(client, proof_block, &format!("many_items cycle {}", cycle))
			.await?;
	}

	// After cycle 1, all items get re-keyed to the cycle-1 renewal block, so subsequent
	// cycles fire at `first_renewal_block + k*cadence`. Anchor on `first_renewal_block` to
	// find cycle N+1 reliably (across two blocks: the inherent splits when weight-bound).
	let exhaustion_block = first_renewal_block + renewal_cadence * RENEWAL_CYCLES_TO_OBSERVE as u64;
	let exhaustion_wait_until = exhaustion_block + 1;
	tracing::info!(
		"Waiting for cycle-N+1 exhaustion at block {} (last observed renewal at {})",
		exhaustion_block,
		last_renewal_block
	);
	wait_for_finalized_height(collator1, exhaustion_wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await?;
	let mut total_failed: u32 = 0;
	let mut total_renewed_post_window: u32 = 0;
	for n in exhaustion_block..=exhaustion_block + 1 {
		let hash = finalized_block_hash_at(client, n).await?;
		let events = client.blocks().at(hash).await?.events().await?;
		total_failed += count_event(&events, "AutoRenewalFailed");
		total_renewed_post_window += count_event(&events, "DataAutoRenewed");
	}
	assert_eq!(
		total_failed, MANY_ITEMS_COUNT,
		"expected exactly {} AutoRenewalFailed events at blocks {}..={} (cycle N+1 exhaustion); saw {}",
		MANY_ITEMS_COUNT, exhaustion_block, exhaustion_block + 1, total_failed
	);
	assert_eq!(
		total_renewed_post_window,
		0,
		"expected 0 DataAutoRenewed events post-exhaustion at blocks {}..={}; saw {} \
		 (Alice's authorization should have been fully consumed by the observation window)",
		exhaustion_block,
		exhaustion_block + 1,
		total_renewed_post_window
	);
	tracing::info!(
		"✓ All {} items hit AutoRenewalFailed at blocks {}..={}; AutoRenewals storage drained",
		MANY_ITEMS_COUNT,
		exhaustion_block,
		exhaustion_block + 1,
	);

	test_log!(TEST, "=== Auto-renew {} items PASSED ===", MANY_ITEMS_COUNT);
	Ok(())
}

const WORST_CASE_WORKERS: u32 = MANY_ITEMS_COUNT;

/// Worst-case PoV: each renewal touches a distinct `Authorizations[Account(worker_i)]` so
/// iterations don't collapse into cache hits — matches the bench's worst-case model. The
/// single-account variant ([`parachain_auto_renew_many_items_test`]) collapses into one
/// `Authorizations` key and exercises a cheaper-than-declared real cost.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_many_items_worst_case_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_many_worst_case";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== Auto-renew {} items via {} distinct workers, measure block weight + clock ===",
		WORST_CASE_WORKERS,
		WORST_CASE_WORKERS,
	);

	verify_parachain_binaries()?;

	let config = build_parachain_network_config_three_relay_validators(get_para_node_args())?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("Failed to get relay alice node")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("Failed to get collator-1 node")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut alice_nonce = get_alice_nonce(collator1).await?;
	tracing::info!("Setting RetentionPeriod to {} blocks", RETENTION_PERIOD);
	set_retention_period(&client, RETENTION_PERIOD, alice_nonce).await?;
	alice_nonce += 1;

	let workers: Vec<Keypair> = (0..WORST_CASE_WORKERS)
		.map(|i| {
			let uri = SecretUri::from_str(&format!("//worker_{}", i)).expect("worker URI parses");
			Keypair::from_uri(&uri).expect("worker keypair derives")
		})
		.collect();

	// Workers have no genesis authorization: `authorize_account` creates a fresh entry with
	// `bytes_allowance = N · data.len()` so cycle N+1's `check_authorization` fails for every
	// worker, draining `AutoRenewals` and leaving the chain idle.
	let alice = dev::alice();
	let cycles_to_authorize = RENEWAL_CYCLES_TO_OBSERVE;
	let pre_authz_block = current_best_block(&client).await?.number() as u64;
	tracing::info!(
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
		// Immortal: subxt 0.44 defaults to mortal-for-32-blocks; signing many txs at the
		// same head and validating them post-fork at the mortality height triggers
		// `InvalidTransaction::BadProof` because `additional_signed` no longer matches.
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
	tracing::info!("All {} sudo authorize_account calls accepted into pool", WORST_CASE_WORKERS);

	// Pool's `validate_signed` reads finalized state — batch-wait once for finality before
	// any worker submits its first signed tx.
	{
		let post_authz_best = current_best_block(&client).await?.number() as u64;
		wait_for_finalized_height(collator1, post_authz_best + 2, BLOCK_PRODUCTION_TIMEOUT_SECS)
			.await?;
		tracing::info!(
			"Worker authorizations finalized (waited finalized >= {})",
			post_authz_best + 2
		);
	}

	// `enable_auto_renew` is fee-paying (unlike `store`, which is treated as feeless by the
	// bulletin authorization extension); workers without balance fail at validate-time.
	const WORKER_FUND: u128 = 1_000_000_000_000; // 1 WND
	tracing::info!(
		"Submitting {} Balances::transfer_keep_alive calls (Alice → workers) in parallel",
		WORST_CASE_WORKERS
	);
	let mut fund_futs = Vec::with_capacity(workers.len());
	for (i, worker) in workers.iter().enumerate() {
		let pubkey = worker.public_key().0;
		// MultiAddress::Id(account) — AccountIdLookupOf<Runtime> shape.
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
	tracing::info!("All {} transfer_keep_alive calls accepted into pool", WORST_CASE_WORKERS);

	// Alice's txs are nonce-ordered, so once the last worker's account exists, every prior
	// authorize+transfer must have settled.
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
		tracing::info!(
			"Last worker funded (System.Account exists) — all Alice batched txs settled"
		);
	}

	let items: Vec<Vec<u8>> = (0..WORST_CASE_WORKERS)
		.map(|i| {
			let mut pattern = b"WORST_CASE_WORKER_".to_vec();
			pattern.extend_from_slice(format!("{:04}_", i).as_bytes());
			generate_test_data(TEST_DATA_SIZE, &pattern)
		})
		.collect();
	let content_hashes: Vec<[u8; 32]> = items.iter().map(|d| blake2_256(d)).collect();

	let pre_store_block = current_best_block(&client).await?.number() as u64;
	tracing::info!(
		"Submitting {} signed stores in parallel (pre-store block={})",
		WORST_CASE_WORKERS,
		pre_store_block
	);
	let mut store_futs = Vec::with_capacity(workers.len());
	for (worker, data) in workers.iter().zip(items.iter()) {
		let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
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
	tracing::info!("All {} stores accepted into pool", WORST_CASE_WORKERS);

	let store_inclusion_target = pre_store_block + 5;
	wait_for_block_height(collator1, store_inclusion_target, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

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
	tracing::info!(
		"Stored {} items across blocks {}..={} ({} distinct blocks)",
		WORST_CASE_WORKERS,
		earliest_store,
		latest_store,
		store_block_histogram.len()
	);

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
	tracing::info!("All {} enable_auto_renew calls accepted into pool", WORST_CASE_WORKERS);

	let enable_inclusion_target = pre_enable_block + 5;
	wait_for_block_height(collator1, enable_inclusion_target, BLOCK_PRODUCTION_TIMEOUT_SECS)
		.await?;

	let renewal_cadence = RETENTION_PERIOD as u64 + 1;
	let first_renewal_block = earliest_store + renewal_cadence;
	let last_renewal_block = latest_store + renewal_cadence * RENEWAL_CYCLES_TO_OBSERVE as u64;
	let wait_until = last_renewal_block + 1;
	tracing::info!(
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

	tracing::info!("--- Per-block proposer block_constructed (wall-clock construction time) ---");
	tracing::info!("Format: blocks (a..=b]: +N blocks, +T s sum, ~ms/block");
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
		tracing::info!(
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
	tracing::info!(
		"BlockWeights::max_block = (ref_time={}, proof_size={})",
		max_block_ref,
		max_block_pov
	);

	tracing::info!("--- Block weight stats ---");
	tracing::info!("Format: block N | extrinsics={{n}} DataAutoRenewed={{n}} AutoRenewalFailed={{n}} | normal=(ref_time,proof_size) op=(...) mand=(...)");

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
		tracing::info!(
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
	tracing::info!("Total renewals across window: {} / {}", total_renewed, expected_total);
	if total_renewed < expected_total {
		anyhow::bail!(
			"Expected at least {} DataAutoRenewed events, saw {}",
			expected_total,
			total_renewed
		);
	}

	// Proof fires at each cycle's source-block + RP; cycle 1's source is the original stores.
	for cycle in 1..=RENEWAL_CYCLES_TO_OBSERVE as u64 {
		let proof_block = first_renewal_block + (cycle - 1) * renewal_cadence - 1;
		assert_proof_checked_at(&client, proof_block, &format!("worst_case cycle {}", cycle))
			.await?;
	}

	// Anchor on `first_renewal_block` (post-cycle-1 consolidation point); see equivalent
	// block in `parachain_auto_renew_many_items_test`.
	let exhaustion_block = first_renewal_block + renewal_cadence * RENEWAL_CYCLES_TO_OBSERVE as u64;
	let exhaustion_wait_until = exhaustion_block + 1;
	tracing::info!(
		"Waiting for cycle-N+1 exhaustion at block {} (last observed renewal at {})",
		exhaustion_block,
		last_renewal_block
	);
	wait_for_block_height(collator1, exhaustion_wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	let mut total_failed: u32 = 0;
	let mut total_renewed_post_window: u32 = 0;
	for n in exhaustion_block..=exhaustion_block + 1 {
		let hash = block_hash_at(&client, n).await?;
		let events = client.blocks().at(hash).await?.events().await?;
		total_failed += count_event(&events, "AutoRenewalFailed");
		total_renewed_post_window += count_event(&events, "DataAutoRenewed");
	}
	assert_eq!(
		total_failed, WORST_CASE_WORKERS,
		"expected exactly {} AutoRenewalFailed events at blocks {}..={} (cycle N+1 exhaustion); saw {}",
		WORST_CASE_WORKERS, exhaustion_block, exhaustion_block + 1, total_failed
	);
	assert_eq!(
		total_renewed_post_window,
		0,
		"expected 0 DataAutoRenewed post-exhaustion at blocks {}..={}; saw {} (every worker's \
		 authorization should have been fully consumed by the observation window)",
		exhaustion_block,
		exhaustion_block + 1,
		total_renewed_post_window
	);
	tracing::info!(
		"✓ All {} workers hit AutoRenewalFailed at blocks {}..={}; AutoRenewals storage drained",
		WORST_CASE_WORKERS,
		exhaustion_block,
		exhaustion_block + 1,
	);

	test_log!(TEST, "=== Worst-case auto-renew {} items PASSED ===", WORST_CASE_WORKERS);

	// Optional post-PASS hold for manual PJS inspection. Off by default.
	let inspect_hold_secs: u64 = std::env::var("INSPECT_HOLD_SECS")
		.ok()
		.and_then(|s| s.parse().ok())
		.unwrap_or(0);
	if inspect_hold_secs > 0 {
		tracing::info!(
			"[para_auto_renew_many_worst_case] Holding network up for {} seconds — open the PJS link printed by collator-1 above to inspect block weights. Ctrl-C the test process to exit early.",
			inspect_hold_secs,
		);
		tokio::time::sleep(std::time::Duration::from_secs(inspect_hold_secs)).await;
	}

	network.destroy().await?;
	Ok(())
}

const ON_INIT_CLEANUP_ITEMS_PER_SET: u32 = 50;

/// `Hooks::on_initialize` cleans up `TransactionByContentHash` for non-auto-renewed items
/// and lets auto-renewed items survive via the `apply_block_inherents` drain, both at the
/// same retention boundary.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_on_initialize_cleanup_test() -> Result<()> {
	const TEST: &str = "para_on_init_cleanup";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== on_initialize cleanup ({} auto-renew + {} non-auto-renew items) ===",
		ON_INIT_CLEANUP_ITEMS_PER_SET,
		ON_INIT_CLEANUP_ITEMS_PER_SET,
	);

	verify_parachain_binaries()?;
	let config = build_parachain_network_config_three_relay_validators(get_para_node_args())?;
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

	let alice = dev::alice();
	let pre_store_block = current_best_block(&client).await?.number() as u64;
	tracing::info!(
		"Submitting {} stores ({} for renewal + {} for cleanup); pre-store block={}",
		total_items,
		ON_INIT_CLEANUP_ITEMS_PER_SET,
		ON_INIT_CLEANUP_ITEMS_PER_SET,
		pre_store_block
	);
	let mut futs = Vec::with_capacity(total_items as usize);
	for (idx, data) in set1.iter().chain(set2.iter()).enumerate() {
		let call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce + idx as u64).build();
		let signer = alice.clone();
		let cli = client.clone();
		futs.push(async move {
			cli.tx()
				.sign_and_submit(&call, &signer, params)
				.await
				.map_err(anyhow::Error::from)
		});
	}
	nonce += total_items as u64;
	let _: Vec<subxt::utils::H256> = futures::future::try_join_all(futs).await?;
	tracing::info!("All {} stores accepted into pool", total_items);

	wait_for_block_height(collator1, pre_store_block + 5, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

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
	tracing::info!("Stores landed at (or before) block {}", store_block);

	tracing::info!("Enabling auto-renew for {} items (set 1)", ON_INIT_CLEANUP_ITEMS_PER_SET);
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

	let expiry_block = store_block + RETENTION_PERIOD as u64 + 1;
	tracing::info!(
		"Waiting for expiry block {} (= store_block {} + RP {} + 1)",
		expiry_block,
		store_block,
		RETENTION_PERIOD
	);
	wait_for_block_height(collator1, expiry_block + 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// Proof for the store-block fires at `store_block + RP = expiry_block - 1`, one block
	// before `on_initialize` takes `Transactions[store_block]`.
	assert_proof_checked_at(&client, expiry_block - 1, "on_init_cleanup post-store").await?;
	{
		let poll_timeout = std::time::Duration::from_secs(60);
		let start = std::time::Instant::now();
		loop {
			let head = current_best_block(&client).await?;
			if head.number() as u64 > expiry_block {
				break;
			}
			if start.elapsed() > poll_timeout {
				anyhow::bail!("Timed out waiting for at_latest >= {}", expiry_block + 1);
			}
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
		}
	}

	let expiry_hash = {
		let mut current = current_best_block(&client).await?;
		while (current.number() as u64) > expiry_block {
			let parent_hash = current.header().parent_hash;
			current = client.blocks().at(parent_hash).await?;
		}
		assert_eq!(current.number() as u64, expiry_block);
		current.hash()
	};

	// Transactions[store_block] is None at expiry block (taken by on_initialize).
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
		tracing::info!("✓ Transactions[{}] is None at expiry block {}", store_block, expiry_block);
	}

	// set-1 (auto-renew) — TransactionByContentHash still points at the expiry block.
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
				tracing::trace!("set1[{}] hash={} → {:?}", i, hex::encode(hash), v);
				set1_renewed += 1;
			},
			None => {
				tracing::warn!(
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
	tracing::info!("✓ All {} set-1 (auto-renew) items still indexed at expiry block", set1_renewed);

	// set-2 (no auto-renew) — TransactionByContentHash removed by on_initialize.
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
				tracing::warn!(
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
	tracing::info!(
		"✓ All {} set-2 (no auto-renew) TransactionByContentHash entries cleaned up",
		set2_cleaned
	);

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
	tracing::info!(
		"✓ {} DataAutoRenewed events at expiry block {} (and zero AutoRenewalFailed)",
		auto_renewed,
		expiry_block
	);

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
	tracing::info!(
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
	tracing::info!("✓ mandatory weight at expiry block within max_block");

	test_log!(TEST, "=== on_initialize cleanup PASSED ===");
	network.destroy().await?;
	Ok(())
}

const ON_INIT_NO_RENEWALS_ITEMS: u32 =
	pallet_bulletin_transaction_storage::DEFAULT_MAX_BLOCK_TRANSACTIONS;

/// Isolate `Hooks::on_initialize` cost from `apply_block_inherents` drain by storing the
/// worst-case item count without auto-renewal. Logs the (expiry - idle baseline) mand
/// weight delta and asserts it fits within `max_block`.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_on_initialize_no_renewals_weight_test() -> Result<()> {
	const TEST: &str = "para_on_init_no_renewals";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== on_initialize cost in isolation ({} items, no auto-renew) ===",
		ON_INIT_NO_RENEWALS_ITEMS
	);

	verify_parachain_binaries()?;
	let config = build_parachain_network_config_three_relay_validators(get_para_node_args())?;
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

	let items: Vec<Vec<u8>> = (0..ON_INIT_NO_RENEWALS_ITEMS)
		.map(|i| {
			let mut p = b"ON_INIT_NO_RENEW_".to_vec();
			p.extend_from_slice(format!("{:04}_", i).as_bytes());
			generate_test_data(TEST_DATA_SIZE, &p)
		})
		.collect();

	let alice = dev::alice();
	let pre_store_block = current_best_block(&client).await?.number() as u64;
	tracing::info!(
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
	tracing::info!("All {} stores accepted into pool", ON_INIT_NO_RENEWALS_ITEMS);

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
	tracing::info!("Stores landed at block {}", store_block);

	let expiry_block = store_block + RETENTION_PERIOD as u64 + 1;
	wait_for_block_height(collator1, expiry_block + 2, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	// Proof for the stores fires at `store_block + RP = expiry_block - 1`, one block before
	// `on_initialize` takes `Transactions[store_block]` as obsolete.
	assert_proof_checked_at(&client, expiry_block - 1, "on_init_no_renewals post-store").await?;

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
	tracing::info!(
		"BlockWeights::max_block = (ref_time={}, proof_size={})",
		max_block_ref,
		max_block_pov
	);

	let mut idle_baseline_ref: Option<u64> = None;
	let mut idle_baseline_pov: Option<u64> = None;
	let mut expiry_mand: Option<(u64, u64)> = None;
	let mut weight_violations: Vec<String> = Vec::new();

	tracing::info!("--- Block weight stats (no auto-renewals) ---");
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

		tracing::info!(
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

		// Idle baseline: block after expiry has no expiry/pending/proof contribution.
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

	if let (Some(em), Some(idle_ref), Some(idle_pov)) =
		(expiry_mand, idle_baseline_ref, idle_baseline_pov)
	{
		let delta_ref = em.0.saturating_sub(idle_ref);
		let delta_pov = em.1.saturating_sub(idle_pov);
		tracing::info!(
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

const SOAK_DURATION_SECS: u64 = 60 * 60;
const SOAK_BLOCKS_PRUNING: u32 = 15;
const SOAK_RETENTION_PERIOD: u32 = 10;
const SOAK_VERIFY_INTERVAL_BLOCKS: u64 = 30;
/// Minimum blocks since last touch before we expect col11 eviction:
/// `pruning(15) + retention/finality_lag(10)`.
const SOAK_PRUNED_AGE_THRESHOLD: u64 = 25;
const SOAK_BITSWAP_TIMEOUT_SECS: u64 = 10;
const SOAK_AUTH_TX_SLOTS: u32 = 3000;

#[derive(Clone)]
struct SoakItem {
	data: Vec<u8>,
	content_hash: [u8; 32],
	last_touch_block: u64,
}

fn pseudo_random(seed: u64) -> u64 {
	let mut x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
	x ^= x << 13;
	x ^= x >> 7;
	x ^= x << 17;
	x
}

/// 60-minute soak on a 3-collator network: drive steady `store` / `renew_content_hash`
/// traffic, periodically verify that data older than the pruning window is no longer served.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_long_running_pruning_soak_test() -> Result<()> {
	const TEST: &str = "para_pruning_soak";
	crate::utils::init_logging();

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

	tracing::info!("==================== PJS LINKS ====================");
	for (name, node) in
		[("collator-1", collator1), ("collator-2", collator2), ("collator-3", collator3)]
	{
		tracing::info!(
			"[{}] PJS:    https://polkadot.js.org/apps/?rpc={}#/explorer",
			name,
			node.ws_uri()
		);
		tracing::info!("[{}] WS:     {}", name, node.ws_uri());
	}
	tracing::info!("===================================================");

	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	let mut nonce = get_alice_nonce(collator1).await?;
	tracing::info!("Setting RetentionPeriod to {} blocks", SOAK_RETENTION_PERIOD);
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
			tracing::info!(
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

		// Only sync the nonce forward: chain's `account_nonce` lags pool-pending txs and
		// syncing local-to-chain would wipe in-flight nonces and trigger pool dedup.
		if block_n.is_multiple_of(10) {
			match client.tx().account_nonce(&dev::alice().public_key().to_account_id()).await {
				Ok(chain_nonce) =>
					if chain_nonce > nonce {
						tracing::info!(
							"[soak] catching up local nonce: local={} chain={}",
							nonce,
							chain_nonce
						);
						nonce = chain_nonce;
					},
				Err(e) => tracing::warn!("[soak] account_nonce query failed: {}", e),
			}
		}

		// Fire-and-forget; the pruning verification step uses bitswap to confirm effects.
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
			Err(e) => tracing::warn!("[soak] store at block {} failed: {}", block_n, e),
		}

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
					Err(e) => tracing::warn!("[soak] renew at block {} failed: {}", block_n, e),
				}
			}
		}

		// Rotate verification across all three collators.
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
						tracing::info!("[soak] ✓ pruning verified on {}", label);
					},
					Err(e) => {
						total_pruned_verifications_failed += 1;
						tracing::warn!("[soak] ✗ pruning verification FAILED on {}: {}", label, e);
					},
				}
			}
		}
	}

	tracing::info!("=== Soak window elapsed; final tallies ===");
	tracing::info!("Total stores attempted/succeeded: {}", total_stores);
	tracing::info!("Total renews succeeded: {}", total_renews);
	tracing::info!(
		"Pruning verifications: {} ok / {} failed",
		total_pruned_verifications_ok,
		total_pruned_verifications_failed
	);
	tracing::info!("Tracked items: {}", stored.len());

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

// Restart-with-modified-args scenarios. zombienet-sdk-0.3.13 has no API to restart a node
// with modified args (`NetworkNode::restart()` reuses the original), so we SIGTERM the
// collator process and re-spawn `polkadot-omni-node` directly with the new `--blocks-pruning`.

const PRUNE_RESTART_INITIAL_BLOCKS_TARGET: u64 = 50;

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

async fn sigterm_and_wait(pid: u32) -> Result<()> {
	let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
	// Wait up to 5s for the process to release file locks.
	for _ in 0..10 {
		tokio::time::sleep(std::time::Duration::from_millis(500)).await;
		let still_alive = std::process::Command::new("kill")
			.args(["-0", &pid.to_string()])
			.status()
			.map(|s| s.success())
			.unwrap_or(false);
		if !still_alive {
			return Ok(());
		}
	}
	let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
	tokio::time::sleep(std::time::Duration::from_secs(1)).await;
	Ok(())
}

/// Re-spawn args with any existing `--blocks-pruning` stripped, the new one injected before
/// `--`, and all ports forced to 0 to avoid colliding with the killed process.
fn build_respawn_args(orig: &[String], new_pruning: Option<u32>) -> Vec<String> {
	let mut out = Vec::with_capacity(orig.len() + 4);
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
		if matches!(a.as_str(), "--rpc-port" | "--port" | "--prometheus-port") {
			out.push(a.clone());
			out.push("0".to_string());
			i += 2;
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
	let config = build_parachain_network_config_three_relay_validators(para_args)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("alice not found")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("collator-1 not found")?;
	tracing::info!(
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

	let orig_args: Vec<String> = collator1.args().iter().map(|s| s.to_string()).collect();
	let base_path = extract_arg_value(&orig_args, "--base-path")
		.ok_or_else(|| anyhow::anyhow!("collator-1 args do not contain --base-path"))?;
	tracing::info!("[{}] collator-1 base_path = {}", scenario, base_path);

	let pid = find_pid_by_base_path(&base_path)
		.ok_or_else(|| anyhow::anyhow!("could not find collator-1 PID via ps"))?;
	tracing::info!("[{}] sending SIGTERM to collator-1 pid={}", scenario, pid);
	sigterm_and_wait(pid).await?;
	tracing::info!("[{}] collator-1 process terminated", scenario);

	let respawn_args = build_respawn_args(&orig_args, restart_pruning);
	tracing::info!("[{}] re-spawning with --blocks-pruning = {:?}", scenario, restart_pruning);

	let binary = std::env::var("POLKADOT_PARACHAIN_BINARY_PATH")
		.unwrap_or_else(|_| "polkadot-omni-node".to_string());
	let (exit, log) =
		spawn_omni_node_capture(&binary, &respawn_args, std::time::Duration::from_secs(15)).await?;

	let relevant = pruning_related_lines(&log);
	let last_30: Vec<&str> = log.lines().rev().take(30).collect();
	let last_30: Vec<&str> = last_30.into_iter().rev().collect();

	tracing::info!(
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
	crate::utils::init_logging();
	test_log!(TEST, "=== Restart from archive (no --blocks-pruning) → --blocks-pruning=10 ===");
	run_pruning_restart_scenario("archive_to_10", None, Some(10)).await?;
	test_log!(TEST, "=== finished — see log above for substrate's response ===");
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_restart_pruning_increase_test() -> Result<()> {
	const TEST: &str = "restart_pruning_increase";
	crate::utils::init_logging();
	test_log!(TEST, "=== Restart from --blocks-pruning=10 → --blocks-pruning=20 (increase) ===");
	run_pruning_restart_scenario("10_to_20", Some(10), Some(20)).await?;
	test_log!(TEST, "=== finished — see log above for substrate's response ===");
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_restart_pruning_decrease_test() -> Result<()> {
	const TEST: &str = "restart_pruning_decrease";
	crate::utils::init_logging();
	test_log!(TEST, "=== Restart from --blocks-pruning=20 → --blocks-pruning=10 (decrease) ===");
	run_pruning_restart_scenario("20_to_10", Some(20), Some(10)).await?;
	test_log!(TEST, "=== finished — see log above for substrate's response ===");
	Ok(())
}

/// Cycles 1 and 2 fit Alice's `bytes_allowance`; cycle 3 trips `PERMANENT_ALLOWANCE_EXCEEDED`
/// and the pallet emits `AutoRenewalFailed`, removing the entry from `AutoRenewals`.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_quota_exhaustion_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_quota_exhaustion";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain Auto-Renewal Quota Exhaustion Test ===");

	let harness = archive_harness().await?;
	let collator1 = &harness.collator1;
	let client_owned = collator1.wait_client().await?;
	let client = &client_owned;

	let mut data_pattern = PARACHAIN_TEST_DATA_PATTERN.to_vec();
	data_pattern.extend_from_slice(b"_quota_exhaustion_");
	let data = generate_test_data(TEST_DATA_SIZE, &data_pattern);
	let (hash_hex, _) = content_hash_and_cid(&data);
	tracing::info!("Test data: {} bytes, hash={}", data.len(), hash_hex);

	let nonce = get_alice_nonce(collator1).await?;
	let (store_block, mut nonce) = authorize_and_store_data(collator1, &data, nonce).await?;
	verify_node_bitswap(collator1, &data, BITSWAP_TIMEOUT_SECS, "post-store").await?;

	// Alice's genesis authorization is ~10 MiB and `authorize_account` is additive — overwrite
	// the entry directly so the per-account cap trips after a known number of cycles.
	let l = data.len() as u64;
	override_alice_authorization(
		client,
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
	tracing::info!(
		"Data stored at block {}; authorization pinned to bytes_allowance = 2 × {}",
		store_block,
		data.len()
	);

	let content_hash = blake2_256(&data);
	enable_auto_renew(client, &content_hash, nonce).await?;
	tracing::info!("Auto-renewal enabled for {}", hash_hex);

	let cadence = RETENTION_PERIOD as u64 + 1;
	let r1 = store_block + cadence;
	let r2 = store_block + 2 * cadence;
	let r3 = store_block + 3 * cadence;

	for (cycle, renewal_block) in [(1u64, r1), (2, r2)] {
		let wait_until = renewal_block + 1;
		tracing::info!(
			"[cycle {}] Waiting for block {} (renewal at {})",
			cycle,
			wait_until,
			renewal_block
		);
		wait_for_block_height(collator1, wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
		dump_renewal_window(client, renewal_block, &format!("quota_exhaustion cycle {}", cycle))
			.await?;

		// Proof for cycle k's source tx_info lands at `renewal_block - 1`.
		assert_proof_checked_at(client, renewal_block - 1, &format!("cycle {}", cycle)).await?;

		let renewal_hash = block_hash_at(client, renewal_block).await?;
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
		tracing::info!("[cycle {}] ✓ DataAutoRenewed at block {}", cycle, renewal_block);
	}

	// Cycle 3: bytes_permanent (= 2L) + L > bytes_allowance (= 2L).
	let wait_until = r3 + 1;
	tracing::info!(
		"[cycle 3] Waiting for block {} (renewal at {}) — expected to fail",
		wait_until,
		r3
	);
	wait_for_block_height(collator1, wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	let r3_hash = block_hash_at(client, r3).await?;
	let events = client.blocks().at(r3_hash).await?.events().await?;
	let failed = count_event(&events, "AutoRenewalFailed");
	let renewed = count_event(&events, "DataAutoRenewed");
	tracing::info!("[cycle 3] block {}: renewed={}, failed={}", r3, renewed, failed);
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
	tracing::info!("[cycle 3] ✓ AutoRenewalFailed at block {}", r3);

	// Query at the renewal block's hash, not `at_latest` (which reads finalized state and
	// lags ~10s behind best on cumulus).
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
	tracing::info!("✓ AutoRenewals[{}] removed at block {}", hash_hex, r3);

	test_log!(TEST, "=== Parachain Auto-Renewal Quota Exhaustion Test PASSED ===");
	Ok(())
}

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

/// Locate the canonical block at `target` by walking back from the latest finalized block.
/// Caller is responsible for ensuring finalized has reached `target` first
/// (e.g. via `wait_for_finalized_height`).
async fn finalized_block_hash_at(
	client: &OnlineClient<SubstrateConfig>,
	target: u64,
) -> Result<subxt::utils::H256> {
	let mut current = current_finalized_block(client).await?;
	if (current.number() as u64) < target {
		anyhow::bail!(
			"finalized height {} has not reached target {}; wait for finalization first",
			current.number(),
			target
		);
	}
	while (current.number() as u64) > target {
		let parent_hash = current.header().parent_hash;
		current = client.blocks().at(parent_hash).await?;
	}
	Ok(current.hash())
}

/// Assert exactly one `ProofChecked` event at the given block; verifies the storage-proof
/// step of `apply_block_inherents` actually ran for `Transactions[block - RetentionPeriod]`.
/// Reads at the canonical (finalized) block — caller must ensure finality has reached `block`.
async fn assert_proof_checked_at(
	client: &OnlineClient<SubstrateConfig>,
	block: u64,
	context: &str,
) -> Result<()> {
	let hash = finalized_block_hash_at(client, block).await?;
	let events = client.blocks().at(hash).await?.events().await?;
	let count = count_event(&events, "ProofChecked");
	assert_eq!(count, 1, "{}: expected 1 ProofChecked at block {}, saw {}", context, block, count);
	Ok(())
}

/// Log event counts at `r-1..=r+2` so a failed assertion can distinguish "renewal fired in
/// a different block" from "renewal never fired".
async fn dump_renewal_window(
	client: &OnlineClient<SubstrateConfig>,
	r: u64,
	label: &str,
) -> Result<()> {
	let head = current_best_block(client).await?;
	tracing::info!(
		"[{}] diagnostic: chain best={}, examining blocks {}..={} for TransactionStorage events",
		label,
		head.number(),
		r.saturating_sub(1),
		r + 2
	);
	for n in r.saturating_sub(1)..=r + 2 {
		let hash = match block_hash_at(client, n).await {
			Ok(h) => h,
			Err(e) => {
				tracing::info!("[{}]   block {}: lookup failed ({})", label, n, e);
				continue;
			},
		};
		let events = client.blocks().at(hash).await?.events().await?;
		let renewed = count_event(&events, "DataAutoRenewed");
		let failed = count_event(&events, "AutoRenewalFailed");
		let enabled = count_event(&events, "AutoRenewalEnabled");
		let stored = count_event(&events, "Stored");
		tracing::info!(
			"[{}]   block {} ({}): Stored={} AutoRenewalEnabled={} DataAutoRenewed={} AutoRenewalFailed={}",
			label,
			n,
			hex::encode(&hash.0[..4]),
			stored,
			enabled,
			renewed,
			failed,
		);
	}
	Ok(())
}

/// Authorization expires between auto-renew cycles: cycle 1 succeeds, cycle 2 trips the
/// expired branch and emits `AutoRenewalFailed`, then re-authorizing exercises the
/// expired-but-present reset path (counters zeroed) for a fresh item.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_auto_renew_authorization_expires_mid_cycle_test() -> Result<()> {
	const TEST: &str = "para_auto_renew_auth_expires_mid_cycle";
	crate::utils::init_logging();

	test_log!(TEST, "=== Parachain Auto-Renewal Authorization Expires Mid-Cycle Test ===");

	let harness = archive_harness().await?;
	let collator1 = &harness.collator1;
	let client_owned = collator1.wait_client().await?;
	let client = &client_owned;

	let mut data_pattern = PARACHAIN_TEST_DATA_PATTERN.to_vec();
	data_pattern.extend_from_slice(b"_auth_expires_");
	let data = generate_test_data(TEST_DATA_SIZE, &data_pattern);
	let (hash_hex, _) = content_hash_and_cid(&data);
	tracing::info!("Test data: {} bytes, hash={}", data.len(), hash_hex);

	let nonce = get_alice_nonce(collator1).await?;
	let (store_block, mut nonce) = authorize_and_store_data(collator1, &data, nonce).await?;
	tracing::info!("Data stored at block {}", store_block);

	let cadence = RETENTION_PERIOD as u64 + 1;
	let r1 = store_block + cadence;
	let r2 = store_block + 2 * cadence;

	// `expired()` is `now >= expiration`; midpoint of (r1, r2] gives slack.
	let override_expiration: u32 = ((r1 + r2) / 2) as u32;
	tracing::info!(
		"Overriding Alice's authorization expiration: r1={}, r2={}, expiration={}",
		r1,
		r2,
		override_expiration
	);

	// Generous allowances so the renewal gate only fails on expiry, not the per-account cap.
	let l = data.len() as u64;
	override_alice_authorization(
		client,
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
	enable_auto_renew(client, &content_hash, nonce).await?;
	nonce += 1;
	tracing::info!("Auto-renewal enabled");

	wait_for_finalized_height(collator1, r1 + 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	// Proof for the original store fires at `r1 - 1 = store_block + RP`.
	assert_proof_checked_at(client, r1 - 1, "auth_expires cycle 1").await?;
	dump_renewal_window(client, r1, "auth_expires cycle 1").await?;
	let r1_hash = finalized_block_hash_at(client, r1).await?;
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
	tracing::info!("[cycle 1] ✓ DataAutoRenewed at block {}", r1);

	wait_for_finalized_height(collator1, r2 + 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	// Proof for the cycle-1 renewal at r1 fires at `r2 - 1 = r1 + RP`.
	assert_proof_checked_at(client, r2 - 1, "auth_expires cycle 2").await?;
	let r2_hash = finalized_block_hash_at(client, r2).await?;
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
	tracing::info!("[cycle 2] ✓ AutoRenewalFailed at block {}; AutoRenewals[hash] removed", r2);

	// Expired-but-present: the pallet zeroes counters and installs a fresh expiration.
	let alice_pk = subxt_signer::sr25519::dev::alice().public_key().0;
	authorize_account_via_sudo(
		client,
		&alice_pk,
		/* transactions */ 10,
		/* bytes */ 4 * l,
		nonce,
	)
	.await?;
	nonce += 1;
	tracing::info!("Re-authorized Alice — expects expired-reset branch to zero counters");

	// End-to-end proof the reset took: a fresh store + auto-renew cycle must succeed; if
	// `bytes_permanent` had carried over, the cycle-1 renewal would trip the per-account cap.
	let data2 = {
		let mut pattern = PARACHAIN_TEST_DATA_PATTERN.to_vec();
		pattern.extend_from_slice(b"_AFTER_REAUTH_");
		generate_test_data(TEST_DATA_SIZE, &pattern)
	};
	let (hash2_hex, _) = content_hash_and_cid(&data2);

	let store2_block = submit_store_signed(client, &data2, nonce).await?;
	nonce += 1;
	tracing::info!("Stored second item at block {} (hash={})", store2_block, hash2_hex);

	let content_hash2 = blake2_256(&data2);
	enable_auto_renew(client, &content_hash2, nonce).await?;
	tracing::info!("Auto-renewal enabled for second item");

	let r1_after = store2_block + cadence;
	wait_for_finalized_height(collator1, r1_after + 1, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	// Proof for the second store fires at `r1_after - 1 = store2_block + RP`.
	assert_proof_checked_at(client, r1_after - 1, "post-reauth cycle 1").await?;
	let r1_after_hash = finalized_block_hash_at(client, r1_after).await?;
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
	tracing::info!("✓ Post-reauth cycle 1 succeeded — counters reset");

	// Shared-harness cleanup: `data` was removed by the cycle-2 failure path; disable `data2`
	// explicitly so it doesn't keep renewing for the rest of the harness lifetime.
	let nonce_after_enable = nonce + 1;
	disable_auto_renew(client, &content_hash2, nonce_after_enable).await?;
	tracing::info!("✓ Disabled auto-renew for data2 — chain idle for the next test");

	test_log!(TEST, "=== Parachain Auto-Renewal Authorization Expires Mid-Cycle Test PASSED ===");
	Ok(())
}
