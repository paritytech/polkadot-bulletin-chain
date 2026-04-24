//! Producer / consumer pipeline for block-capacity throughput testing.
//!
//! A **generator** ([`generate_block_capacity_work`]) signs store extrinsics and sends
//! [`StressWorkItem`]s on a bounded `mpsc` channel. Signing of batch N+1 is overlapped with
//! dispatch of batch N (look-ahead). For each batch the reader sends `Authorize` →
//! `AwaitPendingAuth` → `Store` items. Store items are dispatched to **N worker tasks** over
//! bounded per-worker channels. Every [`POOL_PENDING_PAUSE_THRESHOLD`] items, the reader pauses
//! until the estimated pending pool depth drops. Workers use fire-and-forget RPC
//! (`author_submitExtrinsic`) for maximum throughput.

use anyhow::Result;
use futures::{
	future::join_all,
	stream::{self, StreamExt, TryStreamExt},
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{
	sync::{
		atomic::{AtomicBool, AtomicU64, Ordering},
		Arc, Mutex,
	},
	time::{Duration, Instant},
};
use subxt::{utils::AccountId32, OnlineClient};
use subxt_signer::sr25519::Keypair;
use tokio::sync::{mpsc, mpsc::error::TrySendError, Notify};

use crate::{
	accounts::NonceTracker,
	authorize::{self, AUTHORIZE_BATCH_SIZE},
	client::BulletinConfig,
	report::BlockStats,
	store::{
		classify_tx_error, read_timestamp_at, sign_store_extrinsic_blocking,
		store_submit_pre_signed, stored_content_hashes, BulkStoreResult, DualBlockSubscription,
		PendingBlock, TxPoolError,
	},
};

/// Atomic counters updated by workers (lock-free hot path).
#[derive(Default)]
struct SubmitCounters {
	submitted: AtomicU64,
	submitted_bytes: AtomicU64,
	errors: AtomicU64,
	pool_full_retries: AtomicU64,
	stale_nonces: AtomicU64,
}

/// Content hash → (extrinsic size, submission time). The monitor uses this to compute
/// per-tx inclusion latency and per-block byte accounting.
pub type ContentHashMap = std::collections::HashMap<[u8; 32], (u64, Instant)>;


/// Bounded capacity for the generator → reader `mpsc` (backpressure when full).
pub const WORK_CHANNEL_CAPACITY: usize = 1000;

/// Maximum in-flight `spawn_blocking` tasks (each running `generate_payload` +
/// `sign_store_extrinsic_blocking`) when building store work items ([`build_store_work_items`]).
/// Uses `buffer_unordered` so a new task starts as soon as one completes (no chunk barriers).
pub const STORE_SIGN_PARALLELISM: usize = 64;

/// Weighted mix of store payload sizes (integer weights; any positive scale).
///
/// Used by [`StorePayloadMode::Mixed`] so each signed store draws a size from a distribution.
#[derive(Clone, Debug)]
pub struct PayloadSizeMix {
	sizes: Vec<usize>,
	weights: Vec<u32>,
	total: u32,
}

impl PayloadSizeMix {
	/// Build from `(payload_bytes, weight)` pairs. Entries with weight `0` are skipped.
	pub fn from_weighted_sizes(pairs: &[(usize, u32)]) -> anyhow::Result<Self> {
		if pairs.is_empty() {
			anyhow::bail!("PayloadSizeMix: need at least one (size, weight)");
		}
		let mut sizes = Vec::with_capacity(pairs.len());
		let mut weights = Vec::with_capacity(pairs.len());
		let mut total = 0u32;
		for &(sz, w) in pairs {
			if w == 0 {
				continue;
			}
			sizes.push(sz);
			weights.push(w);
			total = total.saturating_add(w);
		}
		if total == 0 {
			anyhow::bail!("PayloadSizeMix: all weights zero");
		}
		Ok(Self { sizes, weights, total })
	}

	#[must_use]
	pub fn max_payload_bytes(&self) -> usize {
		*self.sizes.iter().max().unwrap_or(&0)
	}

	#[must_use]
	pub fn min_payload_bytes(&self) -> usize {
		*self.sizes.iter().min().unwrap_or(&0)
	}

	/// Expected payload size (for capacity estimates and monitor byte stats).
	#[must_use]
	pub fn mean_payload_bytes(&self) -> f64 {
		let sum: f64 = self
			.sizes
			.iter()
			.zip(self.weights.iter())
			.map(|(&s, &w)| s as f64 * f64::from(w))
			.sum();
		sum / f64::from(self.total)
	}

	pub fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> usize {
		let mut r = rng.gen_range(0..self.total);
		for i in 0..self.sizes.len() {
			let w = self.weights[i];
			if r < w {
				return self.sizes[i];
			}
			r -= w;
		}
		*self.sizes.last().expect("non-empty")
	}
}

/// How [`build_store_work_items`] chooses per-account payload sizes.
#[derive(Clone, Debug)]
pub enum StorePayloadMode {
	Fixed(usize),
	Mixed(PayloadSizeMix),
}

impl StorePayloadMode {
	#[must_use]
	pub fn authorize_bytes_per_account(&self) -> u64 {
		let max_payload = match self {
			Self::Fixed(n) => *n,
			Self::Mixed(m) => m.max_payload_bytes(),
		};
		(max_payload.saturating_add(1024)) as u64
	}
}

/// Max ready+future tx pool depth before [`wait_until_txpool_can_pull_work`] returns; also how many
/// **store work items dispatched** to per-worker channels before the reader calls that check
/// **before** the next `work_rx.recv()` (backpressure when worker queues are saturated).
pub const POOL_PENDING_PAUSE_THRESHOLD: usize = 4000;

/// Accounts per iteration for the block-capacity pipeline (from estimated block tx capacity).
///
/// `measured_blocks_per_iteration` is how many **measured** blocks worth of one-shot txs each
/// iteration targets (`≈` that × `est_block_cap` accounts).
#[must_use]
pub fn block_capacity_accounts_per_iteration(
	est_block_cap: usize,
	measured_blocks_per_iteration: u32,
) -> u32 {
	(measured_blocks_per_iteration as u64 * est_block_cap as u64).max(1) as u32
}

/// One unit of work for the block-capacity pipeline.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum StressWorkItem {
	/// Authorization batch. The reader spawns this as a **background task** so store dispatch
	/// from the previous batch can continue concurrently.
	Authorize { batches: Vec<Vec<subxt::utils::AccountId32>> },
	/// Barrier: the reader must await the in-flight background authorization task before
	/// processing further [`Store`](Self::Store) items.
	AwaitPendingAuth,
	/// Pre-signed `TransactionStorage::store` extrinsic for a one-shot account (nonce 0).
	Store { account_id: AccountId32, extrinsic: Arc<Vec<u8>>, content_hash: [u8; 32] },
}

