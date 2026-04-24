use anyhow::{anyhow, Result};
use blake2::digest::{consts::U32, Digest};
use std::{
	collections::VecDeque,
	sync::{
		atomic::{AtomicBool, AtomicU64, Ordering},
		Arc, Mutex,
	},
	time::{Duration, Instant},
};
use subxt::{
	blocks::{Block, ExtrinsicEvents},
	dynamic::{tx, Value},
	tx::TxStatus,
	utils::H256,
	OnlineClient,
};
use subxt_signer::sr25519::Keypair;
use tokio::sync::Notify;

use crate::{
	accounts::NonceTracker,
	client::{BulletinConfig, BulletinExtrinsicParamsBuilder},
	report::BlockStats,
};

/// Receiver end of a block forwarding channel.
///
/// A background task actively drains the WebSocket subscription and forwards
/// blocks through this channel, so no blocks are lost even if the receiver
/// is not polled immediately.
pub type BlockReceiver =
	tokio::sync::mpsc::UnboundedReceiver<Block<BulletinConfig, OnlineClient<BulletinConfig>>>;

/// Create a monitor connection, subscribe to best blocks, and immediately
/// start consuming the subscription in a background task.
///
/// Returns a channel receiver that delivers blocks as they arrive. Because
/// the subscription is consumed eagerly, no blocks are dropped even if the
/// receiver is polled much later (e.g. after authorization finalization).
pub async fn subscribe_blocks(ws_url: &str) -> Result<BlockReceiver> {
	let monitor_client = crate::client::connect(ws_url).await?;
	let mut sub = monitor_client.blocks().subscribe_best().await?;
	let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
	tokio::spawn(async move {
		// Keep `monitor_client` alive so the subscription stays connected.
		let _client = monitor_client;
		while let Some(Ok(block)) = sub.next().await {
			if tx.send(block).is_err() {
				break; // receiver dropped
			}
		}
	});
	Ok(rx)
}

/// Receiver for finalized block numbers.
pub type FinalizedBlockReceiver = tokio::sync::mpsc::UnboundedReceiver<u64>;

/// Dual best + finalized block subscription.
pub struct DualBlockSubscription {
	pub(crate) best_rx: BlockReceiver,
	pub(crate) finalized_rx: FinalizedBlockReceiver,
	pub(crate) monitor_client: OnlineClient<BulletinConfig>,
	pub(crate) ws_url: String,
}

/// Subscribe to both best and finalized blocks, eagerly draining each in a
/// background task that automatically reconnects on failure.
/// The `monitor_client` is kept alive for storage queries.
pub async fn subscribe_blocks_dual(ws_url: &str) -> Result<DualBlockSubscription> {
	let client = crate::client::connect(ws_url).await?;

	// Best blocks — reconnects on subscription failure.
	let (best_tx, best_rx) = tokio::sync::mpsc::unbounded_channel();
	{
		let url = ws_url.to_string();
		let mut client = client.clone();
		let best_tx = best_tx.clone();
		tokio::spawn(async move {
			loop {
				match client.blocks().subscribe_best().await {
					Ok(mut sub) => {
						while let Some(Ok(block)) = sub.next().await {
							if best_tx.send(block).is_err() {
								return;
							}
						}
						log::warn!("monitor: best block subscription ended, reconnecting");
					},
					Err(e) => {
						log::warn!("monitor: best block subscribe failed: {e}, retrying in 2s");
					},
				}
				tokio::time::sleep(std::time::Duration::from_secs(2)).await;
				match crate::client::connect(&url).await {
					Ok(new_client) => client = new_client,
					Err(e) => log::warn!("monitor: reconnect failed: {e}, retrying"),
				}
			}
		});
	}

	// Finalized blocks — reconnects on subscription failure.
	let (fin_tx, fin_rx) = tokio::sync::mpsc::unbounded_channel();
	{
		let url = ws_url.to_string();
		let mut client = client.clone();
		tokio::spawn(async move {
			loop {
				match client.blocks().subscribe_finalized().await {
					Ok(mut sub) => {
						while let Some(Ok(block)) = sub.next().await {
							if fin_tx.send(block.number() as u64).is_err() {
								return;
							}
						}
						log::warn!("monitor: finalized subscription ended, reconnecting");
					},
					Err(e) => {
						log::warn!("monitor: finalized subscribe failed: {e}, retrying in 2s");
					},
				}
				tokio::time::sleep(std::time::Duration::from_secs(2)).await;
				match crate::client::connect(&url).await {
					Ok(new_client) => client = new_client,
					Err(e) => log::warn!("monitor: reconnect failed: {e}, retrying"),
				}
			}
		});
	}

	Ok(DualBlockSubscription {
		best_rx,
		finalized_rx: fin_rx,
		monitor_client: client,
		ws_url: ws_url.to_string(),
	})
}

