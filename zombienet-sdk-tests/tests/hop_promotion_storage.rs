// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! HOP (Hand-Off Protocol) end-to-end over three blobs, asserting `sc-hop`'s
//! `substrate_hop_*` Prometheus metrics and `sp_hop::HopRuntimeApi` along the way.
//!
//! Acking removes the entry, so one blob cannot cover both the ack path and the
//! promotion path:
//!
//! * **A** — `hop_submit` -> `hop_claim` -> `hop_ack`; leaves the pool via ack.
//! * **B** — `hop_submit` -> promotion -> on-chain `Stored` -> `ProofChecked` + bitswap. A
//!   pre-promotion bitswap probe asserts the blob is *not* yet served, guarding against a stale
//!   col11 entry leaking through.
//! * **C** — `hop_submit` from an account with no authorization; the RPC is rejected.
//!
//! Metrics are per-node. Every read targets `collator-1`, the node that received the
//! submits and whose maintenance task performs the promotions.
//!
//! [`parachain_hop_unpromoted_expiry_test`] covers the data-loss counter on its own
//! network: it needs `buffer < retention`, which contradicts blob B above.

use crate::{
	test_log,
	utils::{
		assert_hop_metrics_registered, assert_proof_checked_at,
		authorize_account_via_sudo_finalized, blake2_256,
		build_parachain_network_config_three_relay_validators, canonical_store_block,
		finalized_block_hash_at, generate_test_data, get_alice_nonce, hash_to_cid, hop_ack,
		hop_api, hop_claim, hop_metric, hop_pool_status, hop_submit, initialize_network, now_ms,
		override_alice_authorization, set_retention_period_finalized, verify_bitswap_fetch,
		verify_parachain_binaries, wait_for_finalized_quiescence, wait_for_session_change_on_node,
		wait_hop_metric, AuthorizationOverride, HopCounters, FINALIZED_TRANSACTION_TIMEOUT_SECS,
		HOP_ACK_NOT_FOUND_METRIC, HOP_CLAIM_NOT_FOUND_METRIC, HOP_POOL_BYTES_METRIC,
		HOP_POOL_ENTRIES_METRIC, HOP_POOL_MAX_BYTES_METRIC, HOP_PROMOTIONS_CONFIRMED_METRIC,
		HOP_PROMOTION_BACKLOG_METRIC, HOP_REMOVED_ACKED_METRIC,
		HOP_REMOVED_EXPIRED_PROMOTED_METRIC, HOP_REMOVED_EXPIRED_UNPROMOTED_METRIC,
		HOP_SUBMIT_NOT_AUTHORIZED_METRIC, NETWORK_READY_TIMEOUT_SECS, NODE_LOG_CONFIG,
		TEST_DATA_SIZE,
	},
};
use anyhow::{Context, Result};
use std::time::Duration;
use subxt::{backend::rpc::RpcClient, config::substrate::SubstrateConfig, OnlineClient};
use subxt_signer::sr25519::{dev, Keypair};

/// Short on-chain retention so the proof block lands within the test window.
const RETENTION_PERIOD: u32 = 10;
/// HOP entry expiration; the proof must be promotable *immediately*, so
/// `HOP_PROMOTION_BUFFER_SECS > HOP_RETENTION_SECS`.
const HOP_RETENTION_SECS: u64 = 10;
const HOP_PROMOTION_BUFFER_SECS: u64 = 60;
/// Maintenance loop cadence — promotion lands within ~one tick of submission.
const HOP_CHECK_INTERVAL_SECS: u64 = 5;

/// Inverted against the promotion test: with `buffer < retention` a fresh entry stays
/// outside the promotion window for `retention - buffer` seconds, which is the room
/// [`parachain_hop_unpromoted_expiry_test`] needs to expire the authorization before the
/// first promotion attempt.
///
/// The 30s gap is far more than the authorization override needs, and leaves the entry
/// inside the window — and so counted in `_promotion_backlog` — for the remaining 60s.
const UNPROMOTED_HOP_RETENTION_SECS: u64 = 90;
const UNPROMOTED_HOP_PROMOTION_BUFFER_SECS: u64 = 60;

/// `can_account_promote` never debits these, but `authorize_account` refuses to write a
/// zero-extent entry.
const AUTH_TRANSACTIONS: u32 = 10;
const AUTH_BYTES: u64 = (TEST_DATA_SIZE as u64) * 8;