/// One iteration of the sweep: account count and derivation prefix for `//{prefix}/{idx}`.
///
/// Keypairs and store payloads are **not** materialized up front (avoids GB of RAM and long
/// stalls before any RPC when `total_accounts` is huge).
#[derive(Clone)]
pub struct IterationPlan {
	pub account_count: u32,
	pub derivation_prefix: String,
}

/// Split `total_accounts` into iteration metadata (no keypair or payload allocation).
pub fn build_iteration_plans(
	total_accounts: u32,
	max_per_iteration: u32,
	seed_prefix: &str,
) -> Vec<IterationPlan> {
	let max_per_iteration = max_per_iteration.max(1);
	let mut plans = Vec::new();
	let mut offset = 0u32;
	let mut remaining = total_accounts;

	while remaining > 0 {
		let take = remaining.min(max_per_iteration);
		remaining -= take;
		let derivation_prefix = format!("{seed_prefix}_{offset}");
		plans.push(IterationPlan { account_count: take, derivation_prefix });
		offset += take;
	}

	if plans.is_empty() {
		plans.push(IterationPlan { account_count: 0, derivation_prefix: String::new() });
	}

	plans
}

/// Derive keypairs for `[range_start..range_end)` (CPU-heavy; call from the generator).
pub fn keypairs_for_range(plan: &IterationPlan, range_start: u32, range_end: u32) -> Vec<Keypair> {
	(range_start..range_end)
		.map(|idx| crate::accounts::keypair_at_derivation_prefix(&plan.derivation_prefix, idx))
		.collect()
}

/// Build `Utility::batch_all`-sized account id batches from already-derived keypairs.
pub fn auth_batches_from_keypairs(keypairs: &[Keypair]) -> Vec<Vec<subxt::utils::AccountId32>> {
	keypairs
		.chunks(AUTHORIZE_BATCH_SIZE)
		.map(|chunk| chunk.iter().map(|k| k.public_key().to_account_id()).collect())
		.collect()
}


/// Block until the estimated number of in-flight transactions (submitted − confirmed)
/// drops to [`POOL_PENDING_PAUSE_THRESHOLD`] or below.
///
/// This avoids the `author_pendingExtrinsics` RPC which fetches all pending
/// transaction bodies and can fail with "message too large" under heavy load.
async fn wait_until_txpool_can_pull_work(
	counters: &Arc<SubmitCounters>,
	confirmed_count: &Arc<AtomicU64>,
	new_block_notify: &Arc<Notify>,
) {
	let mut logged = false;
	let deadline = Instant::now() + Duration::from_secs(60);
	loop {
		let submitted = counters.submitted.load(Ordering::Relaxed);
		let confirmed = confirmed_count.load(Ordering::Relaxed);
		let estimated_pending = submitted.saturating_sub(confirmed) as usize;

		if estimated_pending <= POOL_PENDING_PAUSE_THRESHOLD {
			if logged {
				log::debug!(
					"pipeline: estimated pending at {estimated_pending} \
					 (≤ {POOL_PENDING_PAUSE_THRESHOLD}), resuming reader",
				);
			}
			return;
		}

		if Instant::now() > deadline {
			log::warn!(
				"pipeline: backpressure wait timed out (pending estimate={estimated_pending}), \
				 resuming to avoid deadlock"
			);
			return;
		}

		if !logged {
			log::debug!(
				"pipeline: estimated pending {estimated_pending} \
				 (> {POOL_PENDING_PAUSE_THRESHOLD}), pausing reader \
				 (backpressure, submitted={submitted} confirmed={confirmed})",
			);
			logged = true;
		}

		tokio::time::timeout(Duration::from_millis(500), new_block_notify.notified())
			.await
			.ok();
	}
}

