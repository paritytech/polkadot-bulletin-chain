// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! HOP (Hand-Off Protocol) promotion end-to-end: `hop_submit` -> wait for the on-chain
//! `Stored` event -> wait `RetentionPeriod` -> assert `ProofChecked` covers our blob, and
//! bitswap serves the original content via the published CID. A pre-promotion bitswap
//! probe asserts the blob is *not* yet served — guarding against a stale col11 entry
//! leaking through.

use crate::{
	test_log,
	utils::{
		assert_proof_checked_at, authorize_account_via_sudo_finalized, blake2_256,
		build_parachain_network_config_three_relay_validators, canonical_store_block,
		content_hash_and_cid, finalized_block_hash_at, generate_test_data, get_alice_nonce,
		hop_submit, initialize_network, now_ms, set_retention_period_finalized,
		verify_bitswap_fetch, verify_parachain_binaries, wait_for_session_change_on_node,
		NETWORK_READY_TIMEOUT_SECS, NODE_LOG_CONFIG, TEST_DATA_SIZE,
	},
};
use anyhow::{Context, Result};
use std::time::Duration;
use subxt::{config::substrate::SubstrateConfig, OnlineClient};
use subxt_signer::sr25519::dev;

/// Short on-chain retention so the proof block lands within the test window.
const RETENTION_PERIOD: u32 = 10;
/// HOP entry expiration; the proof must be promotable *immediately*, so
/// `HOP_PROMOTION_BUFFER_SECS > HOP_RETENTION_SECS`.
const HOP_RETENTION_SECS: u64 = 10;
const HOP_PROMOTION_BUFFER_SECS: u64 = 60;
/// Maintenance loop cadence — promotion lands within ~one tick of submission.
const HOP_CHECK_INTERVAL_SECS: u64 = 5;

const SESSION_CHANGE_TIMEOUT_SECS: u64 = 300;
const PROMOTION_TIMEOUT_SECS: u64 = 120;
const BITSWAP_TIMEOUT_SECS: u64 = 20;

fn get_para_node_args() -> Vec<String> {
	vec![
		"--ipfs-server".into(),
		"--enable-hop".into(),
		"--hop-disable-rate-limit".into(),
		format!("--hop-retention-secs={}", HOP_RETENTION_SECS),
		format!("--hop-promotion-buffer-secs={}", HOP_PROMOTION_BUFFER_SECS),
		format!("--hop-check-interval={}", HOP_CHECK_INTERVAL_SECS),
		format!("{},hop=trace,txpool=debug", NODE_LOG_CONFIG),
		// Arguments after "--" are passed to the embedded relay chain client.
		"--".into(),
		"--network-backend=libp2p".into(),
	]
}

/// Subscribe to best blocks and return the number of the first one whose events contain
/// a `TransactionStorage::Stored` with our `content_hash`. Times out after `timeout_secs`.
async fn wait_for_promoted(
	client: &OnlineClient<SubstrateConfig>,
	content_hash: &[u8; 32],
	timeout_secs: u64,
) -> Result<u64> {
	let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
	let mut sub = client.blocks().subscribe_best().await?;
	while std::time::Instant::now() < deadline {
		let Ok(Some(Ok(block))) = tokio::time::timeout(Duration::from_secs(5), sub.next()).await
		else {
			continue;
		};
		let events = block.events().await?;
		let hit = events.iter().filter_map(|e| e.ok()).any(|e| {
			e.pallet_name() == "TransactionStorage" &&
				e.variant_name() == "Stored" &&
				e.field_bytes().windows(32).any(|w| w == content_hash)
		});
		if hit {
			return Ok(block.number() as u64);
		}
	}
	anyhow::bail!(
		"HOP promotion not observed within {}s — no Stored event for 0x{}",
		timeout_secs,
		hex::encode(content_hash)
	)
}