const SESSION_CHANGE_TIMEOUT_SECS: u64 = 300;
const PROMOTION_TIMEOUT_SECS: u64 = 120;
const BITSWAP_TIMEOUT_SECS: u64 = 20;
/// Metric assertions poll the Prometheus endpoint; generous under CI load.
const HOP_METRIC_TIMEOUT_SECS: u64 = 120;
/// Must outlast `UNPROMOTED_HOP_RETENTION_SECS` plus one cleanup tick.
const UNPROMOTED_EXPIRY_TIMEOUT_SECS: u64 = 240;

fn hop_node_args(retention_secs: u64, promotion_buffer_secs: u64) -> Vec<String> {
	vec![
		"--ipfs-server".into(),
		"--enable-hop".into(),
		"--hop-disable-rate-limit".into(),
		format!("--hop-retention-secs={}", retention_secs),
		format!("--hop-promotion-buffer-secs={}", promotion_buffer_secs),
		format!("--hop-check-interval={}", HOP_CHECK_INTERVAL_SECS),
		format!("{},hop=trace,txpool=debug", NODE_LOG_CONFIG),
		// Arguments after "--" are passed to the embedded relay chain client.
		"--".into(),
		"--network-backend=libp2p".into(),
	]
}

fn get_para_node_args() -> Vec<String> {
	hop_node_args(HOP_RETENTION_SECS, HOP_PROMOTION_BUFFER_SECS)
}

/// Network plus everything both tests drive it through. Alice is authorized at genesis;
/// blob C uses a different account to get an unauthorized submit.
struct HopEnv {
	network: zombienet_sdk::Network<zombienet_sdk::LocalFileSystem>,
	collator1: zombienet_sdk::NetworkNode,
	client: OnlineClient<SubstrateConfig>,
	/// One connection for every HOP JSON-RPC call in the test.
	rpc: RpcClient,
	alice: Keypair,
	alice_id: [u8; 32],
	next_nonce: u64,
}

impl HopEnv {
	async fn spawn(para_args: Vec<String>) -> Result<Self> {
		verify_parachain_binaries()?;

		let config = build_parachain_network_config_three_relay_validators(para_args)?;
		let network = initialize_network(config).await?;
		network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;

		let relay_alice = network.get_node("alice").context("get relay alice")?;
		wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;

		let collator1 = network.get_node("collator-1").context("get collator-1")?.clone();
		let client: OnlineClient<SubstrateConfig> = collator1.wait_client().await?;
		let rpc = collator1.rpc().await?;

		// A disabled registry degrades to no-op metrics and absent series read as 0, so
		// every "counter is zero" assertion would pass vacuously. Ticks prove it is wired.
		assert_hop_metrics_registered(&collator1, HOP_METRIC_TIMEOUT_SECS).await?;

		let alice = dev::alice();
		let alice_id = alice.public_key().0;
		let next_nonce = get_alice_nonce(&collator1).await?;
		Ok(Self { network, collator1, client, rpc, alice, alice_id, next_nonce })
	}

	fn take_nonce(&mut self) -> u64 {
		let nonce = self.next_nonce;
		self.next_nonce += 1;
		nonce
	}

	async fn authorize_alice(&mut self) -> Result<()> {
		let nonce = self.take_nonce();
		authorize_account_via_sudo_finalized(
			&self.client,
			&self.alice_id,
			AUTH_TRANSACTIONS,
			AUTH_BYTES,
			nonce,
		)
		.await
	}

	/// Submit `data` naming Alice as the sole recipient.
	async fn submit(&self, data: &[u8]) -> Result<u64> {
		hop_submit(&self.rpc, &self.alice, data, &[self.alice_id], now_ms()).await
	}