#[allow(clippy::too_many_arguments)]
fn spawn_pipeline_dual_monitor(
	dual: DualBlockSubscription,
	new_block_notify: Arc<Notify>,
	block_stats: Arc<Mutex<Vec<BlockStats>>>,
	tx_latencies: Arc<Mutex<Vec<Duration>>>,
	measure_start: Arc<Mutex<Option<Instant>>>,
	monitor_ready: Arc<Notify>,
	cancel: Arc<AtomicBool>,
	target_blocks: Option<u32>,
	target_reached: Arc<AtomicBool>,
	stalled: Arc<AtomicBool>,
	content_hash_map: Arc<Mutex<ContentHashMap>>,
	confirmed_count: Arc<AtomicU64>,
	block_count_offset: u32,
) -> tokio::task::JoinHandle<()> {
	let DualBlockSubscription { mut best_rx, mut finalized_rx, monitor_client, ws_url } = dual;

	tokio::spawn(async move {
		let mut monitor_client = monitor_client;
		let mut best_measured_blocks = 0u32;
		let mut pending: std::collections::HashMap<u64, PendingBlock> =
			std::collections::HashMap::new();
		let mut max_finalized: u64 = 0;
		let mut prev_confirmed_timestamp_ms: Option<u64> = None;
		monitor_ready.notify_one();

		let confirmed_count = confirmed_count.clone();
		let push_finalized = |pb: &PendingBlock, prev_ts: &mut Option<u64>| {
			let interval_ms = match (pb.timestamp_ms, *prev_ts) {
				(Some(ts), Some(prev)) => Some(ts.saturating_sub(prev)),
				_ => None,
			};
			if pb.timestamp_ms.is_some() {
				*prev_ts = pb.timestamp_ms;
			}
			confirmed_count.fetch_add(pb.tx_count, Ordering::Relaxed);
			block_stats.lock().unwrap().push(BlockStats {
				number: pb.number,
				tx_count: pb.tx_count,
				payload_bytes: pb.payload_bytes,
				prefill: false,
				timestamp_ms: pb.timestamp_ms,
				hash: Some(format!("{:?}", pb.hash)),
				finalized: true,
				interval_ms,
			});
		};

		// Confirm all pending blocks in [old_max+1..new_max].
		let drain_finalized =
			|pending: &mut std::collections::HashMap<u64, PendingBlock>,
			 old_max: u64,
			 new_max: u64,
			 prev_ts: &mut Option<u64>| {
				let mut nums: Vec<u64> = pending
					.keys()
					.filter(|&&n| n > old_max && n <= new_max)
					.copied()
					.collect();
				nums.sort();
				for n in nums {
					if let Some(pb) = pending.remove(&n) {
						push_finalized(&pb, prev_ts);
					}
				}
			};

		const STALL_TIMEOUT: Duration = Duration::from_secs(60);
		let mut last_progress = Instant::now();

		loop {
			if cancel.load(Ordering::Relaxed) {
				break;
			}

			if target_reached.load(Ordering::Relaxed) {
				break;
			}

			let stall_deadline =
				tokio::time::Instant::from_std(last_progress + STALL_TIMEOUT);
			tokio::select! {
				_ = tokio::time::sleep_until(stall_deadline) => {
					log::warn!(
						"pipeline monitor: no blocks with txs for {}s, signalling stall",
						STALL_TIMEOUT.as_secs()
					);
					stalled.store(true, Ordering::Relaxed);
					new_block_notify.notify_waiters();
					break;
				}
				Some(block) = best_rx.recv() => {
					let block_number = block.number() as u64;
					let block_hash = block.hash();
					new_block_notify.notify_waiters();

					// Lightweight: get content hashes from Stored events (no block body fetch).
					let hashes = stored_content_hashes(&block).await;
					let store_tx_count = hashes.len() as u64;

					// Look up extrinsic sizes and compute inclusion latencies.
					let now = Instant::now();
					let (store_tx_bytes, block_latencies) = {
						let mut map = content_hash_map.lock().unwrap();
						let mut bytes = 0u64;
						let mut lats = Vec::new();
						for h in &hashes {
							if let Some((size, submitted_at)) = map.remove(h) {
								bytes += size;
								lats.push(now.duration_since(submitted_at));
							}
						}
						(bytes, lats)
					};
					if !block_latencies.is_empty() {
						tx_latencies.lock().unwrap().extend(block_latencies);
					}

					// Only read timestamp for blocks with txs (skip empty blocks).
					let timestamp_ms = if store_tx_count == 0 {
						None
					} else {
						match read_timestamp_at(&monitor_client, block_hash).await {
							Ok(ts) => Some(ts),
							Err(e) if crate::client::is_connection_error(&e) => {
								crate::client::reconnect(
									&mut monitor_client, &ws_url, "pipeline monitor", 1,
								).await;
								read_timestamp_at(&monitor_client, block_hash).await.ok()
							},
							Err(e) => {
								log::warn!(
									"pipeline: block #{block_number}: timestamp read failed: {e}"
								);
								None
							},
						}
					};

					{
						let mut ms = measure_start.lock().unwrap();
						if ms.is_none() {
							log::info!(
								"pipeline: measurement clock starts at block #{block_number}"
							);
							*ms = Some(Instant::now());
						}
					}

					if store_tx_count > 0 {
						last_progress = Instant::now();
						best_measured_blocks += 1;
						let global_count = block_count_offset + best_measured_blocks;
						log::info!(
							"pipeline: [measured] block #{block_number}: \
							 {store_tx_count} store txs, {store_tx_bytes} bytes \
							 (measured #{global_count})"
						);
						if target_blocks.is_some_and(|t| best_measured_blocks >= t) {
							log::info!(
								"pipeline monitor: reached {best_measured_blocks} \
								 measured best blocks (target {:?}), signalling stop",
								target_blocks
							);
							target_reached.store(true, Ordering::Relaxed);
						}
					}

					pending.insert(block_number, PendingBlock {
						number: block_number,
						hash: block_hash,
						tx_count: store_tx_count,
						payload_bytes: store_tx_bytes,
						timestamp_ms,
					});

					// If already finalized (lagging best), confirm immediately.
					if block_number <= max_finalized {
						if let Some(pb) = pending.remove(&block_number) {
							push_finalized(&pb, &mut prev_confirmed_timestamp_ms);
						}
					}
				}

				Some(fin_number) = finalized_rx.recv() => {
					let old_max = max_finalized;
					max_finalized = max_finalized.max(fin_number);
					drain_finalized(&mut pending, old_max, max_finalized, &mut prev_confirmed_timestamp_ms);
				}

				else => break,
			}
		}

		// After work loop stopped: wait for remaining best blocks to finalize.
		if !pending.is_empty() && !cancel.load(Ordering::Relaxed) {
			log::info!(
				"pipeline monitor: waiting for {} pending best blocks to finalize",
				pending.len()
			);
			let finalize_deadline = Instant::now() + Duration::from_secs(30);
			while !pending.is_empty() && Instant::now() < finalize_deadline {
				match tokio::time::timeout(Duration::from_secs(12), finalized_rx.recv()).await {
					Ok(Some(fin_number)) => {
						let old_max = max_finalized;
						max_finalized = max_finalized.max(fin_number);
						drain_finalized(&mut pending, old_max, max_finalized, &mut prev_confirmed_timestamp_ms);
					},
					Ok(None) => {
						log::warn!("pipeline monitor: finalized subscription closed");
						break;
					},
					Err(_) => {
						// No finalization event in 12s — subscription may be stale.
					},
				}
			}
			if !pending.is_empty() {
				log::warn!(
					"pipeline monitor: {} blocks not finalized after timeout, dropping",
					pending.len()
				);
			}
		}
	})
}