/// Read `pallet_timestamp::Now` at a specific block hash.
/// Returns milliseconds since Unix epoch.
pub(crate) async fn read_timestamp_at(
	client: &OnlineClient<BulletinConfig>,
	block_hash: H256,
) -> Result<u64> {
	let addr = subxt::dynamic::storage("Timestamp", "Now", vec![]);
	let value = client
		.storage()
		.at(block_hash)
		.fetch(&addr)
		.await?
		.ok_or_else(|| anyhow!("Timestamp::Now not found at block {block_hash:?}"))?;
	let decoded = value.to_value()?;
	crate::chain_info::value_to_u64(&decoded.value)
		.ok_or_else(|| anyhow!("Cannot decode Timestamp::Now as u64"))
}

/// Internal tracking entry for a best block awaiting finalization confirmation.
pub(crate) struct PendingBlock {
	pub(crate) number: u64,
	pub(crate) hash: H256,
	pub(crate) tx_count: u64,
	pub(crate) payload_bytes: u64,
	pub(crate) timestamp_ms: Option<u64>,
}

const TX_TIMEOUT_SECS: u64 = 60;

/// Classified transaction pool / RPC error.
///
/// Error codes from polkadot-sdk `substrate/client/rpc-api/src/author/error.rs`:
///   1010 = POOL_INVALID_TX (stale nonce, bad sig, exhausts resources, etc.)
///   1011 = POOL_UNKNOWN_VALIDITY (cannot lookup state)
///   1012 = POOL_TEMPORARILY_BANNED
///   1013 = POOL_ALREADY_IMPORTED (duplicate tx)
///   1014 = POOL_TOO_LOW_PRIORITY
///   1016 = POOL_IMMEDIATELY_DROPPED (pool full)
///   1020 = POOL_INVALID_BLOCK_ID
///   1021 = POOL_FUTURE_TX (pool not accepting future nonces)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TxPoolError {
	/// Pool is full (1016) or priority too low (1014) — tx never entered pool,
	/// safe to rollback nonce and retry. On feeless chains all txs have equal
	/// priority, so 1014 fires instead of 1016 when the pool can't evict.
	PoolFull,
	/// Tx entered pool but was later evicted ("Transaction dropped") — nonce may
	/// have been consumed on-chain, NOT safe to re-queue blindly.
	TxDropped,
	/// Already imported (1013) — duplicate, skip.
	AlreadyImported,
	/// Stale nonce (1010 + "stale") — account already used, skip.
	StaleNonce,
	/// Future nonce (1010 + "future") or 1021 — should not happen with pre-init.
	FutureNonce,
	/// Temporarily banned (1012) — retriable after a block.
	Banned,
	/// Exhausts block resources (1010 + "exhaust") — payload too large for block.
	ExhaustsResources,
	/// WebSocket connection is dead — needs reconnect before retrying.
	ConnectionDead,
	/// Any other error — not retriable.
	Other,
}

pub(crate) fn classify_tx_error(e: &anyhow::Error) -> TxPoolError {
	let msg = format!("{e}").to_lowercase();

	// 1016: tx never entered pool (safe to rollback nonce and retry)
	// 1014: priority too low — on feeless chains all txs have equal priority,
	// so the pool can't evict any existing tx; same semantics as PoolFull.
	if msg.contains("1016") ||
		msg.contains("immediately dropped") ||
		msg.contains("1014") ||
		msg.contains("priority is too low")
	{
		return TxPoolError::PoolFull;
	}
	// Tx entered pool but was evicted (nonce may have been consumed on-chain)
	if msg.contains("transaction dropped") || msg.contains("was dropped") {
		return TxPoolError::TxDropped;
	}
	if msg.contains("1013") || msg.contains("already imported") {
		return TxPoolError::AlreadyImported;
	}
	if msg.contains("1012") || msg.contains("temporarily banned") {
		return TxPoolError::Banned;
	}

	// Nonce errors
	if msg.contains("stale") || (msg.contains("1010") && msg.contains("outdated")) {
		return TxPoolError::StaleNonce;
	}
	if msg.contains("1021") ||
		(msg.contains("1010") && msg.contains("future")) ||
		msg.contains("will be valid in the future")
	{
		return TxPoolError::FutureNonce;
	}

	// Resource limits
	if msg.contains("exhaust") {
		return TxPoolError::ExhaustsResources;
	}

	// Connection-level errors — WebSocket died or state pruned, needs reconnect.
	if msg.contains("connection reset") ||
		msg.contains("background task closed") ||
		msg.contains("connection closed") ||
		msg.contains("broken pipe") ||
		msg.contains("restart required") ||
		msg.contains("not connected") ||
		msg.contains("i/o error") ||
		msg.contains("state already discarded")
	{
		return TxPoolError::ConnectionDead;
	}

	// Generic invalid tx (1010) — could be payment, bad proof, etc.
	if msg.contains("1010") || msg.contains("invalid transaction") {
		return TxPoolError::StaleNonce; // most common 1010 cause in our context
	}

	TxPoolError::Other
}