	/// Submit `data` signed by, and addressed to, `signer`.
	async fn submit_as(&self, signer: &Keypair, data: &[u8]) -> Result<u64> {
		hop_submit(&self.rpc, signer, data, &[signer.public_key().0], now_ms()).await
	}
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

/// Distinct test data per blob, salted with wall-clock so re-runs don't collide on
/// content hash.
fn blob(label: &str) -> (Vec<u8>, [u8; 32]) {
	let mut pattern = format!("HOP_PROMOTION_TEST_{label}_").into_bytes();
	pattern.extend_from_slice(format!("{}_", now_ms()).as_bytes());
	let data = generate_test_data(TEST_DATA_SIZE, &pattern);
	let content_hash = blake2_256(&data);
	(data, content_hash)
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

	let mut env = HopEnv::spawn(get_para_node_args()).await?;
	let collator1 = env.collator1.clone();
	let multiaddr = collator1.multiaddr().to_string();

	// Small `RetentionPeriod` so the proof block lands within the test window.
	let nonce = env.take_nonce();
	set_retention_period_finalized(&env.client, RETENTION_PERIOD, nonce).await?;

	// ── Blob C: submit from an unauthorized account ──
	// Bob is in neither `accountAuthorizations` nor `allowedAuthorizers` at genesis
	// (Alice is authorized, Eve is the authorizer), so he cannot promote. `sc-hop`
	// consults `can_account_promote` before touching the pool, so the rejected submit
	// never becomes an entry — it shows up only in `_rpc_errors_total`.
	test_log!(TEST, "--- Blob C: unauthorized submit ---");
	let (data_c, _) = blob("C");
	let bob = dev::bob();

	let max_size = hop_api::max_promotion_size(&env.client).await?;
	assert!(
		max_size as usize >= TEST_DATA_SIZE,
		"max_promotion_size ({}) is below the test payload size ({})",
		max_size,
		TEST_DATA_SIZE,
	);
	assert!(
		!hop_api::can_account_promote(&env.client, &bob.public_key().0, TEST_DATA_SIZE as u32)
			.await?,
		"can_account_promote is true for an unauthorized account",
	);

	let submit_c = env.submit_as(&bob, &data_c).await;
	assert!(submit_c.is_err(), "unauthorized hop_submit succeeded: {:?}", submit_c);
	tracing::info!("unauthorized hop_submit rejected: {}", submit_c.unwrap_err());

	wait_hop_metric(
		&collator1,
		HOP_SUBMIT_NOT_AUTHORIZED_METRIC,
		|errors| errors >= 1,
		HOP_METRIC_TIMEOUT_SECS,
		"unauthorized submit was not counted",
	)
	.await?;

	// Claim/ack a hash that was never submitted, to reach the `hop_claim` / `hop_ack`
	// method labels. Both map any recipient mismatch onto `NotFound`, so an unknown hash
	// and a wrong signer are indistinguishable by design.
	const ABSENT_HASH: [u8; 32] = [0xCD; 32];
	let claim_absent = hop_claim(&env.rpc, &env.alice, &ABSENT_HASH).await;
	assert!(claim_absent.is_err(), "hop_claim succeeded for an absent hash");
	let ack_absent = hop_ack(&env.rpc, &env.alice, &ABSENT_HASH).await;
	assert!(ack_absent.is_err(), "hop_ack succeeded for an absent hash");

	for (metric, method) in
		[(HOP_CLAIM_NOT_FOUND_METRIC, "hop_claim"), (HOP_ACK_NOT_FOUND_METRIC, "hop_ack")]
	{
		wait_hop_metric(
			&collator1,
			metric,
			|errors| errors >= 1,
			HOP_METRIC_TIMEOUT_SECS,
			&format!("{method} on an absent hash was not counted"),
		)
		.await?;
	}
	test_log!(TEST, "✓ Blob C, claim and ack failures counted in _rpc_errors_total");

	// Alice is authorized at genesis; this refreshes the entry to a known extent.
	env.authorize_alice().await?;
	assert!(
		hop_api::can_account_promote(&env.client, &env.alice_id, TEST_DATA_SIZE as u32).await?,
		"can_account_promote is false for the authorized account",
	);

	let baseline = HopCounters::read(&collator1).await?;
	let max_bytes = hop_metric(&collator1, HOP_POOL_MAX_BYTES_METRIC).await?;
	assert!(max_bytes > 0, "substrate_hop_pool_max_bytes is 0");

	// ── Blob A: submit -> claim -> ack ──
	// Claim and ack immediately: with `hop_retention_secs` this short, an entry left
	// sitting would be promoted by the next maintenance tick and then expire.
	test_log!(TEST, "--- Blob A: submit -> claim -> ack ---");
	let (data_a, hash_a) = blob("A");
	let entry_count = env.submit(&data_a).await?;
	tracing::info!("hop_submit A OK; pool entry_count={}", entry_count);
	assert!(entry_count >= 1, "pool reported {} entries right after submit", entry_count);

	let claimed = hop_claim(&env.rpc, &env.alice, &hash_a).await?;
	assert_eq!(claimed, data_a, "hop_claim returned {} bytes != submitted blob", claimed.len());
	hop_ack(&env.rpc, &env.alice, &hash_a).await?;

	wait_hop_metric(
		&collator1,
		HOP_REMOVED_ACKED_METRIC,
		move |acked| acked > baseline.removed_acked,
		HOP_METRIC_TIMEOUT_SECS,
		"ack did not increment _pool_removed_total{reason=\"acked\"}",
	)
	.await?;
	test_log!(TEST, "✓ Blob A claimed, acked and removed from the pool");

	// ── Blob B: promotion path ──
	test_log!(TEST, "--- Blob B: promotion ---");
	let (data_b, hash_b) = blob("B");
	let hash_hex = hex::encode(hash_b);
	tracing::info!(
		"blob B: {} bytes, content_hash={}, CID={}",
		data_b.len(),
		hash_hex,
		hash_to_cid(&hash_b),
	);

	// Bitswap probe BEFORE promotion: blob lives only in the HOP pool, col11 has no entry.
	let before_match = verify_bitswap_fetch(&multiaddr, &data_b, BITSWAP_TIMEOUT_SECS)
		.await
		.unwrap_or(false);
	tracing::info!("bitswap BEFORE promotion: match={}", before_match);

	// `hop_submit` -> maintenance task promotes -> `TransactionStorage::Stored` on-chain.
	let entry_count = env.submit(&data_b).await?;
	tracing::info!("hop_submit B OK; pool entry_count={}", entry_count);
	let store_block = wait_for_promoted(&env.client, &hash_b, PROMOTION_TIMEOUT_SECS).await?;
	tracing::info!("✓ HOP promotion landed at block {}", store_block);

	// Bitswap probe AFTER promotion: blob is now on-chain; bitswap *should* match.
	let after_match = verify_bitswap_fetch(&multiaddr, &data_b, BITSWAP_TIMEOUT_SECS)
		.await
		.unwrap_or(false);
	tracing::info!("bitswap AFTER promotion: match={}", after_match);

	// Tie `ProofChecked` to our blob: the inherent at N proves `Transactions[N - RP]`, so
	// confirm the content hash is still indexed at `store_block` when the proof fires.
	let proof_block = store_block + RETENTION_PERIOD as u64;
	let proof_hash = finalized_block_hash_at(&env.client, proof_block).await?;
	let indexed_at = canonical_store_block(&env.client, proof_hash, &hash_b).await?;
	assert_eq!(
		indexed_at, store_block,
		"HOP blob indexed at {} but proof at {} reads Transactions[{}]",
		indexed_at, proof_block, store_block,
	);
	assert_proof_checked_at(&env.client, proof_block, "HOP-promoted blob").await?;
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

	// ── Runtime API: promotion is visible, and the extrinsic builder is well-formed ──
	assert!(
		hop_api::is_promoted_on_chain(&env.client, &hash_b).await?,
		"is_promoted_on_chain is false for a blob promoted at block {}",
		store_block,
	);
	assert!(
		!hop_api::is_promoted_on_chain(&env.client, &[0xAB; 32]).await?,
		"is_promoted_on_chain is true for an unrelated hash",
	);

	let extrinsic =
		hop_api::create_promotion_extrinsic(&env.client, &env.alice, &data_b, now_ms()).await?;
	assert!(
		extrinsic.len() > data_b.len(),
		"create_promotion_extrinsic returned {} bytes for a {}-byte blob",
		extrinsic.len(),
		data_b.len(),
	);
	// `data` must stay the last call argument: `do_store` indexes the trailing
	// `data.len()` bytes of the encoded extrinsic under the blob's content hash.
	assert!(
		extrinsic.ends_with(&data_b),
		"create_promotion_extrinsic does not end with the blob — `data` is no longer the \
		 last call argument, which corrupts the indexed bytes",
	);
	test_log!(TEST, "✓ HopRuntimeApi: promotion visible, extrinsic ends with the blob");

	// ── Promotion + expiry metrics ──
	wait_hop_metric(
		&collator1,
		HOP_PROMOTIONS_CONFIRMED_METRIC,
		move |confirmed| confirmed > baseline.promotions_confirmed,
		HOP_METRIC_TIMEOUT_SECS,
		"promotion was not counted",
	)
	.await?;
	wait_hop_metric(
		&collator1,
		HOP_PROMOTION_BACKLOG_METRIC,
		|backlog| backlog == 0,
		HOP_METRIC_TIMEOUT_SECS,
		"promotion backlog did not drain",
	)
	.await?;
	// Retention is `HOP_RETENTION_SECS`; the proof wait above already spent longer than
	// that, so the promoted entry ages out inside the test window.
	wait_hop_metric(
		&collator1,
		HOP_REMOVED_EXPIRED_PROMOTED_METRIC,
		move |expired| expired > baseline.removed_expired_promoted,
		HOP_METRIC_TIMEOUT_SECS,
		"promoted entry did not age out of the pool",
	)
	.await?;

	// The pool is empty once A is acked and B has promoted and expired.
	for (metric, what) in [(HOP_POOL_ENTRIES_METRIC, "entries"), (HOP_POOL_BYTES_METRIC, "bytes")] {
		wait_hop_metric(
			&collator1,
			metric,
			|v| v == 0,
			HOP_METRIC_TIMEOUT_SECS,
			&format!("pool {what} did not return to zero"),
		)
		.await?;
	}

	// Cross-check the pool's own view against the gauges just asserted.
	let status = hop_pool_status(&env.rpc).await?;
	assert_eq!(
		(status.entry_count, status.total_bytes, status.max_bytes),
		(0, 0, max_bytes),
		"hop_poolStatus disagrees with the pool gauges",
	);

	let final_counters = HopCounters::read(&collator1).await?;
	tracing::info!("final HOP counters: {:?}", final_counters);

	// Direct blob-loss assertion: nothing may leave the pool unpromoted. `sc-hop`
	// documents this series as an upper bound on loss, so compare against zero rather
	// than an expected count. Blob C never entered the pool, so it cannot appear here;
	// the counter is exercised from the other side by
	// `parachain_hop_unpromoted_expiry_test`.
	assert_eq!(
		final_counters.removed_expired_unpromoted, 0,
		"{} entries expired unpromoted — HOP dropped data",
		final_counters.removed_expired_unpromoted,
	);
	// Two blobs reached the pool (A and B); C was rejected before insertion. The counter
	// tracks accounted size, which includes per-recipient overhead, hence `>=`.
	let inserted = final_counters.inserted_bytes - baseline.inserted_bytes;
	let expected = (data_a.len() + data_b.len()) as u64;
	assert!(
		inserted >= expected,
		"pool recorded {} inserted bytes for two {}-byte blobs",
		inserted,
		TEST_DATA_SIZE,
	);

	test_log!(TEST, "=== HOP promotion bitswap test PASSED ===");
	env.network.destroy().await?;
	Ok(())
}

/// A pooled blob whose account authorization lapses before the promotion window opens is
/// dropped, not stored: `_pool_removed_total{reason="expired_unpromoted"}` is the only
/// signal for that loss.
///
/// Promotion never re-checks authorization itself — `sc-hop` builds the extrinsic and
/// hands it to `submit_local`, where the pool's `authorize_promote` hook rejects it with
/// `BadSigner`. `mark_promoted` is reached only once `is_promoted_on_chain` confirms the
/// extrinsic landed, so a rejected submission leaves the entry unpromoted and
/// `_promotions_confirmed_total` untouched.
///
/// Needs its own network: `buffer < retention` keeps the fresh entry out of the promotion
/// window while the authorization is expired, and that inverts what
/// `parachain_hop_promotion_bitswap_test` requires.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_hop_unpromoted_expiry_test() -> Result<()> {
	const TEST: &str = "para_hop_unpromoted_expiry";
	crate::utils::init_logging();