struct StoreWorkMsg {
	account_id: AccountId32,
	extrinsic: Arc<Vec<u8>>,
	content_hash: [u8; 32],
}

/// Per-worker state for store submission.
struct StoreWorker {
	worker_id: usize,
	client: Arc<jsonrpsee::ws_client::WsClient>,
	reconnect_url: String,
	consecutive_conn_errors: u32,
	counters: Arc<SubmitCounters>,
	content_hash_map: Arc<Mutex<ContentHashMap>>,
	new_block_notify: Arc<Notify>,
}

impl StoreWorker {
	/// Submit one pre-signed store extrinsic; retries pool-full / banned / reconnect.
	async fn submit(&mut self, msg: &StoreWorkMsg) -> Result<()> {
		let id = self.worker_id;
		loop {
			let result =
				store_submit_pre_signed(self.client.as_ref(), msg.extrinsic.as_ref()).await;

			match result {
				Ok(hash) => {
					let ext_len = msg.extrinsic.len() as u64;
					let n = self.counters.submitted.fetch_add(1, Ordering::Relaxed) + 1;
					self.counters.submitted_bytes.fetch_add(ext_len, Ordering::Relaxed);
					self.content_hash_map.lock().unwrap().insert(msg.content_hash, (ext_len, Instant::now()));
					self.consecutive_conn_errors = 0;
					if n == 1 || n.is_multiple_of(256) {
						log::debug!("pipeline store: worker {id} accepted total={n} hash={hash:?}");
					}
					return Ok(());
				},
				Err(e) => {
					let class = classify_tx_error(&e);
					log::debug!(
						"pipeline store: worker {id} class={class:?} account={} err={e:#}",
						msg.account_id
					);
					match class {
						TxPoolError::PoolFull => {
							self.counters.pool_full_retries.fetch_add(1, Ordering::Relaxed);
							self.consecutive_conn_errors = 0;
							tokio::time::sleep(Duration::from_secs(1)).await;
						},
						TxPoolError::Banned | TxPoolError::ExhaustsResources => {
							self.counters.pool_full_retries.fetch_add(1, Ordering::Relaxed);
							self.consecutive_conn_errors = 0;
							tokio::time::timeout(
								Duration::from_secs(3),
								self.new_block_notify.notified(),
							)
							.await
							.ok();
						},
						TxPoolError::ConnectionDead => {
							self.consecutive_conn_errors += 1;
							if self.consecutive_conn_errors == 1 {
								log::warn!(
									"pipeline store: worker {id} connection dead, reconnecting"
								);
							}
							let c = self.consecutive_conn_errors;
							let backoff = Duration::from_secs((1u64 << c.min(5)).min(30));
							tokio::time::sleep(backoff).await;

							match crate::client::connect_ws(&self.reconnect_url).await {
								Ok(new_client) => {
									self.client = Arc::new(new_client);
									self.consecutive_conn_errors = 0;
								},
								Err(_) =>
									if self.consecutive_conn_errors >= 60 {
										log::error!("pipeline store: worker {id}: giving up reconnect");
										self.counters.errors.fetch_add(1, Ordering::Relaxed);
										return Err(anyhow::anyhow!(
											"pipeline store: reconnect failed (worker {id})"
										));
									},
							}
						},
						TxPoolError::TxDropped => {
							self.consecutive_conn_errors = 0;
							self.counters.pool_full_retries.fetch_add(1, Ordering::Relaxed);
							return Ok(());
						},
						TxPoolError::AlreadyImported => {
							self.consecutive_conn_errors = 0;
							return Ok(());
						},
						TxPoolError::StaleNonce => {
							self.consecutive_conn_errors = 0;
							self.counters.stale_nonces.fetch_add(1, Ordering::Relaxed);
							return Ok(());
						},
						TxPoolError::FutureNonce => {
							self.consecutive_conn_errors = 0;
							self.counters.errors.fetch_add(1, Ordering::Relaxed);
							return Ok(());
						},
						TxPoolError::Other => {
							self.consecutive_conn_errors = 0;
							log::warn!("pipeline store: worker {id} (class={class:?}): {e:#}");
							self.counters.errors.fetch_add(1, Ordering::Relaxed);
							return Ok(());
						},
					}
				},
			}
		}
	}
}

/// Prefer a worker with spare capacity; otherwise block on `rr`’s channel
/// with a timeout to avoid deadlocking when all workers are stuck reconnecting.
async fn dispatch_store_to_workers(
	mut msg: StoreWorkMsg,
	txs: &[mpsc::Sender<StoreWorkMsg>],
	rr: &mut usize,
) -> Result<()> {
	let n = txs.len().max(1);
	for attempt in 0..n {
		let i = (*rr + attempt) % n;
		match txs[i].try_send(msg) {
			Ok(()) => {
				*rr = (i + 1) % n;
				return Ok(());
			},
			Err(TrySendError::Full(returned)) => msg = returned,
			Err(TrySendError::Closed(_)) =>
				return Err(anyhow::anyhow!("store worker {i} input channel closed")),
		}
	}
	// All channels full — wait with a timeout so we don’t deadlock if workers are stuck.
	let i = *rr % n;
	match tokio::time::timeout(Duration::from_secs(30), txs[i].send(msg)).await {
		Ok(Ok(())) => {
			*rr = (i + 1) % n;
			Ok(())
		},
		Ok(Err(_)) => Err(anyhow::anyhow!("store worker {i} channel closed")),
		Err(_) => Err(anyhow::anyhow!("dispatch timeout — all workers stalled")),
	}
}

