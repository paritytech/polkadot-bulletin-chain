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
	dynamic::{tx, At, Value},
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
						tracing::warn!("monitor: best block subscription ended, reconnecting");
					},
					Err(e) => {
						tracing::warn!("monitor: best block subscribe failed: {e}, retrying in 2s");
					},
				}
				tokio::time::sleep(std::time::Duration::from_secs(2)).await;
				match crate::client::connect(&url).await {
					Ok(new_client) => client = new_client,
					Err(e) => tracing::warn!("monitor: reconnect failed: {e}, retrying"),
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
						tracing::warn!("monitor: finalized subscription ended, reconnecting");
					},
					Err(e) => {
						tracing::warn!("monitor: finalized subscribe failed: {e}, retrying in 2s");
					},
				}
				tokio::time::sleep(std::time::Duration::from_secs(2)).await;
				match crate::client::connect(&url).await {
					Ok(new_client) => client = new_client,
					Err(e) => tracing::warn!("monitor: reconnect failed: {e}, retrying"),
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

/// Current wall clock as milliseconds since Unix epoch.
fn now_ms() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_millis() as u64
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
pub enum TxPoolError {
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

	// Connection-level errors — WebSocket died, needs reconnect.
	if msg.contains("connection reset") ||
		msg.contains("background task closed") ||
		msg.contains("connection closed") ||
		msg.contains("broken pipe") ||
		msg.contains("restart required") ||
		msg.contains("not connected") ||
		msg.contains("i/o error")
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
						tracing::warn!(
							"block #{block_number}: Stored event field_bytes too short ({})",
							fb.len()
						);
					}
				}
			},
		Err(e) => {
			tracing::warn!("block #{block_number}: failed to fetch events: {e}");
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
		});
	}

	let total_items = work_items.len();
	let num_submitters = submitters.min(total_items).max(1);
	let num_connections = num_submitters.max(8).min(total_items).max(1);
	tracing::info!(
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
	tracing::info!("bulk_store: pre-initializing nonces for {} accounts...", keypairs.len());
	let (nonce_ok, nonce_fail) =
		crate::accounts::batch_init_nonces(&pool, &nonce_tracker, &keypairs, num_connections * 4)
			.await;
	tracing::info!("bulk_store: nonces initialized: {nonce_ok} ok, {nonce_fail} failed");
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
						tracing::info!(
							"bulk_store: measurement clock starts at block \
							 #{block_number}"
						);
						*ms = Some(Instant::now());
					}
				}

				if store_tx_count > 0 || !is_prefill {
					tracing::info!(
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
					received_at_ms: None,
					interval_ms: None,
				});

				if !is_prefill && store_tx_count > 0 {
					total_store_blocks += 1;
					if let Some(limit) = stop_after_blocks {
						if total_store_blocks >= limit {
							tracing::info!(
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
	tracing::info!("bulk_store: block monitor ready, starting submitters");

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
									tracing::info!(
										"bulk_store submitter {task_id}: pool saturated \
										 (first PoolFull) — measurement starts now"
									);
								}
								tracing::debug!(
									"bulk_store submitter {task_id}: pool full (1016), \
									 rollback nonce & re-queuing: {e}"
								);
								// Tx never entered pool → nonce not consumed, safe to
								// rollback and retry immediately.
								nonce_tracker.rollback(&account_id);
								account_queue.lock().unwrap().push_front((signer, data));
								// Brief yield to avoid busy-spin, then retry.
								tokio::time::sleep(Duration::from_millis(20)).await;
							},
							TxPoolError::Banned | TxPoolError::ExhaustsResources => {
								pool_full_retries.fetch_add(1, Ordering::Relaxed);
								consecutive_conn_errors = 0;
								tracing::warn!(
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
									tracing::warn!(
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
										tracing::info!(
											"bulk_store submitter {task_id}: reconnected \
											 after {consecutive_conn_errors} failures"
										);
										consecutive_conn_errors = 0;
									},
									Err(re) => {
										if consecutive_conn_errors.is_multiple_of(10) {
											tracing::warn!(
												"bulk_store submitter {task_id}: reconnect \
												 failed ({consecutive_conn_errors} attempts): {re}"
											);
										}
										// Give up after 60 consecutive failures (~5 min)
										if consecutive_conn_errors >= 60 {
											tracing::error!(
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
								tracing::warn!(
									"bulk_store submitter {task_id}: tx dropped from \
									 pool (nonce may be consumed), skipping: {e}"
								);
								pool_full_retries.fetch_add(1, Ordering::Relaxed);
								if !pool_saturated.swap(true, Ordering::Relaxed) {
									tracing::info!(
										"bulk_store submitter {task_id}: pool saturated \
										 (TxDropped) — measurement starts now"
									);
								}
							},
							TxPoolError::AlreadyImported => {
								consecutive_conn_errors = 0;
								tracing::debug!(
									"bulk_store submitter {task_id}: already imported, \
									 skipping: {e}"
								);
							},
							TxPoolError::StaleNonce => {
								consecutive_conn_errors = 0;
								tracing::debug!(
									"bulk_store submitter {task_id}: stale nonce (already \
									 used), skipping: {e}"
								);
								stale_nonces.fetch_add(1, Ordering::Relaxed);
							},
							TxPoolError::FutureNonce => {
								consecutive_conn_errors = 0;
								tracing::warn!(
									"bulk_store submitter {task_id}: future nonce, \
									 skipping: {e}"
								);
								errors.fetch_add(1, Ordering::Relaxed);
							},
							TxPoolError::Other => {
								consecutive_conn_errors = 0;
								tracing::warn!("bulk_store submitter {task_id}: skipping: {e}");
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
		tracing::info!(
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
					tracing::info!("bulk_store: block target reached, {confirmed} txs confirmed");
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
					tracing::warn!(
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
					tracing::info!("bulk_store: all {confirmed}/{sub} txs confirmed");
					break;
				}
				if Instant::now() > deadline {
					tracing::warn!(
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

	tracing::info!(
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
	})
}

// ---------------------------------------------------------------------------
// Sequential nonce upload (single account, pre-signed, wave-based)
// ---------------------------------------------------------------------------

/// A pre-signed store extrinsic with metadata.
pub struct PreSignedTx {
	pub nonce: u64,
	pub encoded: Vec<u8>,
	pub tx_hash: H256,
	pub payload_size: usize,
}

/// Result of a sequential nonce upload.
pub struct SequentialUploadResult {
	pub total_submitted: u64,
	pub total_confirmed: u64,
	pub total_errors: u64,
	pub gap_repairs: u64,
	pub waves_submitted: u64,
	pub duration: Duration,
	pub blocks: Vec<BlockStats>,
	pub fork_detections: u64,
}

/// Sign a batch of store extrinsics with explicit sequential nonces.
///
/// Fetches the current best block header **once** and reuses it for all txs
/// (mortal era anchor).  Each tx is then signed offline — no RPC round-trips
/// in the loop.
pub async fn sign_sequential_txs(
	client: &OnlineClient<BulletinConfig>,
	signer: &Keypair,
	payloads: &[Vec<u8>],
	start_nonce: u64,
	mortality_block: Option<(u64, H256)>,
) -> Result<Vec<PreSignedTx>> {
	use subxt::config::Hasher;
	use subxt::config::transaction_extensions::Params;

	// Use the provided block for the mortal era anchor (typically the
	// best block we just received), or fall back to finalized.
	let (block_number, block_hash) = match mortality_block {
		Some((n, h)) => (n, h),
		None => {
			let block_ref = client.backend().latest_finalized_block_ref().await?;
			let header = client
				.backend()
				.block_header(block_ref.hash())
				.await?
				.ok_or_else(|| anyhow!("cannot fetch block header"))?;
			(header.number.into(), block_ref.hash())
		},
	};

	let mut txs = Vec::with_capacity(payloads.len());
	for (i, payload) in payloads.iter().enumerate() {
		let nonce = start_nonce + i as u64;
		let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(payload)]);

		// Build params and inject block + nonce manually so
		// create_partial_offline makes zero RPC calls.
		let mut params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).mortal(16).build();
		params.inject_block(block_number, block_hash);

		let mut partial = client.tx().create_partial_offline(&store_call, params)?;
		let signed = partial.sign(signer);
		let tx_hash = client.hasher().hash(signed.encoded());
		txs.push(PreSignedTx {
			nonce,
			encoded: signed.into_encoded(),
			tx_hash,
			payload_size: payload.len(),
		});
	}
	Ok(txs)
}

/// Fire-and-forget a slice of pre-signed txs in parallel via `author_submitExtrinsic`.
/// Uses the backend's `submit_transaction` which subscribes but we only check the first event.
/// Runs with high concurrency across multiple RPC connections.
pub async fn submit_sequential_wave(
	ws_urls: &[String],
	txs: &[PreSignedTx],
) -> (u64, Vec<(u64, TxPoolError)>) {
	use futures::stream::{self, StreamExt};
	use jsonrpsee::{core::client::ClientT, rpc_params, ws_client::WsClientBuilder};

	let nonce_range = txs
		.first()
		.zip(txs.last())
		.map(|(f, l)| format!("{}..{}", f.nonce, l.nonce))
		.unwrap_or_default();
	tracing::info!(
		"submit_wave: submitting {} txs (nonces {nonce_range}) to {} endpoints",
		txs.len(),
		ws_urls.len()
	);

	// Build one jsonrpsee WS client per URL for raw RPC calls.
	let mut rpc_clients = Vec::new();
	for url in ws_urls {
		match WsClientBuilder::default()
			.max_request_size(50 * 1024 * 1024)
			.build(url)
			.await
		{
			Ok(c) => rpc_clients.push(std::sync::Arc::new(c)),
			Err(e) => tracing::warn!("submit_wave: failed to connect to {url}: {e}"),
		}
	}
	if rpc_clients.is_empty() {
		tracing::error!("submit_wave: no RPC connections available");
		return (0, vec![]);
	}

	let submit_start = Instant::now();
	let num_clients = rpc_clients.len();
	// Collect raw error strings for the first few failures for debugging.
	let raw_errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
	// Broadcast: send every tx to every RPC endpoint so all pools get it.
	let results: Vec<_> = stream::iter(
		txs.iter().flat_map(|pre| {
			let hex = format!("0x{}", hex::encode(&pre.encoded));
			(0..num_clients).map(move |rpc_idx| (pre.nonce, hex.clone(), rpc_idx))
		}),
	)
		.map(|(nonce, hex, rpc_idx)| {
			let rpc = rpc_clients[rpc_idx].clone();
			let raw_errs = raw_errors.clone();
			async move {
				let result: Result<String, _> =
					rpc.request("author_submitExtrinsic", rpc_params![hex]).await;
				match result {
					Ok(hash) => {
						tracing::trace!("submit nonce {nonce} (RPC {rpc_idx}): accepted (hash={hash})");
						Ok(nonce)
					},
					Err(e) => {
						let msg = format!("{e}");
						let class = classify_tx_error(&anyhow::anyhow!("{}", &msg));
						// Keep first 5 raw errors for log output.
						{
							let mut errs = raw_errs.lock().unwrap();
							if errs.len() < 5 {
								errs.push(format!(
									"nonce {nonce} (RPC {rpc_idx}): {class:?} — {msg}"
								));
							}
						}
						Err((nonce, class))
					},
				}
			}
		})
		.buffer_unordered(num_clients * 16)
		.collect()
		.await;

	let elapsed = submit_start.elapsed();
	// Count per-nonce: accepted if ANY RPC accepted it.
	let mut accepted_nonces: std::collections::HashSet<u64> = std::collections::HashSet::new();
	let mut errors = Vec::new();
	for r in results {
		match r {
			Ok(nonce) => { accepted_nonces.insert(nonce); },
			Err(e) => errors.push(e),
		}
	}
	let ok = accepted_nonces.len() as u64;

	// Log summary.
	if errors.is_empty() {
		tracing::info!(
			"submit_wave: all {} accepted in {:.1}s",
			ok, elapsed.as_secs_f64()
		);
	} else {
		let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
		for (_, class) in &errors {
			*counts.entry(format!("{class:?}")).or_default() += 1;
		}
		let summary: Vec<_> = counts.iter().map(|(k, v)| format!("{k}={v}")).collect();
		tracing::warn!(
			"submit_wave: {ok} ok, {} errors in {:.1}s [{}]",
			errors.len(),
			elapsed.as_secs_f64(),
			summary.join(", ")
		);
		// Log first few raw errors for debugging.
		for msg in raw_errors.lock().unwrap().iter() {
			tracing::warn!("  sample error: {msg}");
		}
	}

	(ok, errors)
}

/// Absolute timeout for the entire upload.
const SEQ_TIMEOUT_SECS: u64 = 600;

/// Compute how many payloads (starting at `from_idx`) fit in one block,
/// accumulating weight and length until a limit is hit.
fn compute_batch_end(
	payloads: &[Vec<u8>],
	from_idx: usize,
	limits: &crate::chain_info::ChainLimits,
) -> usize {
	let mut weight = 0u64;
	let mut length = 0u64;
	let mut count = 0u32;
	let mut idx = from_idx;

	while idx < payloads.len() {
		let payload_len = payloads[idx].len() as u64;
		let tx_weight =
			limits.store_weight_base + limits.store_weight_per_byte * payload_len;
		let tx_length = payload_len + limits.extrinsic_length_overhead;

		if weight + tx_weight > limits.max_normal_weight {
			break;
		}
		if length + tx_length > limits.normal_block_length {
			break;
		}
		if count + 1 > limits.max_block_transactions {
			break;
		}

		weight += tx_weight;
		length += tx_length;
		count += 1;
		idx += 1;
	}
	idx
}

/// Upload data using a single account with sequential nonces.
///
/// Subscribes to best blocks on the first RPC endpoint. On every best block,
/// reads the account nonce at that block's hash and signs+submits a fresh
/// batch from that nonce. The batch size is computed dynamically from the
/// actual payload sizes to fill one block. Uses a single RPC for block
/// watching because with a single submitting account, all nonce progress
/// is sequential. Submission still goes to all RPCs in parallel.
///
/// Finalized blocks are monitored for completion only.
pub async fn sequential_nonce_upload(
	client: &OnlineClient<BulletinConfig>,
	signer: &Keypair,
	payloads: Vec<Vec<u8>>,
	ws_urls: Vec<String>,
	limits: &crate::chain_info::ChainLimits,
) -> Result<SequentialUploadResult> {
	let total_txs = payloads.len() as u64;

	// Query start nonce.
	let account_id = signer.public_key().to_account_id();
	let start_nonce = client.tx().account_nonce(&account_id).await?;
	let expected_final = start_nonce + total_txs;
	tracing::info!(
		"sequential_nonce_upload: {total_txs} txs, \
		 start_nonce={start_nonce}, expected_final={expected_final}"
	);

	tracing::info!("Using {} RPC endpoints for submission", ws_urls.len());

	// Connect to the first RPC for monitoring.
	let monitor_client = crate::client::connect(&ws_urls[0]).await?;

	// Lightweight header-only subscriptions — no full block/event
	// downloads.  We only need block numbers as triggers.
	let mut best_sub = monitor_client.blocks().subscribe_best().await?;
	let mut fin_sub = monitor_client.blocks().subscribe_finalized().await?;

	// Raw RPC client for system_accountNextIndex calls.
	let rpc_client = jsonrpsee::ws_client::WsClientBuilder::default()
		.build(&ws_urls[0])
		.await
		.map_err(|e| anyhow!("failed to connect RPC client: {e}"))?;

	let start = Instant::now();
	let deadline = start + Duration::from_secs(SEQ_TIMEOUT_SECS);

	// Record the finalized block at start — we'll scan from here after.
	let start_finalized_block = {
		let fin_ref = monitor_client.backend().latest_finalized_block_ref().await?;
		let header = monitor_client.backend().block_header(fin_ref.hash()).await?
			.ok_or_else(|| anyhow!("cannot fetch finalized header"))?;
		let num: u64 = header.number.into();
		tracing::info!("Start finalized block: #{num}");
		num
	};

	// ── Watermarks ─────────────────────────────────────────────────────
	let mut best_nonce = start_nonce;
	let mut finalized_nonce = start_nonce;

	// ── Stats ──────────────────────────────────────────────────────────
	let mut waves_submitted: u64 = 0;
	let mut total_submitted: u64 = 0;
	let mut total_errors: u64 = 0;
	let all_blocks = Vec::<BlockStats>::new();
	let mut measure_start: Option<Instant> = None;
	let mut measure_end: Option<Instant> = None;
	let mut end_finalized_block: u64 = 0;


	// Helper: sign and submit a batch.
	async fn sign_and_submit(
		client: &OnlineClient<BulletinConfig>,
		signer: &Keypair,
		payloads: &[Vec<u8>],
		start_nonce: u64,
		from_nonce: u64,
		to_nonce: u64,
		ws_urls: &[String],
		mortality_block: Option<(u64, H256)>,
	) -> Result<(u64, u64)> {
		let from_idx = (from_nonce - start_nonce) as usize;
		let to_idx = (to_nonce - start_nonce) as usize;
		let batch = sign_sequential_txs(
			client, signer, &payloads[from_idx..to_idx], from_nonce, mortality_block,
		).await?;
		let (ok, errs) = submit_sequential_wave(ws_urls, &batch).await;
		Ok((ok, errs.len() as u64))
	}

	// ── Initial wave ───────────────────────────────────────────────────
	let initial_end_idx = compute_batch_end(&payloads, 0, limits);
	let initial_to = start_nonce + initial_end_idx as u64;
	tracing::info!(
		"Wave 0: signing nonces {}..{} ({} txs)",
		start_nonce,
		initial_to - 1,
		initial_to - start_nonce
	);
	let (ok, err_count) = sign_and_submit(
		client, signer, &payloads, start_nonce, start_nonce, initial_to, &ws_urls, None,
	)
	.await?;
	total_submitted += ok;
	total_errors += err_count;
	waves_submitted += 1;
	tracing::info!("  → {ok} accepted, {err_count} errors");

	// ── Main loop ──────────────────────────────────────────────────────
	loop {
		if Instant::now() > deadline {
			tracing::warn!("sequential_nonce_upload: timeout after {}s", SEQ_TIMEOUT_SECS);
			break;
		}

		tokio::select! {
			block = best_sub.next() => {
				let Some(Ok(block)) = block else {
					tracing::warn!("Best block subscription ended");
					break;
				};
				let block_number = block.number() as u64;
				let block_hash = block.hash();
				let parent_hash_bytes = block.header().parent_hash;

				// ── Read account nonce via system_accountNextIndex ──
				// This RPC uses the node's internal best block (not a
				// specific hash), so it never fails on pruned state.
				let prev_best = best_nonce;
				{
					use jsonrpsee::core::client::ClientT;
					match rpc_client.request::<u64, _>(
						"system_accountNextIndex",
						jsonrpsee::rpc_params![account_id.to_string()],
					).await {
						Ok(n) => {
							tracing::info!(
								"Block #{block_number} (hash={block_hash:?}, parent={parent_hash_bytes:?}): \
								 accountNextIndex={n} (prev={prev_best}, delta={}{})",
								if n >= prev_best { "+" } else { "" },
								n as i64 - prev_best as i64,
							);
							best_nonce = n;
						},
						Err(e) => {
							tracing::debug!(
								"Block #{block_number}: accountNextIndex failed: {e}"
							);
							continue;
						},
					}
				}

				let confirmed = best_nonce.saturating_sub(start_nonce);

				if measure_start.is_none() && best_nonce > start_nonce {
					measure_start = Some(Instant::now());
				}

				// All txs in best? Stop submitting but keep reading
				// nonce — a reorg could drop it back down.
				if best_nonce >= expected_final {
					if measure_end.is_none() {
						measure_end = Some(Instant::now());
					}
					tracing::info!(
						"Block #{block_number}: all {total_txs} txs in best \
						 (nonce={best_nonce}), waiting for finalization"
					);
					continue;
				}

				// Reorg detected — nonce went backwards, clear measure_end
				// so we resume timing.
				if measure_end.is_some() {
					tracing::warn!(
						"Block #{block_number}: reorg detected, nonce dropped \
						 to {best_nonce}, resuming submission"
					);
					measure_end = None;
				}

				// ── Feed the pool ───────────────────────────────────
				// Sign a fresh batch from the current nonce every block.
				// Batch size is computed from actual payload sizes.
				let from_idx = (best_nonce - start_nonce) as usize;
				let end_idx = compute_batch_end(&payloads, from_idx, limits);
				let feed_to = start_nonce + end_idx as u64;
				let count = feed_to - best_nonce;
				waves_submitted += 1;
				tracing::info!(
					"Block #{block_number}: nonce {best_nonce} \
					 (+{}), confirmed {confirmed}/{total_txs} → \
					 wave {waves_submitted}: sign {count} txs ({best_nonce}..{})",
					best_nonce.saturating_sub(prev_best),
					feed_to - 1,
				);
				match sign_and_submit(
					client, signer, &payloads, start_nonce,
					best_nonce, feed_to, &ws_urls,
					Some((block_number, block.hash())),
				).await {
					Ok((ok, err_count)) => {
						total_submitted += ok;
						total_errors += err_count;
						tracing::info!("  → {ok} accepted, {err_count} errors");
					},
					Err(e) => {
						tracing::warn!("  → sign_and_submit failed: {e}");
					},
				}
			}
			fin_block = fin_sub.next() => {
				let Some(Ok(fin_block)) = fin_block else {
					tracing::warn!("Finalized subscription ended");
					break;
				};
				let fin_number = fin_block.number() as u64;
				let fin_hash = fin_block.hash();

				// Query nonce at finalized head.
				let nonce_addr = subxt::dynamic::storage(
					"System", "Account",
					vec![Value::from_bytes(account_id.0)],
				);
				let new_fin = monitor_client.storage()
					.at(fin_hash).fetch(&nonce_addr).await
					.ok().flatten()
					.and_then(|v| v.to_value().ok())
					.and_then(|v| v.at("nonce")
						.and_then(|n| n.as_u128())
						.map(|n| n as u64))
					.unwrap_or(finalized_nonce);

				let fin_confirmed = new_fin.saturating_sub(start_nonce);
				let best_confirmed = best_nonce.saturating_sub(start_nonce);
				let fin_lag = best_nonce.saturating_sub(new_fin);

				if new_fin > finalized_nonce {
					finalized_nonce = new_fin;
					tracing::info!(
						"Finalized #{fin_number} (hash={fin_hash:?}): nonce {new_fin}, \
						 confirmed {fin_confirmed}/{total_txs} \
						 (best={best_nonce}, lag={fin_lag} nonces)"
					);
				} else {
					tracing::info!(
						"Finalized #{fin_number} (hash={fin_hash:?}): nonce unchanged at {new_fin} \
						 ({fin_confirmed}/{total_txs}), best={best_nonce} ({best_confirmed}/{total_txs}), \
						 lag={fin_lag} nonces"
					);
				}
				if finalized_nonce >= expected_final {
					end_finalized_block = fin_number;
					tracing::info!(
						"All {total_txs} txs FINALIZED (nonce={finalized_nonce}, \
						 block #{fin_number})"
					);
					break;
				}
			}
			_ = tokio::time::sleep(Duration::from_secs(30)) => {
				tracing::debug!("No events for 30s, continuing...");
			}
		}
	}

	let last_on_chain_nonce = finalized_nonce.max(best_nonce);

	// Duration covers first-tx-in-best → all-txs-in-best, excluding
	// the finalization wait that follows.
	let duration = match (measure_start, measure_end) {
		(Some(s), Some(e)) => e.duration_since(s),
		(Some(s), None) => s.elapsed(),
		_ => start.elapsed(),
	};
	let total_confirmed = last_on_chain_nonce.saturating_sub(start_nonce);

	// ── Post-finalization scan ──────────────────────────────────────
	// Scan the finalized block range [start_finalized_block, end_finalized_block]
	// to build accurate per-block stats for THIS account only.
	let result_blocks = if end_finalized_block > start_finalized_block {
		tracing::info!(
			"Scanning finalized blocks #{start_finalized_block}..#{end_finalized_block} \
			 for per-account stats..."
		);
		let mut final_blocks = Vec::<BlockStats>::new();
		let storage = monitor_client.storage().at_latest().await?;
		let nonce_addr = subxt::dynamic::storage(
			"System",
			"Account",
			vec![Value::from_bytes(account_id.0)],
		);
		let mut prev_nonce = start_nonce;

		for block_num in start_finalized_block..=end_finalized_block {
			// Fetch block hash by number.
			let hash_addr = subxt::dynamic::storage(
				"System",
				"BlockHash",
				vec![Value::u128(block_num as u128)],
			);
			let block_hash = match storage.fetch(&hash_addr).await {
				Ok(Some(val)) => {
					let encoded = val.encoded();
					if encoded.len() >= 32 {
						H256::from_slice(&encoded[..32])
					} else {
						continue;
					}
				},
				_ => continue,
			};
			if block_hash == H256::zero() {
				continue;
			}

			// Read account nonce at this block.
			let nonce_at_block = monitor_client
				.storage()
				.at(block_hash)
				.fetch(&nonce_addr)
				.await
				.ok()
				.flatten()
				.and_then(|v| v.to_value().ok())
				.and_then(|v| {
					v.at("nonce")
						.and_then(|n| n.as_u128())
						.map(|n| n as u64)
				})
				.unwrap_or(prev_nonce);

			// Delta = how many of our txs were included in this block.
			let our_tx_count = nonce_at_block.saturating_sub(prev_nonce);

			// Estimate payload bytes from the payloads array.
			let our_payload_bytes: u64 = if our_tx_count > 0 {
				let from_idx = (prev_nonce - start_nonce) as usize;
				let to_idx = (nonce_at_block - start_nonce) as usize;
				payloads[from_idx..to_idx.min(payloads.len())]
					.iter()
					.map(|p| p.len() as u64)
					.sum()
			} else {
				0
			};

			let timestamp_ms = read_timestamp_at(&monitor_client, block_hash)
				.await
				.ok();

			if our_tx_count > 0 {
				tracing::info!(
					"  Block #{block_num}: {our_tx_count} txs, \
					 nonce {prev_nonce}→{nonce_at_block}, {} KB",
					our_payload_bytes / 1024
				);
			}

			final_blocks.push(BlockStats {
				number: block_num,
				tx_count: our_tx_count,
				payload_bytes: our_payload_bytes,
				prefill: false,
				timestamp_ms,
				hash: Some(format!("{block_hash:?}")),
				finalized: true,
				received_at_ms: Some(now_ms()),
				interval_ms: None,
			});

			prev_nonce = nonce_at_block;

			// Stop early if we've accounted for all txs.
			if nonce_at_block >= expected_final {
				break;
			}
		}

		let our_total: u64 = final_blocks.iter().map(|b| b.tx_count).sum();
		let blocks_with_txs = final_blocks.iter().filter(|b| b.tx_count > 0).count();
		tracing::info!(
			"Finalized scan: {our_total} txs across {blocks_with_txs} blocks"
		);
		final_blocks
	} else {
		tracing::warn!("No finalized block range to scan, using best-block stats");
		all_blocks
	};

	let fin_lag = best_nonce.saturating_sub(finalized_nonce);
	tracing::info!(
		"sequential_nonce_upload: DONE — confirmed={total_confirmed}/{total_txs} \
		 (best={best_nonce}, finalized={finalized_nonce}, fin_lag={fin_lag}), \
		 submitted={total_submitted}, errors={total_errors}, \
		 waves={waves_submitted}, duration={:.1}s",
		duration.as_secs_f64()
	);
	if fin_lag > 0 {
		tracing::warn!(
			"Finalization gap: {fin_lag} txs in best but not yet finalized. \
			 best_nonce={best_nonce}, finalized_nonce={finalized_nonce}. \
			 The test may need more time for relay chain finality to catch up."
		);
	}

	Ok(SequentialUploadResult {
		total_submitted,
		total_confirmed,
		total_errors,
		gap_repairs: 0,
		waves_submitted,
		duration,
		blocks: result_blocks,
		fork_detections: 0,
	})
}