	test_log!(
		TEST,
		"=== HOP unpromoted expiry (hop_retention={}s, hop_buffer={}s) ===",
		UNPROMOTED_HOP_RETENTION_SECS,
		UNPROMOTED_HOP_PROMOTION_BUFFER_SECS,
	);

	let mut env = HopEnv::spawn(hop_node_args(
		UNPROMOTED_HOP_RETENTION_SECS,
		UNPROMOTED_HOP_PROMOTION_BUFFER_SECS,
	))
	.await?;
	let collator1 = env.collator1.clone();

	env.authorize_alice().await?;
	let baseline = HopCounters::read(&collator1).await?;

	// Submit while the authorization is still valid, so the blob reaches the pool.
	let (data_d, hash_d) = blob("D");
	let entry_count = env.submit(&data_d).await?;
	tracing::info!("hop_submit D OK; pool entry_count={}", entry_count);
	assert!(entry_count >= 1, "pool reported {} entries right after submit", entry_count);

	// The only place the pool gauges can be observed non-zero: promotion attempts do not
	// start until `retention - buffer` seconds after submit, so the entry sits here. The
	// promotion test's 10s retention is too short to read them before they drain.
	assert!(
		hop_metric(&collator1, HOP_POOL_ENTRIES_METRIC).await? >= 1,
		"substrate_hop_pool_entries is 0 while an entry is pooled",
	);
	let pooled_bytes = hop_metric(&collator1, HOP_POOL_BYTES_METRIC).await?;
	assert!(
		pooled_bytes >= data_d.len() as u64,
		"substrate_hop_pool_bytes is {} for a pooled {}-byte blob",
		pooled_bytes,
		data_d.len(),
	);

