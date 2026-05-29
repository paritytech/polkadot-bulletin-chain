// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end test for the HOP (Hand-Off Protocol) promotion flow on a single-collator
//! parachain network.
//!
//! Flow exercised:
//! 1. Spawn a HOP-enabled collator with a small on-chain `RetentionPeriod`, a small HOP retention,
//!    and a `promotion_buffer >= hop_retention` so submitted entries become promotable immediately.
//! 2. Authorize Alice for HOP promotion (`pallet_bulletin_transaction_storage::authorize_account`).
//! 3. Submit `hop_submit` over JSON-RPC with Alice's sr25519 signature.
//! 4. Wait for the collator's maintenance task to land the on-chain `HopPromotion::promote`
//!    extrinsic (detected via `TransactionStorage::Stored`).
//! 5. Wait for `store_block + RetentionPeriod + 1` so the runtime's storage-proof inherent runs
//!    against the promoted blob — strong assertion that `ProofChecked` fires.
//! 6. Bitswap-fetch the content both before and after promotion and compare against the original.
//!    **This part is expected to fail** — surfacing whatever bug the test is meant to drive out
//!    (e.g. CID/chunking mismatch or missing col11 indexing on the promotion path). The proof check
//!    above passes; the bitswap check is the demonstration.

use crate::{
	test_log,
	utils::{
		assert_proof_checked_at, authorize_account_via_sudo_finalized, blake2_256,
		build_parachain_network_config_three_relay_validators, canonical_store_block,
		content_hash_and_cid, finalized_block_hash_at, generate_test_data, get_alice_nonce,
		hop_submit, initialize_network, now_ms, set_retention_period_finalized,
		verify_bitswap_fetch, verify_parachain_binaries, wait_for_block_height,
		wait_for_finalized_height, wait_for_session_change_on_node, NETWORK_READY_TIMEOUT_SECS,
		TEST_DATA_SIZE,
	},
};
use anyhow::{Context, Result};
use std::time::Duration;
use subxt::{config::substrate::SubstrateConfig, OnlineClient};
use subxt_signer::sr25519::dev;

/// Short on-chain retention so the proof block lands within the test window.
const RETENTION_PERIOD: u32 = 10;

/// HOP data pool retention. Picked smaller than the `promotion_buffer` below so
/// every submitted entry is in the "near-expiry" promotion window immediately.
const HOP_RETENTION_SECS: u64 = 10;
/// Must be greater than `HOP_RETENTION_SECS` so `get_promotable` returns the
/// entry on the first tick after submission.
const HOP_PROMOTION_BUFFER_SECS: u64 = 60;
/// Short maintenance cadence so the promotion lands within seconds of submission.
const HOP_CHECK_INTERVAL_SECS: u64 = 5;

const SESSION_CHANGE_TIMEOUT_SECS: u64 = 300;
const BLOCK_PRODUCTION_TIMEOUT_SECS: u64 = 300;
const PROMOTION_TIMEOUT_SECS: u64 = 120;
const BITSWAP_TIMEOUT_SECS: u64 = 20;

/// Logging for HOP — extends the standard target list with `hop=trace` so a failed
/// "promotion never landed" surfaces the maintenance-loop diagnostics, plus the
/// promotion's `Submitted/Failed` lines from `sc-hop`.
const HOP_NODE_LOG_CONFIG: &str =
	"-lsync=trace,sub-libp2p=trace,litep2p=trace,request-response=trace,\
	transaction-storage=trace,bitswap=trace,hop=trace,txpool=debug";

fn get_para_node_args() -> Vec<String> {
	vec![
		"--ipfs-server".into(),
		"--enable-hop".into(),
		"--hop-disable-rate-limit".into(),
		format!("--hop-retention-secs={}", HOP_RETENTION_SECS),
		format!("--hop-promotion-buffer-secs={}", HOP_PROMOTION_BUFFER_SECS),
		format!("--hop-check-interval={}", HOP_CHECK_INTERVAL_SECS),
		HOP_NODE_LOG_CONFIG.into(),
		// Arguments after "--" are passed to the embedded relay chain client.
		"--".into(),
		"--network-backend=libp2p".into(),
	]
}