/// Wait for a transaction to be included in a best block.
pub async fn wait_for_in_best_block(
	mut progress: subxt::tx::TxProgress<BulletinConfig, OnlineClient<BulletinConfig>>,
) -> Result<(H256, ExtrinsicEvents<BulletinConfig>)> {
	while let Some(status) = progress.next().await {
		match status? {
			TxStatus::InBestBlock(tx_in_block) => {
				let block_hash = tx_in_block.block_hash();
				let events = tx_in_block.wait_for_success().await?;
				return Ok((block_hash, events));
			},
			TxStatus::Error { message } |
			TxStatus::Invalid { message } |
			TxStatus::Dropped { message } => {
				anyhow::bail!("Transaction failed: {message}");
			},
			_ => continue,
		}
	}
	anyhow::bail!("Transaction stream ended without InBestBlock status")
}

/// Fire-and-forget store (highest throughput).
///
/// Returns the extrinsic hash computed the same way as `block.extrinsics()` —
/// blake2-256 of the raw encoded bytes (no SCALE length prefix). This allows
/// hash-based matching of submitted txs against block contents.
pub async fn store_fire_and_forget(
	client: &OnlineClient<BulletinConfig>,
	signer: &Keypair,
	nonce_tracker: &NonceTracker,
	data: &[u8],
) -> Result<H256> {
	use subxt::config::Hasher;

	let account_id = signer.public_key().to_account_id();
	let nonce = nonce_tracker.next_nonce(&account_id);
	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
	let params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).build();

	let signed = client.tx().create_signed(&store_call, signer, params).await?;
	// Hash the raw encoded bytes (no SCALE length prefix) — same as
	// block.extrinsics().iter().hash() does — so we can match hashes.
	let raw_hash = client.hasher().hash(signed.encoded());
	signed.submit().await?;
	Ok(raw_hash)
}

/// Build a signed `TransactionStorage::store` extrinsic bytes (use nonce `0` for one-shot accounts
/// after [`crate::accounts::batch_init_nonces`]).
pub async fn sign_store_extrinsic(
	client: &OnlineClient<BulletinConfig>,
	signer: &Keypair,
	data: &[u8],
	nonce: u64,
) -> Result<Vec<u8>> {
	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
	let params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).build();
	let signed = client.tx().create_signed(&store_call, signer, params).await?;
	Ok(signed.into_encoded())
}

/// Synchronous version of [`sign_store_extrinsic`] for use inside
/// [`tokio::task::spawn_blocking`]. Uses `create_partial_offline` + `sign` (both sync) instead of
/// the async `create_signed`.
pub fn sign_store_extrinsic_blocking(
	client: &OnlineClient<BulletinConfig>,
	signer: &Keypair,
	data: &[u8],
	nonce: u64,
) -> Result<Vec<u8>> {
	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
	let params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).build();
	let mut partial = client
		.tx()
		.create_partial_offline(&store_call, params)
		.map_err(|e| anyhow!("create_partial_offline: {e}"))?;
	let signed = partial.sign(signer);
	Ok(signed.into_encoded())
}

/// Submit a pre-signed store extrinsic (see [`sign_store_extrinsic`]). Same hash semantics as
/// [`store_fire_and_forget`].
/// Fire-and-forget submit using `author_submitExtrinsic` (no subscription).
///
/// This is much faster than subxt's `submit()` which uses `author_submitAndWatchExtrinsic`
/// and holds one subscription per tx waiting for a status event.
pub async fn store_submit_pre_signed(
	rpc_client: &jsonrpsee::ws_client::WsClient,
	encoded: &[u8],
) -> Result<H256> {
	use jsonrpsee::core::client::ClientT;

	let raw_hash = H256::from(crate::client::blake2b_256(encoded));

	let hex = format!("0x{}", hex::encode(encoded));
	let _hash: String = rpc_client
		.request("author_submitExtrinsic", jsonrpsee::rpc_params![hex])
		.await
		.map_err(|e| anyhow!("author_submitExtrinsic: {e}"))?;

	Ok(raw_hash)
}