	// Expire the authorization out from under the pooled entry. `authorize_account` can
	// neither shrink an entry nor set a custom expiration, hence the storage override.
	// This is finalized well before the promotion window opens at
	// `retention - buffer` seconds after submit.
	let nonce = env.take_nonce();
	override_alice_authorization(
		&env.client,
		AuthorizationOverride {
			transactions: 0,
			transactions_allowance: AUTH_TRANSACTIONS,
			bytes: 0,
			bytes_permanent: 0,
			bytes_allowance: AUTH_BYTES,
			expiration: 1,
		},
		nonce,
	)
	.await?;
	// The override is only best-block included, but runtime API calls read the latest
	// *finalized* block, so the expiry is invisible until finality catches up.
	wait_for_finalized_quiescence(&collator1, FINALIZED_TRANSACTION_TIMEOUT_SECS).await?;
	assert!(
		!hop_api::can_account_promote(&env.client, &env.alice_id, TEST_DATA_SIZE as u32).await?,
		"can_account_promote is still true after expiring the authorization",
	);
	test_log!(TEST, "✓ Blob D pooled, then authorization expired");

	// `in_promotion_window` counts entries that are unpromoted, within `buffer` of expiry
	// and still under `MAX_PROMOTION_ATTEMPTS`. D qualifies from `retention - buffer`
	// seconds after submit until it expires, which is the only window in either test where
	// the backlog is observable above zero.
	wait_hop_metric(
		&collator1,
		HOP_PROMOTION_BACKLOG_METRIC,
		|backlog| backlog >= 1,
		UNPROMOTED_EXPIRY_TIMEOUT_SECS,
		"promotion backlog never counted the pooled entry",
	)
	.await?;
	test_log!(TEST, "✓ Blob D counted in _promotion_backlog");