/// Poll the chain for a `TransactionStorage::Stored` event matching `content_hash` up to
/// `timeout_secs`. Returns the canonical block number the promotion landed in.
async fn wait_for_promoted(
	client: &OnlineClient<SubstrateConfig>,
	content_hash: &[u8; 32],
	timeout_secs: u64,
) -> Result<u64> {
	let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
	let mut blocks_sub = client.blocks().subscribe_best().await?;
	while std::time::Instant::now() < deadline {
		match tokio::time::timeout(Duration::from_secs(5), blocks_sub.next()).await {
			Ok(Some(Ok(block))) => {
				let events = block.events().await?;
				for ev in events.iter().filter_map(|e| e.ok()) {
					if ev.pallet_name() == "TransactionStorage" &&
						ev.variant_name() == "Stored" &&
						ev.field_bytes().windows(32).any(|w| w == content_hash)
					{
						return Ok(block.number() as u64);
					}
				}
			},
			Ok(Some(Err(e))) => anyhow::bail!("block subscription error: {e}"),
			Ok(None) => anyhow::bail!("block subscription ended unexpectedly"),
			Err(_) => continue,
		}
	}
	anyhow::bail!(
		"HOP promotion not observed within {}s — no Stored event for content_hash 0x{}",
		timeout_secs,
		hex::encode(content_hash)
	)
}