/// Generate random (incompressible) test payload of given size.
///
/// Uses random bytes so that PoV compression cannot artificially shrink the data,
/// giving realistic results when testing against parachain PoV size limits.
pub fn generate_payload(size: usize) -> Vec<u8> {
	use rand::RngCore;
	let mut data = vec![0u8; size];
	rand::thread_rng().fill_bytes(&mut data);
	data
}

/// Generate a unique payload for a given `(size, index)` pair.
///
/// Produces a random base and XORs the index bytes at the start, so each index
/// yields a distinct CID while remaining incompressible.
pub fn generate_indexed_payload(size: usize, index: u32) -> Vec<u8> {
	let mut data = generate_payload(size);
	let idx_bytes = index.to_le_bytes();
	for (i, &b) in idx_bytes.iter().enumerate().take(data.len()) {
		data[i] ^= b;
	}
	data
}

/// Compute the CIDv1 for raw data using Blake2b-256, matching the chain's
/// default CID configuration (Raw codec 0x55, Blake2b-256 multihash 0xb220).
pub fn compute_cid_blake2b256(data: &[u8]) -> Result<cid::Cid> {
	let digest: [u8; 32] = blake2::Blake2b::<U32>::digest(data).into();
	let mh = cid::multihash::Multihash::<64>::wrap(0xb220, &digest)
		.map_err(|e| anyhow!("multihash wrap failed: {e}"))?;
	Ok(cid::Cid::new_v1(0x55, mh))
}

/// Count `TransactionStorage::Stored` events in a block. Uses events (lightweight)
/// instead of extrinsics (full block body) to avoid RPC response size limits.
/// Count of store transactions and their total encoded extrinsic bytes in a block.
/// Extract content hashes from `TransactionStorage::Stored` events in a block.
///
/// The `Stored` event is `{ index: u32, content_hash: [u8; 32], cid: Option<Cid> }`.
/// In SCALE encoding the content_hash occupies bytes 4..36 of `field_bytes()`.
pub(crate) async fn stored_content_hashes(
	block: &Block<BulletinConfig, OnlineClient<BulletinConfig>>,
) -> Vec<[u8; 32]> {
	let block_number = block.number();
	let mut hashes = Vec::new();
	match block.events().await {
		Ok(events) =>
			for ev in events.iter().flatten() {
				if ev.pallet_name() == "TransactionStorage" && ev.variant_name() == "Stored" {
					let fb = ev.field_bytes();
					// index: u32 (4 bytes) then content_hash: [u8; 32]
					if fb.len() >= 36 {
						let mut h = [0u8; 32];
						h.copy_from_slice(&fb[4..36]);
						hashes.push(h);
					} else {
						log::warn!(
							"block #{block_number}: Stored event field_bytes too short ({})",
							fb.len()
						);
					}
				}
			},
		Err(e) => {
			log::warn!("block #{block_number}: failed to fetch events: {e}");
		},
	}
	hashes
}

/// Result of a bulk store operation.
pub struct BulkStoreResult {
	pub total_submitted: u64,
	pub total_confirmed: u64,
	pub total_errors: u64,
	pub stale_nonces: u64,
	pub pool_full_retries: u64,
	pub remaining_in_queue: u64,
	pub nonces_initialized: u64,
	pub nonces_failed: u64,
	/// Duration of the measurement window (after pool saturation, if applicable).
	pub duration: Duration,
	/// All blocks observed (each annotated with `prefill` flag).
	pub blocks: Vec<BlockStats>,
	/// Number of fork replacements / fork victims detected.
	pub fork_detections: u64,
	/// Pipeline stalled (no blocks with txs for extended period). Caller should retry.
	pub stalled: bool,
	/// Per-tx inclusion latencies (submission → seen in block).
	pub tx_latencies_ms: Vec<f64>,
}