fn spawn_store_submit_workers(
	num_workers: usize,
	pool: &[Arc<jsonrpsee::ws_client::WsClient>],
	ws_urls_owned: &[String],
	counters: Arc<SubmitCounters>,
	content_hash_map: Arc<Mutex<ContentHashMap>>,
	new_block_notify: Arc<Notify>,
) -> (Vec<mpsc::Sender<StoreWorkMsg>>, Vec<tokio::task::JoinHandle<Result<()>>>) {
	let per_worker_cap = 2;
	let mut txs = Vec::with_capacity(num_workers);
	let mut handles = Vec::with_capacity(num_workers);

	for worker_id in 0..num_workers {
		let (tx, mut rx) = mpsc::channel::<StoreWorkMsg>(per_worker_cap);
		txs.push(tx);

		let mut worker = StoreWorker {
			worker_id,
			client: pool[worker_id].clone(),
			reconnect_url: ws_urls_owned[worker_id % ws_urls_owned.len()].clone(),
			consecutive_conn_errors: 0,
			counters: counters.clone(),
			content_hash_map: content_hash_map.clone(),
			new_block_notify: new_block_notify.clone(),
		};

		handles.push(tokio::spawn(async move {
			while let Some(msg) = rx.recv().await {
				worker.submit(&msg).await?;
			}
			Ok(())
		}));
	}

	(txs, handles)
}