	// Every promotion attempt now fails in the pool, so the entry ages out unpromoted.
	wait_hop_metric(
		&collator1,
		HOP_REMOVED_EXPIRED_UNPROMOTED_METRIC,
		move |unpromoted| unpromoted > baseline.removed_expired_unpromoted,
		UNPROMOTED_EXPIRY_TIMEOUT_SECS,
		"pooled entry did not expire unpromoted",
	)
	.await?;

	let final_counters = HopCounters::read(&collator1).await?;
	tracing::info!("final HOP counters: {:?}", final_counters);

	assert_eq!(
		final_counters.promotions_confirmed, baseline.promotions_confirmed,
		"promotion was confirmed for a blob whose authorization had expired",
	);
	assert_eq!(
		final_counters.removed_expired_promoted, baseline.removed_expired_promoted,
		"entry was counted as expiring promoted",
	);
	assert!(
		!hop_api::is_promoted_on_chain(&env.client, &hash_d).await?,
		"is_promoted_on_chain is true for a blob that was never promoted",
	);
	for (metric, what) in [
		(HOP_POOL_ENTRIES_METRIC, "pool entries"),
		(HOP_PROMOTION_BACKLOG_METRIC, "promotion backlog"),
	] {
		wait_hop_metric(
			&collator1,
			metric,
			|v| v == 0,
			HOP_METRIC_TIMEOUT_SECS,
			&format!("{what} did not return to zero"),
		)
		.await?;
	}

	test_log!(TEST, "=== HOP unpromoted expiry test PASSED ===");
	env.network.destroy().await?;
	Ok(())
}