#[tokio::test(flavor = "multi_thread")]
async fn parachain_hop_promotion_bitswap_test() -> Result<()> {
	const TEST: &str = "para_hop_promotion";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== HOP promotion (RP={}, hop_retention={}s, hop_buffer={}s) ===",
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

	// Small `RetentionPeriod` so the proof block lands within the test window.
	let mut nonce = get_alice_nonce(collator1).await?;
	set_retention_period_finalized(&client, RETENTION_PERIOD, nonce).await?;
	nonce += 1;

	// `can_account_promote` only requires Alice's authorization to exist + be unexpired —
	// the per-extent counters aren't debited by HOP, but `authorize_account` won't write a
	// zero-extent entry, so pass a generous amount.
	let alice = dev::alice();
	authorize_account_via_sudo_finalized(
		&client,
		&alice.public_key().0,
		10,
		(TEST_DATA_SIZE as u64) * 8,
		nonce,
	)
	.await?;

	// Salt with a wall-clock suffix so re-runs don't collide on content hash.
	let mut pattern = b"HOP_PROMOTION_TEST_".to_vec();
	pattern.extend_from_slice(format!("{}_", now_ms()).as_bytes());
	let data = generate_test_data(TEST_DATA_SIZE, &pattern);
	let content_hash = blake2_256(&data);
	let (hash_hex, cid) = content_hash_and_cid(&data);
	tracing::info!("test data: {} bytes, content_hash={}, CID={}", data.len(), hash_hex, cid);

	let multiaddr = collator1.multiaddr().to_string();
	let ws_uri = collator1.ws_uri().to_string();

	// Bitswap probe BEFORE promotion: blob lives only in the HOP pool, col11 has no entry.
	let before_match = verify_bitswap_fetch(&multiaddr, &data, BITSWAP_TIMEOUT_SECS)
		.await
		.unwrap_or(false);
	tracing::info!("bitswap BEFORE promotion: match={}", before_match);

	// `hop_submit` -> maintenance task promotes -> `TransactionStorage::Stored` on-chain.
	let entry_count = hop_submit(&ws_uri, &alice, &data, &[alice.public_key().0], now_ms()).await?;
	tracing::info!("hop_submit OK; pool entry_count={}", entry_count);
	let store_block = wait_for_promoted(&client, &content_hash, PROMOTION_TIMEOUT_SECS).await?;
	tracing::info!("✓ HOP promotion landed at block {}", store_block);

	// Bitswap probe AFTER promotion: blob is now on-chain; bitswap *should* match.
	let after_match = verify_bitswap_fetch(&multiaddr, &data, BITSWAP_TIMEOUT_SECS)
		.await
		.unwrap_or(false);
	tracing::info!("bitswap AFTER promotion: match={}", after_match);

	// Tie `ProofChecked` to our blob: the inherent at N proves `Transactions[N - RP]`, so
	// confirm the content hash is still indexed at `store_block` when the proof fires.
	let proof_block = store_block + RETENTION_PERIOD as u64;
	let proof_hash = finalized_block_hash_at(&client, proof_block).await?;
	let indexed_at = canonical_store_block(&client, proof_hash, &content_hash).await?;
	assert_eq!(
		indexed_at, store_block,
		"HOP blob indexed at {} but proof at {} reads Transactions[{}]",
		indexed_at, proof_block, store_block,
	);
	assert_proof_checked_at(&client, proof_block, "HOP-promoted blob").await?;
	tracing::info!("✓ ProofChecked at block {} covers HOP blob {}", proof_block, hash_hex);

	// BEFORE is expected to be `false` (blob lives only in the HOP pool, no col11 entry yet).
	// Guard against a stale col11 entry leaking through but don't fail the test on the
	// tautological case. The real signal is the AFTER probe.
	assert!(
		!before_match,
		"bitswap returned matching content BEFORE promotion — stale col11 entry?",
	);
	assert!(
		after_match,
		"bitswap did not match AFTER promotion at block {} (proof at {}) — \
		 HOP -> col11/bitswap gap",
		store_block, proof_block,
	);

	test_log!(TEST, "=== HOP promotion bitswap test PASSED ===");
	network.destroy().await?;
	Ok(())
}