/// Run block-capacity load using a pre-planned work stream (generator ↔ bounded channel ↔ reader
/// task + per-connection store workers).
///
/// `submitters` is how many WebSocket RPC clients and matching store worker tasks to spawn (each
/// worker owns one connection); actual count is `max(submitters, 8)`.
#[allow(clippy::too_many_arguments)]
pub async fn run_block_capacity_pipeline(
	mut work_rx: mpsc::Receiver<StressWorkItem>,
	dual: DualBlockSubscription,
	ws_urls: &[&str],
	submitters: usize,
	store_payload: StorePayloadMode,
	client: &OnlineClient<BulletinConfig>,
	authorizer: &Keypair,
	authorizer_nonce_tracker: &NonceTracker,
	cancel: &Arc<AtomicBool>,
	target_blocks: Option<u32>,
	shared_content_hash_map: Arc<Mutex<ContentHashMap>>,
	shared_tx_latencies: Arc<Mutex<Vec<Duration>>>,
	block_count_offset: u32,
) -> Result<BulkStoreResult> {
	let authorize_bytes = store_payload.authorize_bytes_per_account();

	let new_block_notify = Arc::new(Notify::new());
	let block_stats = Arc::new(Mutex::new(Vec::<BlockStats>::new()));
	let measure_start = Arc::new(Mutex::new(None::<Instant>));
	let monitor_ready = Arc::new(Notify::new());
	let target_reached = Arc::new(AtomicBool::new(false));
	let stalled = Arc::new(AtomicBool::new(false));
	let counters = Arc::new(SubmitCounters::default());
	let content_hash_map = shared_content_hash_map;
	let confirmed_count = Arc::new(AtomicU64::new(0));
	let tx_latencies = shared_tx_latencies;

	let monitor_handle = spawn_pipeline_dual_monitor(
		dual,
		new_block_notify.clone(),
		block_stats.clone(),
		tx_latencies.clone(),
		measure_start.clone(),
		monitor_ready.clone(),
		cancel.clone(),
		target_blocks,
		target_reached.clone(),
		stalled.clone(),
		content_hash_map.clone(),
		confirmed_count.clone(),
		block_count_offset,
	);

	monitor_ready.notified().await;
	log::info!("pipeline: block monitor ready, starting work reader + store workers");

	let num_connections = submitters.max(1).max(8);

	let connect_futs: Vec<_> = (0..num_connections)
		.map(|i| {
			let url = ws_urls[i % ws_urls.len()].to_string();
			async move { crate::client::connect_ws(&url).await.map(Arc::new) }
		})
		.collect();
	let pool: Vec<_> = futures::future::try_join_all(connect_futs).await?;

	log::info!("pipeline: {num_connections} store worker(s) connected");

	let ws_urls_owned: Vec<String> = ws_urls.iter().map(|s| s.to_string()).collect();

	let (worker_txs, mut worker_handles) = spawn_store_submit_workers(
		num_connections,
		&pool,
		&ws_urls_owned,
		counters.clone(),
		content_hash_map.clone(),
		new_block_notify.clone(),
	);

	let start = Instant::now();

	let mut dbg_work_auth = 0u64;
	let mut dbg_work_store = 0u64;

	let mut store_worker_rr: usize = 0;
	let mut stores_dispatched_since_txpool: u64 = 0;

	let mut pending_auth: Option<tokio::task::JoinHandle<Result<()>>> = None;

	// Run the work loop; capture errors but don't bail — we always want measurements.
	let work_error: Option<anyhow::Error> = None;

	'work: loop {
		if cancel.load(Ordering::Relaxed) {
			log::warn!("pipeline: cancel requested, stopping work loop");
			break;
		}
		if target_reached.load(Ordering::Relaxed) {
			log::info!("pipeline: target block count reached, stopping work loop");
			break;
		}
		if stalled.load(Ordering::Relaxed) {
			log::warn!("pipeline: stall detected, stopping work loop for restart");
			break;
		}

		if stores_dispatched_since_txpool >= POOL_PENDING_PAUSE_THRESHOLD as u64 {
			let bp_start = Instant::now();
			wait_until_txpool_can_pull_work(&counters, &confirmed_count, &new_block_notify).await;
			let bp_elapsed = bp_start.elapsed();
			if bp_elapsed.as_millis() > 100 {
				log::debug!(
					"pipeline: backpressure paused reader for {:.1}s",
					bp_elapsed.as_secs_f64()
				);
			}
			stores_dispatched_since_txpool = 0;
		}

		let Some(item) = work_rx.recv().await else {
			break;
		};

		match item {
			StressWorkItem::Authorize { batches } => {
				if batches.is_empty() {
					continue;
				}
				debug_assert!(
					pending_auth.is_none(),
					"Authorize received while previous auth still pending — \
					 generator must send AwaitPendingAuth between Authorize items"
				);
				dbg_work_auth += 1;
				let n_accounts: usize = batches.iter().map(|b| b.len()).sum();
				log::info!(
					"pipeline: Authorize {n_accounts} accounts, {} batches \
					 (dispatch #{dbg_work_auth})",
					batches.len(),
				);

				// Spawn authorization as a background task. Reconnects on RPC failure,
				// cycling through available URLs.
				let mut task_client = client.clone();
				let task_authorizer = authorizer.clone();
				let task_nonce = authorizer_nonce_tracker.clone();
				let task_ws_urls: Vec<String> =
					ws_urls.iter().map(|s| s.to_string()).collect();
				pending_auth = Some(tokio::spawn(async move {
					let mut failed = 0u32;
					for account_ids in &batches {
						let mut attempts = 0u32;
						loop {
							match authorize::authorize_account_batch(
								&task_client,
								&task_authorizer,
								&task_nonce,
								account_ids,
								1,
								authorize_bytes,
							)
							.await
							{
								Ok(()) => break,
								Err(e) => {
									attempts += 1;
									let authorizer_id =
										task_authorizer.public_key().to_account_id();
									if let Err(re) =
										task_nonce.refresh(&task_client, &authorizer_id).await
									{
										log::warn!("pipeline: nonce refresh failed: {re}");
									}

									if crate::client::is_connection_error(&e) && attempts <= 5 {
										// Cycle through URLs on each reconnect attempt.
										let url = &task_ws_urls
											[attempts as usize % task_ws_urls.len()];
										crate::client::reconnect(
											&mut task_client,
											url,
											"pipeline auth",
											attempts,
										)
										.await;
										continue;
									}
									failed += 1;
									log::warn!(
										"pipeline: auth batch failed ({} accounts): {e:#}",
										account_ids.len()
									);
									break;
								},
							}
						}
					}
					if failed > 0 {
						anyhow::bail!("{failed} of {} auth batches failed", batches.len());
					}
					Ok(())
				}));
			},
			StressWorkItem::AwaitPendingAuth => {
				let await_start = Instant::now();
				if let Some(handle) = pending_auth.take() {
					match tokio::time::timeout(Duration::from_secs(60), handle).await {
						Ok(Ok(Ok(()))) => {
							log::debug!(
								"pipeline: AwaitPendingAuth completed in {:.1}s (auth #{dbg_work_auth})",
								await_start.elapsed().as_secs_f64()
							);
						},
						Ok(Ok(Err(e))) => {
							log::warn!(
								"pipeline: auth task failed after {:.1}s (continuing): {e:#}",
								await_start.elapsed().as_secs_f64()
							);
						},
						Ok(Err(e)) => {
							log::warn!("pipeline: auth task join failed (continuing): {e}");
						},
						Err(_) => {
							log::warn!(
								"pipeline: AwaitPendingAuth timed out after {:.1}s, continuing",
								await_start.elapsed().as_secs_f64()
							);
						},
					}
				}
			},
			StressWorkItem::Store { account_id, extrinsic, content_hash } => {
				if let Err(e) = dispatch_store_to_workers(
					StoreWorkMsg { account_id, extrinsic, content_hash },
					&worker_txs,
					&mut store_worker_rr,
				)
				.await
				{
					log::warn!("pipeline: dispatch failed (workers stalled?): {e:#}");
					// Treat as stall so the sweep retries.
					stalled.store(true, Ordering::Relaxed);
					break 'work;
				}
				stores_dispatched_since_txpool += 1;
				dbg_work_store += 1;
				if dbg_work_store.is_multiple_of(512) {
					let sub = counters.submitted.load(Ordering::Relaxed);
					let conf = confirmed_count.load(Ordering::Relaxed);
					log::debug!(
						"pipeline: dispatched={dbg_work_store} submitted={sub} \
						 confirmed={conf} pending_estimate={}",
						sub.saturating_sub(conf)
					);
				}
			},
		}
	}

	let is_stalled = stalled.load(Ordering::Relaxed);

	shutdown_pipeline(
		cancel,
		&target_reached,
		&stalled,
		pending_auth,
		worker_txs,
		&mut worker_handles,
		&counters,
		&confirmed_count,
	)
	.await;

	// Ensure measurement clock is set (even if no blocks were seen).
	measure_start.lock().unwrap().get_or_insert(start);

	// Wait for the monitor to finalize pending blocks.
	new_block_notify.notify_waiters();
	let monitor_timeout = if cancel.load(Ordering::Relaxed) { 1 } else { 35 };
	if tokio::time::timeout(Duration::from_secs(monitor_timeout), monitor_handle)
		.await
		.is_err()
	{
		log::warn!("pipeline: monitor did not exit in time, aborting");
	}

	collect_results(
		start,
		&measure_start,
		&counters,
		&block_stats,
		&tx_latencies,
		work_error,
		is_stalled,
	)
}