/// Concurrently store data using one-shot accounts (each account submits 1 tx
/// at nonce 0). Uses the same concurrent submitter + backpressure pattern as
/// the throughput tests.
///
/// Each `(Keypair, Arc<Vec<u8>>)` in `work_items` is one store operation.
/// For identical data across all items, share a single `Arc<Vec<u8>>`.
///
/// If `stop_after_blocks` is `Some(n)`, stops submitters after `n` blocks
/// containing our txs have been observed (for throughput measurement).
/// If `None`, waits until all submitted txs are confirmed in blocks.
pub async fn bulk_store_oneshot(
	work_items: Vec<(Keypair, Arc<Vec<u8>>)>,
	ws_urls: &[&str],
	stop_after_blocks: Option<u32>,
	submitters: usize,
	block_input: BlockReceiver,
) -> Result<BulkStoreResult> {
	if work_items.is_empty() {
		return Ok(BulkStoreResult {
			total_submitted: 0,
			total_confirmed: 0,
			total_errors: 0,
			stale_nonces: 0,
			pool_full_retries: 0,
			remaining_in_queue: 0,
			nonces_initialized: 0,
			nonces_failed: 0,
			duration: Duration::ZERO,
			blocks: vec![],
			fork_detections: 0,
			stalled: false,
			tx_latencies_ms: vec![],
		});
	}

	let total_items = work_items.len();
	let num_submitters = submitters.min(total_items).max(1);
	let num_connections = num_submitters.max(8).min(total_items).max(1);
	log::info!(
		"bulk_store: {total_items} items, {num_submitters} submitters, {num_connections} connections"
	);

	let mut pool = Vec::with_capacity(num_connections);
	for i in 0..num_connections {
		let url = ws_urls[i % ws_urls.len()];
		pool.push(Arc::new(crate::client::connect(url).await?));
	}

	// Pre-init all account nonces from chain before starting the clock.
	// This avoids per-account RPC queries in the hot loop.
	let nonce_tracker = NonceTracker::new();
	let keypairs: Vec<_> = work_items.iter().map(|(kp, _)| kp.clone()).collect();
	log::info!("bulk_store: pre-initializing nonces for {} accounts...", keypairs.len());
	let (nonce_ok, nonce_fail) =
		crate::accounts::batch_init_nonces(&pool, &nonce_tracker, &keypairs, num_connections * 4)
			.await;
	log::info!("bulk_store: nonces initialized: {nonce_ok} ok, {nonce_fail} failed");
	drop(keypairs);

	let account_queue = Arc::new(Mutex::new(VecDeque::from(work_items)));
	let submitted = Arc::new(AtomicU64::new(0));
	let errors = Arc::new(AtomicU64::new(0));
	let pool_full_retries = Arc::new(AtomicU64::new(0));
	let stale_nonces = Arc::new(AtomicU64::new(0));
	let stop = Arc::new(AtomicBool::new(false));
	let new_block_notify = Arc::new(Notify::new());

	// Pool saturation flag: when stop_after_blocks is set, submitters signal
	// the first PoolFull error so the monitor knows the pool is fully loaded.
	// Measurement (block counting + timer) only starts after saturation.
	let pool_saturated = Arc::new(AtomicBool::new(stop_after_blocks.is_none()));

	let start = Instant::now();

	// --- Spawn monitor FIRST so it is actively consuming blocks before any
	// --- submissions happen. This guarantees we never miss a block.
	let block_stats = Arc::new(Mutex::new(Vec::<BlockStats>::new()));
	let block_stats_monitor = block_stats.clone();
	let stop_monitor = stop.clone();
	let pool_saturated_monitor = pool_saturated.clone();
	let new_block_notify_monitor = new_block_notify.clone();
	let measure_start = Arc::new(Mutex::new(None::<Instant>));
	let measure_start_monitor = measure_start.clone();
	let monitor_ready = Arc::new(Notify::new());
	let monitor_ready_signal = monitor_ready.clone();
	let fork_detections = Arc::new(AtomicU64::new(0));

	let mut blocks_rx = block_input;
	let monitor_handle = tokio::spawn(async move {
		let mut total_store_blocks = 0u32;
		monitor_ready_signal.notify_one();

		while !stop_monitor.load(Ordering::Relaxed) {
			if let Some(block) = blocks_rx.recv().await {
				let block_number = block.number() as u64;
				new_block_notify_monitor.notify_waiters();

				let store_tx_count = stored_content_hashes(&block).await.len() as u64;
				let is_prefill = !pool_saturated_monitor.load(Ordering::Relaxed);

				if is_prefill && store_tx_count == 0 {
					continue;
				}

				let phase = if is_prefill { "[pre-fill]" } else { "[measured]" };

				if !is_prefill {
					let mut ms = measure_start_monitor.lock().unwrap();
					if ms.is_none() {
						log::info!(
							"bulk_store: measurement clock starts at block \
							 #{block_number}"
						);
						*ms = Some(Instant::now());
					}
				}

				if store_tx_count > 0 || !is_prefill {
					log::info!(
						"bulk_store: {phase} block #{block_number}: \
						 {store_tx_count} store txs"
					);
				}

				block_stats_monitor.lock().unwrap().push(BlockStats {
					number: block_number,
					tx_count: store_tx_count,
					payload_bytes: 0,
					prefill: is_prefill,
					timestamp_ms: None,
					hash: None,
					finalized: false,
					interval_ms: None,
				});

				if !is_prefill && store_tx_count > 0 {
					total_store_blocks += 1;
					if let Some(limit) = stop_after_blocks {
						if total_store_blocks >= limit {
							log::info!(
								"bulk_store: reached {total_store_blocks} measured \
								 blocks with txs (target {limit}), stopping"
							);
							stop_monitor.store(true, Ordering::Relaxed);
							new_block_notify_monitor.notify_waiters();
						}
					}
				}
			}
		}
	});

	// Wait for the monitor to be actively consuming blocks before submitting.
	monitor_ready.notified().await;
	log::info!("bulk_store: block monitor ready, starting submitters");

	// Spawn concurrent submitter tasks
	let mut handles = Vec::new();
	let ws_urls_owned: Vec<String> = ws_urls.iter().map(|s| s.to_string()).collect();
	for task_id in 0..num_submitters {
		let account_queue = account_queue.clone();
		let submitted = submitted.clone();
		let errors = errors.clone();
		let pool_full_retries = pool_full_retries.clone();
		let stale_nonces = stale_nonces.clone();
		let stop = stop.clone();
		let pool_saturated = pool_saturated.clone();
		let new_block_notify = new_block_notify.clone();
		let mut worker_client = pool[task_id % num_connections].clone();
		let reconnect_url = ws_urls_owned[task_id % ws_urls_owned.len()].clone();
		let nonce_tracker = nonce_tracker.clone();
		let has_block_target = stop_after_blocks.is_some();

		handles.push(tokio::spawn(async move {
			let mut empty_polls = 0u32;
			let mut consecutive_conn_errors = 0u32;
			loop {
				if stop.load(Ordering::Relaxed) {
					break;
				}

				let item = { account_queue.lock().unwrap().pop_front() };
				let Some((signer, data)) = item else {
					if has_block_target && !stop.load(Ordering::Relaxed) {
						empty_polls += 1;
						// Wait for re-queued items from pool-full retries.
						// Exit after ~5s of empty queue (all items accepted).
						if empty_polls > 25 {
							break;
						}
						tokio::time::timeout(
							Duration::from_millis(200),
							new_block_notify.notified(),
						)
						.await
						.ok();
						continue;
					}
					break;
				};
				empty_polls = 0;

				// Race the submit against the stop signal so we don't block
				// for minutes on a slow RPC call after the test is done.
				let submit_result = tokio::select! {
					r = store_fire_and_forget(
						&worker_client, &signer, &nonce_tracker, &data,
					) => r,
					() = async {
						while !stop.load(Ordering::Relaxed) {
							tokio::time::sleep(Duration::from_millis(100)).await;
						}
					} => {
						break;
					}
				};

				match submit_result {
					Ok(_hash) => {
						submitted.fetch_add(1, Ordering::Relaxed);
						consecutive_conn_errors = 0;
					},
					Err(e) => {
						// Don't re-queue after stop — just exit.
						if stop.load(Ordering::Relaxed) {
							break;
						}
						let account_id = signer.public_key().to_account_id();
						match classify_tx_error(&e) {
							TxPoolError::PoolFull => {
								pool_full_retries.fetch_add(1, Ordering::Relaxed);
								consecutive_conn_errors = 0;
								if !pool_saturated.swap(true, Ordering::Relaxed) {
									log::info!(
										"bulk_store submitter {task_id}: pool saturated \
										 (first PoolFull) — measurement starts now"
									);
								}
								log::debug!(
									"bulk_store submitter {task_id}: pool full (1016), \
									 rollback nonce & re-queuing: {e}"
								);
								// Tx never entered pool → nonce not consumed, safe to
								// rollback and retry immediately.
								nonce_tracker.rollback(&account_id);
								account_queue.lock().unwrap().push_front((signer, data));
								// Brief yield to avoid busy-spin, then retry.
								tokio::time::sleep(Duration::from_millis(100)).await;
							},
							TxPoolError::Banned | TxPoolError::ExhaustsResources => {
								pool_full_retries.fetch_add(1, Ordering::Relaxed);
								consecutive_conn_errors = 0;
								log::warn!(
									"bulk_store submitter {task_id}: banned/exhausts, \
									 rollback nonce & re-queuing: {e}"
								);
								nonce_tracker.rollback(&account_id);
								account_queue.lock().unwrap().push_front((signer, data));
								tokio::time::timeout(
									Duration::from_secs(12),
									new_block_notify.notified(),
								)
								.await
								.ok();
							},
							TxPoolError::ConnectionDead => {
								consecutive_conn_errors += 1;
								// Re-queue the item — nonce was never sent.
								nonce_tracker.rollback(&account_id);
								account_queue.lock().unwrap().push_front((signer, data));

								if consecutive_conn_errors == 1 {
									log::warn!(
										"bulk_store submitter {task_id}: connection dead, \
										 attempting reconnect to {reconnect_url}"
									);
								}

								// Exponential backoff: 1s, 2s, 4s, ... up to 30s
								let backoff = Duration::from_secs(
									(1u64 << consecutive_conn_errors.min(5)).min(30),
								);
								tokio::time::sleep(backoff).await;

								match crate::client::connect(&reconnect_url).await {
									Ok(new_client) => {
										worker_client = Arc::new(new_client);
										log::info!(
											"bulk_store submitter {task_id}: reconnected \
											 after {consecutive_conn_errors} failures"
										);
										consecutive_conn_errors = 0;
									},
									Err(re) => {
										if consecutive_conn_errors.is_multiple_of(10) {
											log::warn!(
												"bulk_store submitter {task_id}: reconnect \
												 failed ({consecutive_conn_errors} attempts): {re}"
											);
										}
										// Give up after 60 consecutive failures (~5 min)
										if consecutive_conn_errors >= 60 {
											log::error!(
												"bulk_store submitter {task_id}: giving up \
												 after {consecutive_conn_errors} reconnect \
												 failures"
											);
											errors.fetch_add(1, Ordering::Relaxed);
											break;
										}
									},
								}
							},
							TxPoolError::TxDropped => {
								// Tx entered pool but was later evicted. The nonce may
								// have been consumed on-chain (tx could have been
								// included before eviction). Do NOT re-queue — treat
								// as a loss.
								consecutive_conn_errors = 0;
								log::warn!(
									"bulk_store submitter {task_id}: tx dropped from \
									 pool (nonce may be consumed), skipping: {e}"
								);
								pool_full_retries.fetch_add(1, Ordering::Relaxed);
								if !pool_saturated.swap(true, Ordering::Relaxed) {
									log::info!(
										"bulk_store submitter {task_id}: pool saturated \
										 (TxDropped) — measurement starts now"
									);
								}
							},
							TxPoolError::AlreadyImported => {
								consecutive_conn_errors = 0;
								log::debug!(
									"bulk_store submitter {task_id}: already imported, \
									 skipping: {e}"
								);
							},
							TxPoolError::StaleNonce => {
								consecutive_conn_errors = 0;
								log::debug!(
									"bulk_store submitter {task_id}: stale nonce (already \
									 used), skipping: {e}"
								);
								stale_nonces.fetch_add(1, Ordering::Relaxed);
							},
							TxPoolError::FutureNonce => {
								consecutive_conn_errors = 0;
								log::warn!(
									"bulk_store submitter {task_id}: future nonce, \
									 skipping: {e}"
								);
								errors.fetch_add(1, Ordering::Relaxed);
							},
							TxPoolError::Other => {
								consecutive_conn_errors = 0;
								log::warn!("bulk_store submitter {task_id}: skipping: {e}");
								errors.fetch_add(1, Ordering::Relaxed);
							},
						}
					},
				}

				if stop.load(Ordering::Relaxed) {
					break;
				}
			}
		}));
	}

	// Wait for submitters to finish.
	for handle in handles {
		let _ = handle.await;
	}

	// If submitters finished without saturating the pool, treat everything as
	// measured (no pre-fill distinction needed).
	if !pool_saturated.load(Ordering::Relaxed) {
		log::info!(
			"bulk_store: submitters finished without pool saturation — all blocks are measured"
		);
		pool_saturated.store(true, Ordering::Relaxed);
		// Re-label any prefill blocks as measured.
		for b in block_stats.lock().unwrap().iter_mut() {
			b.prefill = false;
		}
		let mut ms = measure_start.lock().unwrap();
		if ms.is_none() {
			*ms = Some(start);
		}
	}

	// Wait for the block target or all confirmations, whichever applies.
	if !stop.load(Ordering::Relaxed) {
		if stop_after_blocks.is_some() {
			// Block-target mode: the monitor sets `stop` when enough blocks are seen.
			// Also stop if no blocks WITH TRANSACTIONS appear for 60s (work
			// exhausted / chain stalled). We track confirmed tx count rather than
			// total block count because empty blocks are recorded continuously
			// and would reset the inactivity timer.
			let mut last_confirmed: u64 =
				block_stats.lock().unwrap().iter().map(|b| b.tx_count).sum();
			let mut last_activity = Instant::now();
			let inactivity_limit = Duration::from_secs(TX_TIMEOUT_SECS);
			loop {
				if stop.load(Ordering::Relaxed) {
					let confirmed: u64 =
						block_stats.lock().unwrap().iter().map(|b| b.tx_count).sum();
					log::info!("bulk_store: block target reached, {confirmed} txs confirmed");
					break;
				}
				let current_confirmed: u64 =
					block_stats.lock().unwrap().iter().map(|b| b.tx_count).sum();
				if current_confirmed > last_confirmed {
					last_confirmed = current_confirmed;
					last_activity = Instant::now();
				}
				if last_activity.elapsed() > inactivity_limit {
					let bs = block_stats.lock().unwrap();
					let n = bs.len();
					let confirmed: u64 = bs.iter().map(|b| b.tx_count).sum();
					drop(bs);
					log::warn!(
						"bulk_store: no new confirmed txs for {:.0}s, stopping \
						 ({n} blocks, {confirmed} txs)",
						inactivity_limit.as_secs_f64()
					);
					break;
				}
				tokio::time::sleep(Duration::from_millis(500)).await;
			}
		} else if submitted.load(Ordering::Relaxed) > 0 {
			// No block target: wait for all submitted txs to be confirmed.
			let deadline = Instant::now() + Duration::from_secs(TX_TIMEOUT_SECS * 3);
			loop {
				let confirmed: u64 = block_stats.lock().unwrap().iter().map(|b| b.tx_count).sum();
				let sub = submitted.load(Ordering::Relaxed);
				if confirmed >= sub || stop.load(Ordering::Relaxed) {
					log::info!("bulk_store: all {confirmed}/{sub} txs confirmed");
					break;
				}
				if Instant::now() > deadline {
					log::warn!(
						"bulk_store: timed out waiting for confirmations, {confirmed}/{sub}"
					);
					break;
				}
				tokio::time::sleep(Duration::from_millis(500)).await;
			}
		}
	}

	stop.store(true, Ordering::Relaxed);
	new_block_notify.notify_waiters();
	let _ = monitor_handle.await;

	let duration = measure_start
		.lock()
		.unwrap()
		.map(|ms| ms.elapsed())
		.unwrap_or_else(|| start.elapsed());
	let total_wall = start.elapsed();
	let total_submitted = submitted.load(Ordering::Relaxed);
	let total_errors = errors.load(Ordering::Relaxed);
	let total_pool_full = pool_full_retries.load(Ordering::Relaxed);
	let total_stale = stale_nonces.load(Ordering::Relaxed);
	let remaining = account_queue.lock().unwrap().len();
	let all_blocks = block_stats.lock().unwrap().clone();
	let total_confirmed: u64 = all_blocks.iter().map(|b| b.tx_count).sum();
	let prefill_count = all_blocks.iter().filter(|b| b.prefill).count();
	let measured_count = all_blocks.len() - prefill_count;

	log::info!(
		"bulk_store: DONE — wall={:.1}s, measured={:.1}s, submitted={total_submitted}, \
		 confirmed={total_confirmed}, errors={total_errors}, pool_full_retries={total_pool_full}, \
		 stale_nonces={total_stale}, remaining_in_queue={remaining}, \
		 blocks={} (prefill={prefill_count}, measured={measured_count})",
		total_wall.as_secs_f64(),
		duration.as_secs_f64(),
		all_blocks.len(),
	);

	Ok(BulkStoreResult {
		total_submitted,
		total_confirmed,
		total_errors,
		stale_nonces: total_stale,
		pool_full_retries: total_pool_full,
		remaining_in_queue: remaining as u64,
		nonces_initialized: nonce_ok,
		nonces_failed: nonce_fail,
		duration,
		blocks: all_blocks,
		fork_detections: fork_detections.load(Ordering::Relaxed),
		stalled: false,
		tx_latencies_ms: vec![],
	})
}