/// HOP-promotion smoke test. Demonstrates the proof-check path works while the
/// bitswap-content-match check is **expected to fail** (the bug this test surfaces).
#[tokio::test(flavor = "multi_thread")]
async fn parachain_hop_promotion_bitswap_test() -> Result<()> {
	const TEST: &str = "para_hop_promotion";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== Parachain HOP promotion (RP={}, hop_retention={}s, hop_buffer={}s) ===",
		RETENTION_PERIOD,
		HOP_RETENTION_SECS,
		HOP_PROMOTION_BUFFER_SECS,
	);

	verify_parachain_binaries()?;

	let config = build_parachain_network_config_three_relay_validators(get_para_node_args())?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

	let relay_alice = network.get_node("alice").context("get relay alice")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

	let collator1 = network.get_node("collator-1").context("get collator-1")?;
	let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;

	// Set on-chain RetentionPeriod=10 (finalize so subsequent reads see the change).
	let mut nonce = get_alice_nonce(collator1).await?;
	set_retention_period_finalized(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// Authorize Alice for at least one promotion's worth of bytes.
	// `account_has_active_authorization` only requires the entry to exist + be unexpired — the
	// per-extent counters aren't debited by HOP promotions, but `authorize_account` won't write an
	// entry with zero extent, so give it a generous chunk to make the active state unambiguous.
	let alice_pk = dev::alice().public_key().0;
	authorize_account_via_sudo_finalized(
		&client,
		&alice_pk,
		10,
		(TEST_DATA_SIZE as u64).saturating_mul(8),
		nonce,
	)
	.await?;
	// `account_nonce` reads finalized state and the call above finalized — refresh from
	// the chain so we don't double-spend the nonce in any later signed op.
	let _ = nonce; // unused from here

	// Generate distinct test data (salt with timestamp suffix to keep the content hash
	// fresh across re-runs of the same chain image).
	let mut pattern = b"HOP_PROMOTION_TEST_".to_vec();
	pattern.extend_from_slice(format!("{}_", now_ms()).as_bytes());
	let data = generate_test_data(TEST_DATA_SIZE, &pattern);
	let content_hash = blake2_256(&data);
	let (hash_hex, cid) = content_hash_and_cid(&data);
	tracing::info!("Test data: {} bytes, content_hash={}, CID={}", data.len(), hash_hex, cid);

	// --- Bitswap probe BEFORE promotion ----------------------------------------------------
	// At this point the blob lives only in the HOP data pool — col11 has no entry, so
	// bitswap MUST NOT serve matching content. We assert "bitswap content matches original"
	// here as part of the demonstration: this assertion is expected to fail.
	test_log!(TEST, "Bitswap probe BEFORE promotion (expected: no match yet — drives the bug)");
	let ws_uri = collator1.ws_uri().to_string();
	let multiaddr = collator1.multiaddr().to_string();

	let before_match = verify_bitswap_fetch(&multiaddr, &data, BITSWAP_TIMEOUT_SECS)
		.await
		.unwrap_or(false);
	tracing::info!("[BEFORE promotion] bitswap content match = {}", before_match);

	// --- HOP submit ---------------------------------------------------------------------
	// Alice signs the submit payload; recipient list contains Alice's own key so the
	// runtime side doesn't reject the recipient encoding.
	let alice = dev::alice();
	let recipients = [alice.public_key().0];
	let submit_ts = now_ms();
	tracing::info!("Submitting hop_submit via {} (ts_ms={})", ws_uri, submit_ts);
	let entry_count = hop_submit(&ws_uri, &alice, &data, &recipients, submit_ts)
		.await
		.context("hop_submit RPC call failed")?;
	tracing::info!("hop_submit OK; pool entry_count={}", entry_count);

	// --- Wait for promotion ----------------------------------------------------------
	// The maintenance loop runs every `HOP_CHECK_INTERVAL_SECS`; the entry is in the
	// promotion window immediately because `buffer > retention`.
	let store_block = wait_for_promoted(&client, &content_hash, PROMOTION_TIMEOUT_SECS)
		.await
		.context("HOP promotion did not land on-chain")?;
	tracing::info!("✓ HOP promotion landed at block {} (Stored event observed)", store_block);

	// --- Bitswap probe AFTER promotion -----------------------------------------------
	// With the blob promoted on-chain, bitswap *should* match the original content.
	// The user-visible failure of this test is here.
	test_log!(TEST, "Bitswap probe AFTER promotion (expected: match — drives the bug if not)");
	let after_match = verify_bitswap_fetch(&multiaddr, &data, BITSWAP_TIMEOUT_SECS)
		.await
		.unwrap_or(false);
	tracing::info!("[AFTER promotion] bitswap content match = {}", after_match);

	// --- Wait for retention period, assert the proof at `store_block + RP` covers our blob ---
	// The runtime proves `Transactions[N - RetentionPeriod]` at block N, so confirming the
	// HOP content hash is still indexed at `store_block` when the proof fires ties the
	// `ProofChecked` event back to *our* promoted blob.
	let proof_block = store_block + RETENTION_PERIOD as u64;
	let wait_until = proof_block + 1;
	tracing::info!("Waiting for finalized block {} (proof at {})", wait_until, proof_block);
	wait_for_block_height(collator1, wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	wait_for_finalized_height(collator1, wait_until, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;

	let proof_hash = finalized_block_hash_at(&client, proof_block).await?;
	let indexed_at = canonical_store_block(&client, proof_hash, &content_hash).await?;
	assert_eq!(
		indexed_at, store_block,
		"HOP blob {} indexed at {} but proof at {} reads Transactions[{}]",
		hash_hex, indexed_at, proof_block, store_block,
	);
	assert_proof_checked_at(&client, proof_block, "HOP-promoted blob").await?;
	tracing::info!("✓ ProofChecked at block {} covers HOP blob {}", proof_block, hash_hex);

	// --- The bitswap content-match demonstration --------------------------------------
	// Both probes should ideally match the original. They will not — that's the goal of
	// this test. Assert here so the test exits with a clear failure pointing at the gap.
	if !before_match {
		anyhow::bail!(
			"BEFORE promotion: bitswap did not return content matching the original. \
			 (Pre-promotion the blob lives only in the HOP pool, not in col11, so this is \
			 expected — but the assertion is part of demonstrating the bitswap/HOP gap.)"
		);
	}
	if !after_match {
		anyhow::bail!(
			"AFTER promotion: bitswap did not return content matching the original at \
			 block {} (proof_block={}). The promote extrinsic landed (Stored event seen) \
			 and the storage-proof inherent ran (ProofChecked event seen), but bitswap is \
			 still not serving the blob via the published CID. This is the bug the test is \
			 meant to surface.",
			store_block,
			proof_block,
		);
	}

	test_log!(TEST, "=== Parachain HOP promotion bitswap test PASSED ===");
	network.destroy().await?;
	Ok(())
}