#[allow(clippy::too_many_arguments)]
async fn shutdown_pipeline(
	cancel: &Arc<AtomicBool>,
	target_reached: &Arc<AtomicBool>,
	stalled: &Arc<AtomicBool>,
	pending_auth: Option<tokio::task::JoinHandle<Result<()>>>,
	worker_txs: Vec<mpsc::Sender<StoreWorkMsg>>,
	worker_handles: &mut Vec<tokio::task::JoinHandle<Result<()>>>,
	counters: &Arc<SubmitCounters>,
	confirmed_count: &Arc<AtomicU64>,
) {
	const TX_TIMEOUT_SECS: u64 = 60;
	let stopping = cancel.load(Ordering::Relaxed) ||
		target_reached.load(Ordering::Relaxed) ||
		stalled.load(Ordering::Relaxed);

	if stopping {
		if let Some(handle) = pending_auth {
			handle.abort();
		}
		drop(worker_txs);
		for h in worker_handles.iter() {
			h.abort();
		}
	} else {
		if let Some(handle) = pending_auth {
			match tokio::time::timeout(Duration::from_secs(2), handle).await {
				Ok(Ok(Ok(()))) => {},
				Ok(Ok(Err(e))) => log::warn!("pipeline: trailing auth task failed: {e:#}"),
				Ok(Err(e)) => log::warn!("pipeline: trailing auth task join failed: {e}"),
				Err(_) => log::warn!("pipeline: trailing auth task timed out, skipping"),
			}
		}

		log::info!("pipeline: work stream finished, closing store worker inputs");
		drop(worker_txs);

		if tokio::time::timeout(Duration::from_secs(10), join_all(&mut *worker_handles))
			.await
			.is_err()
		{
			log::warn!("pipeline: store workers did not finish in time, aborting");
			for h in worker_handles.iter() {
				h.abort();
			}
		}

		// Wait for confirmations to catch up with submissions.
		if counters.submitted.load(Ordering::Relaxed) > 0 {
			let deadline = Instant::now() + Duration::from_secs(TX_TIMEOUT_SECS * 3);
			loop {
				let confirmed = confirmed_count.load(Ordering::Relaxed);
				let sub = counters.submitted.load(Ordering::Relaxed);
				if confirmed >= sub {
					break;
				}
				if Instant::now() > deadline {
					log::warn!(
						"pipeline: confirmation wait timed out — \
						 confirmed={confirmed} submitted={sub}, proceeding with partial results",
					);
					break;
				}
				tokio::time::sleep(Duration::from_millis(500)).await;
			}
		}
	}
}

fn collect_results(
	start: Instant,
	measure_start: &Arc<Mutex<Option<Instant>>>,
	counters: &Arc<SubmitCounters>,
	block_stats: &Arc<Mutex<Vec<BlockStats>>>,
	tx_latencies: &Arc<Mutex<Vec<Duration>>>,
	work_error: Option<anyhow::Error>,
	stalled: bool,
) -> Result<BulkStoreResult> {
	let duration = measure_start
		.lock()
		.unwrap()
		.map(|ms| ms.elapsed())
		.unwrap_or_else(|| start.elapsed());
	let total_wall = start.elapsed();
	let total_submitted = counters.submitted.load(Ordering::Relaxed);
	let total_errors = counters.errors.load(Ordering::Relaxed);
	let total_pool_full = counters.pool_full_retries.load(Ordering::Relaxed);
	let total_stale = counters.stale_nonces.load(Ordering::Relaxed);
	let all_blocks = block_stats.lock().unwrap().clone();
	let total_confirmed: u64 = all_blocks.iter().map(|b| b.tx_count).sum();

	if let Some(e) = &work_error {
		log::warn!(
			"pipeline: FINISHED WITH ERROR — wall={:.1}s, submitted={total_submitted}, \
			 confirmed={total_confirmed}, errors={total_errors}, cause: {e:#}",
			total_wall.as_secs_f64(),
		);
	} else {
		log::info!(
			"pipeline: DONE — wall={:.1}s, submitted={total_submitted}, \
			 confirmed={total_confirmed}, errors={total_errors}",
			total_wall.as_secs_f64(),
		);
	}
	log::debug!(
		"pipeline: DONE detail — pool_full_retries={total_pool_full} stale_nonces={total_stale}"
	);

	Ok(BulkStoreResult {
		total_submitted,
		total_confirmed,
		total_errors,
		stale_nonces: total_stale,
		pool_full_retries: total_pool_full,
		remaining_in_queue: 0,
		nonces_initialized: 0,
		nonces_failed: 0,
		duration,
		blocks: all_blocks,
		fork_detections: 0,
		stalled,
		tx_latencies_ms: tx_latencies
			.lock()
			.unwrap()
			.iter()
			.map(|d| d.as_secs_f64() * 1000.0)
			.collect(),
	})
}

/// Sign [`StressWorkItem::Store`] items for `keypairs` (does not send them). Runs up to
/// [`STORE_SIGN_PARALLELISM`] tasks concurrently via `buffer_unordered`. Each task runs payload
/// generation, SCALE encoding, and sr25519 signing in a single [`tokio::task::spawn_blocking`]
/// call, keeping all CPU work off the async runtime.
///
/// For [`StorePayloadMode::Mixed`], each account samples a payload size from the distribution
/// using the shared `mix_rng`.
async fn build_store_work_items(
	client: &OnlineClient<BulletinConfig>,
	keypairs: &[Keypair],
	mode: &StorePayloadMode,
	mix_rng: &Option<Arc<Mutex<StdRng>>>,
) -> Result<Vec<StressWorkItem>> {
	// Pre-sample all payload sizes under the lock once, then release it.
	let payload_sizes: Vec<usize> = match mode {
		StorePayloadMode::Fixed(n) => vec![*n; keypairs.len()],
		StorePayloadMode::Mixed(mix) => {
			let mut g = mix_rng
				.as_ref()
				.ok_or_else(|| anyhow::anyhow!("pipeline: mixed mode requires RNG"))?
				.lock()
				.unwrap();
			(0..keypairs.len()).map(|_| mix.sample(&mut *g)).collect()
		},
	};

	stream::iter(keypairs.iter().cloned().zip(payload_sizes))
		.map(|(kp, payload_size)| {
			let client = client.clone();
			async move {
				let (account_id, encoded, content_hash) =
					tokio::task::spawn_blocking(move || {
						let payload = crate::store::generate_payload(payload_size);
						let content_hash = crate::client::blake2b_256(&payload);
						let encoded = sign_store_extrinsic_blocking(&client, &kp, &payload, 0)?;
						Ok::<_, anyhow::Error>((
							kp.public_key().to_account_id(),
							encoded,
							content_hash,
						))
					})
					.await
					.map_err(|e| anyhow::anyhow!("pipeline: spawn_blocking join: {e}"))??;
				Ok::<_, anyhow::Error>(StressWorkItem::Store {
					account_id,
					extrinsic: Arc::new(encoded),
					content_hash,
				})
			}
		})
		.buffer_unordered(STORE_SIGN_PARALLELISM)
		.try_collect()
		.await
}

/// Sign and push work items with look-ahead signing.
///
/// Signing of batch N+1 runs concurrently (background task) while batch N is
/// authorized and dispatched. Each batch follows: `Authorize` → `AwaitPendingAuth` → `Store` items.
///
/// For [`StorePayloadMode::Mixed`], `mix_seed` fixes the RNG; if `None`, uses OS entropy.
pub async fn generate_block_capacity_work(
	work_tx: mpsc::Sender<StressWorkItem>,
	plans: &[IterationPlan],
	store_payload: StorePayloadMode,
	mix_seed: Option<u64>,
	client: Arc<OnlineClient<BulletinConfig>>,
) -> Result<()> {
	if plans.is_empty() || plans.iter().all(|p| p.account_count == 0) {
		return Ok(());
	}

	let mix_rng = match &store_payload {
		StorePayloadMode::Mixed(_) => Some(Arc::new(Mutex::new(match mix_seed {
			Some(s) => StdRng::seed_from_u64(s),
			None => StdRng::from_entropy(),
		}))),
		StorePayloadMode::Fixed(_) => None,
	};

	// Collect all (plan_idx, batch_start, batch_end) ranges up front so we can
	// look ahead to sign the next batch while dispatching the current one.
	let mut batches: Vec<(usize, u32, u32)> = Vec::new();
	for (iter_idx, plan) in plans.iter().enumerate() {
		if plan.account_count == 0 {
			continue;
		}
		log::info!(
			"pipeline: block-capacity iteration {} of {} ({} accounts)",
			iter_idx + 1,
			plans.len(),
			plan.account_count,
		);
		let mut batch_start = 0u32;
		while batch_start < plan.account_count {
			let batch_end =
				batch_start.saturating_add(AUTHORIZE_BATCH_SIZE as u32).min(plan.account_count);
			batches.push((iter_idx, batch_start, batch_end));
			batch_start = batch_end;
		}
	}

	// Process batches with look-ahead: sign batch N+1 concurrently while
	// dispatching batch N's stores. This avoids the pool draining during signing.
	type SignResult = Result<(Vec<Vec<AccountId32>>, Vec<StressWorkItem>)>;
	let mut pending_sign: Option<tokio::task::JoinHandle<SignResult>> = None;

	for (batch_idx, &(plan_idx, start, end)) in batches.iter().enumerate() {
		let batch_num = batch_idx + 1;
		let n_accounts = end - start;

		// Get this batch's signed items: either from a previously spawned task
		// or by signing now (first batch, or if look-ahead wasn't possible).
		let sign_start = Instant::now();
		let (auth_batches, store_items) = if let Some(handle) = pending_sign.take() {
			log::debug!(
				"generator: batch {batch_num}/{} — awaiting look-ahead signing \
				 ({n_accounts} accounts)",
				batches.len(),
			);
			handle.await.map_err(|e| anyhow::anyhow!("pipeline: sign task join: {e}"))??
		} else {
			log::debug!(
				"generator: batch {batch_num}/{} — signing {n_accounts} accounts (no look-ahead)",
				batches.len(),
			);
			let keypairs = keypairs_for_range(&plans[plan_idx], start, end);
			let auth_batches = auth_batches_from_keypairs(&keypairs);
			let store_items =
				build_store_work_items(&client, &keypairs, &store_payload, &mix_rng).await?;
			(auth_batches, store_items)
		};
		let sign_elapsed = sign_start.elapsed();
		let total_bytes: u64 = store_items
			.iter()
			.map(|item| match item {
				StressWorkItem::Store { extrinsic, .. } => extrinsic.len() as u64,
				_ => 0,
			})
			.sum();
		log::info!(
			"generator: batch {batch_num}/{} — {} stores ready \
			 ({:.1} MB, signed {:.1}s)",
			batches.len(),
			store_items.len(),
			total_bytes as f64 / (1024.0 * 1024.0),
			sign_elapsed.as_secs_f64(),
		);

		// Start signing the NEXT batch in the background while we authorize + dispatch.
		if let Some(&(next_plan_idx, next_start, next_end)) = batches.get(batch_idx + 1) {
			let sign_client = client.clone();
			let sign_payload = store_payload.clone();
			let sign_rng = mix_rng.clone();
			let sign_plans = plans[next_plan_idx].clone();
			pending_sign = Some(tokio::spawn(async move {
				let keypairs = keypairs_for_range(&sign_plans, next_start, next_end);
				let auth_batches = auth_batches_from_keypairs(&keypairs);
				let store_items =
					build_store_work_items(&sign_client, &keypairs, &sign_payload, &sign_rng)
						.await?;
				Ok((auth_batches, store_items))
			}));
		}

		// Authorize → wait → dispatch stores.
		work_tx
			.send(StressWorkItem::Authorize { batches: auth_batches })
			.await
			.map_err(|_| anyhow::anyhow!("pipeline work channel closed (auth)"))?;
		work_tx
			.send(StressWorkItem::AwaitPendingAuth)
			.await
			.map_err(|_| anyhow::anyhow!("pipeline work channel closed (await)"))?;

		let dispatch_start = Instant::now();
		let n_stores = store_items.len();
		for item in store_items {
			work_tx
				.send(item)
				.await
				.map_err(|_| anyhow::anyhow!("pipeline work channel closed (store)"))?;
		}
		log::debug!(
			"generator: batch {batch_num}/{} — dispatched {n_stores} stores in {:.1}s",
			batches.len(),
			dispatch_start.elapsed().as_secs_f64(),
		);
	}

	Ok(())
}
