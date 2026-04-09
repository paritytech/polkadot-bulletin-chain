//! Producer / consumer pipeline for long block-capacity runs.
//!
//! A **generator** sends [`StressWorkItem`]s on a **bounded** `mpsc` channel. The **reader** pulls
//! that channel: [`StressWorkItem::Authorize`] is **spawned as a background task**, and
//! [`StressWorkItem::AwaitPendingAuth`] is a barrier that awaits completion of the in-flight auth.
//! Between barriers the reader dispatches [`StressWorkItem::Store`] items to **N worker tasks**
//! over bounded per-worker `mpsc` channels, so authorization of batch N+1 runs concurrently with
//! store dispatch of batch N. Every [`POOL_PENDING_PAUSE_THRESHOLD`] items dispatched to workers,
//! the reader calls [`wait_until_txpool_can_pull_work`] before the next `recv`. Store txs are
//! pre-signed at nonce 0 in the generator ([`crate::store::sign_store_extrinsic`]).

use anyhow::Result;
use futures::{
	future::join_all,
	stream::{self, StreamExt, TryStreamExt},
};
use std::{
	collections::HashMap,
	sync::{
		atomic::{AtomicU64, Ordering},
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
		classify_tx_error, count_stored_events, read_timestamp_at, sign_store_extrinsic_blocking,
		store_submit_pre_signed, BulkStoreResult, DualBlockSubscription, PendingBlock, TxPoolError,
	},
};

#[derive(Default)]
struct SubmitStats {
	submitted: u64,
	errors: u64,
	pool_full_retries: u64,
	stale_nonces: u64,
}

/// Bounded capacity for the generator → reader `mpsc` (backpressure when full).
pub const WORK_CHANNEL_CAPACITY: usize = 1000;

/// Maximum in-flight `sign_store_extrinsic` + `generate_payload` futures when building store work
/// items ([`build_store_work_items`]). Uses `buffer_unordered` so a new future starts as soon as
/// one completes (no chunk barriers).
pub const STORE_SIGN_PARALLELISM: usize = 64;

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
	Store { account_id: AccountId32, extrinsic: Arc<Vec<u8>> },
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

/// Block until `txpool_status` (or fallback RPC) reports ready+future count ≤
/// [`POOL_PENDING_PAUSE_THRESHOLD`], polling the given WS URL (control node).
async fn wait_until_txpool_can_pull_work(ws_url: &str) {
	let mut logged = false;
	loop {
		match crate::client::fetch_txpool_pending_total(ws_url).await {
			Ok(n) if n <= POOL_PENDING_PAUSE_THRESHOLD => {
				if logged {
					log::info!(
						"pipeline: txpool pending at {n} (≤ {POOL_PENDING_PAUSE_THRESHOLD}), \
						 resuming reader",
					);
				}
				return;
			},
			Ok(n) => {
				if !logged {
					log::info!(
						"pipeline: txpool pending {n} (> {POOL_PENDING_PAUSE_THRESHOLD}), pausing \
						 reader (backpressure)",
					);
					logged = true;
				}
				tokio::time::sleep(Duration::from_millis(100)).await;
			},
			Err(e) => {
				log::warn!("pipeline: txpool RPC check failed: {e:#}");
				tokio::time::sleep(Duration::from_millis(500)).await;
			},
		}
	}
}

#[allow(clippy::too_many_arguments)]
fn spawn_pipeline_dual_monitor(
	dual: DualBlockSubscription,
	payload_size: usize,
	fork_detections: Arc<AtomicU64>,
	new_block_notify: Arc<Notify>,
	block_stats: Arc<Mutex<Vec<BlockStats>>>,
	measure_start: Arc<Mutex<Option<Instant>>>,
	monitor_ready: Arc<Notify>,
) -> tokio::task::JoinHandle<()> {
	let DualBlockSubscription { mut best_rx, mut finalized_rx, monitor_client } = dual;

	tokio::spawn(async move {
		let mut measured_blocks = 0u32;
		let mut cumulative_stored_events = 0u64;
		let mut pending: HashMap<u64, PendingBlock> = HashMap::new();
		let mut max_finalized: u64 = 0;
		let mut prev_confirmed_timestamp_ms: Option<u64> = None;
		monitor_ready.notify_one();

		// Shared logic: confirm a finalized pending block and push to stats.
		let confirm_block = |pb: PendingBlock,
		                     prev_ts: &mut Option<u64>,
		                     measured: &mut u32,
		                     cumulative: &mut u64| {
			let interval_ms = match (pb.timestamp_ms, *prev_ts) {
				(Some(ts), Some(prev)) => Some(ts.saturating_sub(prev)),
				_ => None,
			};
			if pb.timestamp_ms.is_some() {
				*prev_ts = pb.timestamp_ms;
			}

			let is_measured = !pb.prefill && pb.tx_count > 0;
			*cumulative = cumulative.saturating_add(pb.tx_count);
			if is_measured {
				*measured += 1;
				if *measured == 1 || measured.is_multiple_of(5) {
					log::debug!(
						"pipeline monitor: finalized measured #{} height={} \
						 store_txs_in_block={} cumulative_stored_events_seen={}",
						*measured,
						pb.number,
						pb.tx_count,
						*cumulative,
					);
				}
			}

			block_stats.lock().unwrap().push(BlockStats {
				number: pb.number,
				tx_count: pb.tx_count,
				payload_bytes: pb.payload_bytes,
				prefill: pb.prefill,
				timestamp_ms: pb.timestamp_ms,
				hash: Some(format!("{:?}", pb.hash)),
				finalized: true,
				interval_ms,
			});
		};

		loop {
			tokio::select! {
				Some(block) = best_rx.recv() => {
					let block_number = block.number() as u64;
					let block_hash = block.hash();
					new_block_notify.notify_waiters();

					let total_store_extrinsics = count_stored_events(&block).await;

					let timestamp_ms = match read_timestamp_at(&monitor_client, block_hash).await {
						Ok(ts) => Some(ts),
						Err(e) => {
							log::warn!(
								"pipeline: block #{block_number}: failed to read timestamp: {e}"
							);
							None
						},
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

					if total_store_extrinsics > 0 {
						log::info!(
							"pipeline: [measured] block #{block_number}: \
							 {total_store_extrinsics} store txs"
						);
					}

					if let Some(old) = pending.get(&block_number) {
						if old.hash != block_hash {
							log::warn!(
								"pipeline: fork detected at block #{block_number}: \
								 hash changed from {:?} to {block_hash:?}",
								old.hash
							);
							fork_detections.fetch_add(1, Ordering::Relaxed);
						}
					}

					pending.insert(block_number, PendingBlock {
						number: block_number,
						hash: block_hash,
						tx_count: total_store_extrinsics,
						payload_bytes: total_store_extrinsics * payload_size as u64,
						timestamp_ms,
						prefill: false,
					});

					if block_number <= max_finalized {
						if let Some(pb) = pending.remove(&block_number) {
							confirm_block(
								pb,
								&mut prev_confirmed_timestamp_ms,
								&mut measured_blocks,
								&mut cumulative_stored_events,
							);
						}
					}
				}

				Some(fin_number) = finalized_rx.recv() => {
					let old_max = max_finalized;
					max_finalized = max_finalized.max(fin_number);

					let mut to_confirm: Vec<u64> = pending
						.keys()
						.filter(|&&n| n > old_max && n <= max_finalized)
						.copied()
						.collect();
					to_confirm.sort();

					for num in to_confirm {
						if let Some(pb) = pending.remove(&num) {
							confirm_block(
								pb,
								&mut prev_confirmed_timestamp_ms,
								&mut measured_blocks,
								&mut cumulative_stored_events,
							);
						}
					}

					let stale: Vec<u64> = pending
						.keys()
						.filter(|&&n| n < max_finalized.saturating_sub(10))
						.copied()
						.collect();
					for num in stale {
						if let Some(pb) = pending.remove(&num) {
							log::warn!(
								"pipeline: fork victim: block #{num} (hash {:?}) \
								 was never finalized, dropping from stats",
								pb.hash
							);
							fork_detections.fetch_add(1, Ordering::Relaxed);
						}
					}
				}

				else => break,
			}
		}
	})
}

type StoreWorkMsg = (AccountId32, Arc<Vec<u8>>);

/// Submit one pre-signed store on a **single** RPC client; retries pool-full / banned / reconnect.
#[allow(clippy::too_many_arguments)]
async fn submit_one_store_single_connection(
	worker_id: usize,
	account_id: AccountId32,
	extrinsic: Arc<Vec<u8>>,
	client: &mut Arc<OnlineClient<BulletinConfig>>,
	reconnect_url: &str,
	consecutive_conn_errors: &mut u32,
	stats: &Arc<Mutex<SubmitStats>>,
	new_block_notify: &Arc<Notify>,
) -> Result<()> {
	loop {
		let submit_result = store_submit_pre_signed(client.as_ref(), extrinsic.as_ref()).await;

		match submit_result {
			Ok(hash) => {
				let n = {
					let mut s = stats.lock().unwrap();
					s.submitted += 1;
					s.submitted
				};
				*consecutive_conn_errors = 0;
				if n == 1 || n.is_multiple_of(256) {
					log::debug!(
						"pipeline store: worker {worker_id} rpc_accepted total={n} \
						 extrinsic_hash={hash:?}"
					);
				}
				return Ok(());
			},
			Err(e) => {
				let class = classify_tx_error(&e);
				log::debug!(
					"pipeline store: worker {worker_id} class={class:?} account={account_id} \
					 err={e:#}"
				);
				match class {
					TxPoolError::PoolFull => {
						stats.lock().unwrap().pool_full_retries += 1;
						*consecutive_conn_errors = 0;
						tokio::time::sleep(Duration::from_millis(100)).await;
					},
					TxPoolError::Banned | TxPoolError::ExhaustsResources => {
						stats.lock().unwrap().pool_full_retries += 1;
						*consecutive_conn_errors = 0;
						tokio::time::timeout(Duration::from_secs(12), new_block_notify.notified())
							.await
							.ok();
					},
					TxPoolError::ConnectionDead => {
						*consecutive_conn_errors += 1;
						if *consecutive_conn_errors == 1 {
							log::warn!(
								"pipeline store: worker {worker_id} connection dead, reconnecting \
								 to {reconnect_url}"
							);
						}
						let c = *consecutive_conn_errors;
						let backoff = Duration::from_secs((1u64 << c.min(5)).min(30));
						tokio::time::sleep(backoff).await;

						match crate::client::connect(reconnect_url).await {
							Ok(new_client) => {
								*client = Arc::new(new_client);
								*consecutive_conn_errors = 0;
							},
							Err(_) =>
								if *consecutive_conn_errors >= 60 {
									log::error!(
										"pipeline store: worker {worker_id}: giving up reconnect"
									);
									stats.lock().unwrap().errors += 1;
									return Err(anyhow::anyhow!(
										"pipeline store: reconnect failed (worker {worker_id})"
									));
								},
						}
					},
					TxPoolError::TxDropped => {
						*consecutive_conn_errors = 0;
						stats.lock().unwrap().pool_full_retries += 1;
						return Ok(());
					},
					TxPoolError::AlreadyImported => {
						*consecutive_conn_errors = 0;
						return Ok(());
					},
					TxPoolError::StaleNonce => {
						*consecutive_conn_errors = 0;
						stats.lock().unwrap().stale_nonces += 1;
						return Ok(());
					},
					TxPoolError::FutureNonce => {
						*consecutive_conn_errors = 0;
						stats.lock().unwrap().errors += 1;
						return Ok(());
					},
					TxPoolError::Other => {
						*consecutive_conn_errors = 0;
						log::warn!("pipeline store: worker {worker_id} (class={class:?}): {e:#}");
						stats.lock().unwrap().errors += 1;
						return Ok(());
					},
				}
			},
		}
	}
}

/// Prefer a worker with spare capacity; otherwise block on `rr`’s channel.
async fn dispatch_store_to_workers(
	mut account_id: AccountId32,
	mut extrinsic: Arc<Vec<u8>>,
	txs: &[mpsc::Sender<StoreWorkMsg>],
	rr: &mut usize,
) -> Result<()> {
	let n = txs.len().max(1);
	for attempt in 0..n {
		let i = (*rr + attempt) % n;
		let msg = (account_id, extrinsic);
		match txs[i].try_send(msg) {
			Ok(()) => {
				*rr = (i + 1) % n;
				return Ok(());
			},
			Err(TrySendError::Full((a, x))) => {
				account_id = a;
				extrinsic = x;
			},
			Err(TrySendError::Closed(_)) =>
				return Err(anyhow::anyhow!("store worker {i} input channel closed")),
		}
	}
	let i = *rr % n;
	txs[i]
		.send((account_id, extrinsic))
		.await
		.map_err(|_| anyhow::anyhow!("store worker {i} send failed (channel closed)"))?;
	*rr = (i + 1) % n;
	Ok(())
}

fn spawn_store_submit_workers(
	num_workers: usize,
	pool: &[Arc<OnlineClient<BulletinConfig>>],
	ws_urls_owned: &[String],
	stats: Arc<Mutex<SubmitStats>>,
	new_block_notify: Arc<Notify>,
) -> (Vec<mpsc::Sender<StoreWorkMsg>>, Vec<tokio::task::JoinHandle<Result<()>>>) {
	let per_worker_cap = (WORK_CHANNEL_CAPACITY / num_workers.max(1)).max(32);
	let mut txs = Vec::with_capacity(num_workers);
	let mut handles = Vec::with_capacity(num_workers);

	for worker_id in 0..num_workers {
		let (tx, mut rx) = mpsc::channel::<StoreWorkMsg>(per_worker_cap);
		txs.push(tx);

		let stats = stats.clone();
		let new_block_notify = new_block_notify.clone();
		let reconnect_url = ws_urls_owned[worker_id % ws_urls_owned.len()].clone();
		let mut worker_client = pool[worker_id].clone();

		handles.push(tokio::spawn(async move {
			let mut consecutive_conn_errors = 0u32;
			while let Some((account_id, extrinsic)) = rx.recv().await {
				submit_one_store_single_connection(
					worker_id,
					account_id,
					extrinsic,
					&mut worker_client,
					&reconnect_url,
					&mut consecutive_conn_errors,
					&stats,
					&new_block_notify,
				)
				.await?;
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
/// worker owns one connection); count is at least `max(1, 8)` for nonce-init fan-out.
#[allow(clippy::too_many_arguments)]
pub async fn run_block_capacity_pipeline(
	mut work_rx: mpsc::Receiver<StressWorkItem>,
	dual: DualBlockSubscription,
	ws_urls: &[&str],
	submitters: usize,
	payload_size: usize,
	client: &OnlineClient<BulletinConfig>,
	authorizer: &Keypair,
	authorizer_nonce_tracker: &NonceTracker,
) -> Result<BulkStoreResult> {
	const TX_TIMEOUT_SECS: u64 = 60;

	let fork_detections = Arc::new(AtomicU64::new(0));
	let new_block_notify = Arc::new(Notify::new());
	let block_stats = Arc::new(Mutex::new(Vec::<BlockStats>::new()));
	let measure_start = Arc::new(Mutex::new(None::<Instant>));
	let monitor_ready = Arc::new(Notify::new());

	let monitor_handle = spawn_pipeline_dual_monitor(
		dual,
		payload_size,
		fork_detections.clone(),
		new_block_notify.clone(),
		block_stats.clone(),
		measure_start.clone(),
		monitor_ready.clone(),
	);

	monitor_ready.notified().await;
	log::info!("pipeline: block monitor ready, starting work reader + store workers");

	let num_connections = submitters.max(1).max(8);

	let mut pool = Vec::with_capacity(num_connections);
	for i in 0..num_connections {
		let url = ws_urls[i % ws_urls.len()];
		pool.push(Arc::new(crate::client::connect(url).await?));
	}

	log::info!("pipeline: {num_connections} store worker(s), each with a dedicated RPC connection");

	let submit_stats = Arc::new(Mutex::new(SubmitStats::default()));
	let ws_urls_owned: Vec<String> = ws_urls.iter().map(|s| s.to_string()).collect();

	let (worker_txs, worker_handles) = spawn_store_submit_workers(
		num_connections,
		&pool,
		&ws_urls_owned,
		submit_stats.clone(),
		new_block_notify.clone(),
	);

	let start = Instant::now();

	let mut dbg_work_auth = 0u64;
	let mut dbg_work_store = 0u64;

	let mut store_worker_rr: usize = 0;
	let mut stores_dispatched_since_txpool: u64 = 0;

	// Background authorization task handle (interleaved with store dispatch).
	let mut pending_auth: Option<tokio::task::JoinHandle<Result<()>>> = None;

	loop {
		if stores_dispatched_since_txpool >= POOL_PENDING_PAUSE_THRESHOLD as u64 {
			wait_until_txpool_can_pull_work(ws_urls[0]).await;
			stores_dispatched_since_txpool = 0;
		}

		let item = work_rx.recv().await;
		let Some(item) = item else {
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

				// Spawn authorization as a background task so the reader can
				// continue dispatching Store items from the previous batch.
				let task_client = client.clone();
				let task_authorizer = authorizer.clone();
				let task_nonce = authorizer_nonce_tracker.clone();
				let task_payload_size = payload_size;
				pending_auth = Some(tokio::spawn(async move {
					for account_ids in batches {
						authorize::authorize_account_batch(
							&task_client,
							&task_authorizer,
							&task_nonce,
							&account_ids,
							1,
							(task_payload_size + 1024) as u64,
						)
						.await?;
					}
					Ok(())
				}));
			},
			StressWorkItem::AwaitPendingAuth =>
				if let Some(handle) = pending_auth.take() {
					handle.await.map_err(|e| anyhow::anyhow!("auth task join: {e}"))??;
					log::debug!("pipeline: AwaitPendingAuth completed (auth #{dbg_work_auth})");
				},
			StressWorkItem::Store { account_id, extrinsic } => {
				dispatch_store_to_workers(account_id, extrinsic, &worker_txs, &mut store_worker_rr)
					.await?;
				stores_dispatched_since_txpool += 1;
				dbg_work_store += 1;
				if dbg_work_store == 1 || dbg_work_store.is_multiple_of(256) {
					let sub = submit_stats.lock().unwrap().submitted;
					let conf: u64 = block_stats.lock().unwrap().iter().map(|b| b.tx_count).sum();
					log::debug!(
						"pipeline: Store #{dbg_work_store} dispatched (rpc_submitted={sub} \
						 confirmed_store_events_in_stats={conf})"
					);
				}
			},
		}
	}

	// Await any trailing auth task.
	if let Some(handle) = pending_auth.take() {
		handle.await.map_err(|e| anyhow::anyhow!("trailing auth task join: {e}"))??;
	}

	log::info!("pipeline: work stream finished, closing store worker inputs");
	drop(worker_txs);

	for join_res in join_all(worker_handles).await {
		join_res.map_err(|e| anyhow::anyhow!("store worker task join: {e}"))??;
	}

	{
		let mut ms = measure_start.lock().unwrap();
		if ms.is_none() {
			*ms = Some(start);
		}
	}

	if submit_stats.lock().unwrap().submitted > 0 {
		let deadline = Instant::now() + Duration::from_secs(TX_TIMEOUT_SECS * 3);
		loop {
			let confirmed: u64 = block_stats.lock().unwrap().iter().map(|b| b.tx_count).sum();
			let sub = submit_stats.lock().unwrap().submitted;
			if confirmed >= sub {
				break;
			}
			if Instant::now() > deadline {
				log::warn!(
					"pipeline: confirmation wait deadline ({}) — confirmed={confirmed} submitted={sub}",
					TX_TIMEOUT_SECS * 3,
				);
				break;
			}
			tokio::time::sleep(Duration::from_millis(500)).await;
		}
	}

	new_block_notify.notify_waiters();
	monitor_handle.abort();
	let _ = monitor_handle.await;

	let duration = measure_start
		.lock()
		.unwrap()
		.map(|ms| ms.elapsed())
		.unwrap_or_else(|| start.elapsed());
	let total_wall = start.elapsed();
	let ss = submit_stats.lock().unwrap();
	let total_submitted = ss.submitted;
	let total_errors = ss.errors;
	let total_pool_full = ss.pool_full_retries;
	let total_stale = ss.stale_nonces;
	drop(ss);
	let all_blocks = block_stats.lock().unwrap().clone();
	let total_confirmed: u64 = all_blocks.iter().map(|b| b.tx_count).sum();
	let fork_detections = fork_detections.load(Ordering::Relaxed);

	log::info!(
		"pipeline: DONE — wall={:.1}s, submitted={total_submitted}, confirmed={total_confirmed}, \
		 errors={total_errors}",
		total_wall.as_secs_f64(),
	);
	log::debug!(
		"pipeline: DONE detail — pool_full_retries={total_pool_full} stale_nonces={total_stale} \
		 fork_detections={fork_detections}"
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
		fork_detections,
	})
}

/// Sign [`StressWorkItem::Store`] items for `keypairs` (does not send them). Runs up to
/// [`STORE_SIGN_PARALLELISM`] tasks concurrently via `buffer_unordered`. Each task runs payload
/// generation, SCALE encoding, and sr25519 signing in a single [`tokio::task::spawn_blocking`]
/// call, keeping all CPU work off the async runtime.
async fn build_store_work_items(
	client: &OnlineClient<BulletinConfig>,
	keypairs: &[Keypair],
	payload_size: usize,
) -> Result<Vec<StressWorkItem>> {
	stream::iter(keypairs.iter().cloned())
		.map(|kp| {
			let client = client.clone();
			async move {
				let (account_id, encoded) = tokio::task::spawn_blocking(move || {
					let payload = crate::store::generate_payload(payload_size);
					let encoded = sign_store_extrinsic_blocking(&client, &kp, &payload, 0)?;
					Ok::<_, anyhow::Error>((kp.public_key().to_account_id(), encoded))
				})
				.await
				.map_err(|e| anyhow::anyhow!("pipeline: spawn_blocking join: {e}"))??;
				Ok::<_, anyhow::Error>(StressWorkItem::Store {
					account_id,
					extrinsic: Arc::new(encoded),
				})
			}
		})
		.buffer_unordered(STORE_SIGN_PARALLELISM)
		.try_collect()
		.await
}

/// Push work items for iterative block-capacity runs (continuous production).
///
/// Work items are **interleaved** so that authorization of batch N+1 runs concurrently with store
/// dispatch of batch N:
///
/// ```text
/// Authorize(0), AwaitPendingAuth,
/// Authorize(1), Store(0)…, AwaitPendingAuth,
/// Authorize(2), Store(1)…, AwaitPendingAuth,
/// …
/// Store(last)…
/// ```
///
/// The reader spawns each `Authorize` as a background task and only blocks on
/// `AwaitPendingAuth`, so stores from the previous batch flow to workers while the next batch's
/// authorization waits for block inclusion.
pub async fn generate_block_capacity_work(
	work_tx: mpsc::Sender<StressWorkItem>,
	plans: &[IterationPlan],
	payload_size: usize,
	client: Arc<OnlineClient<BulletinConfig>>,
) -> Result<()> {
	if plans.is_empty() || plans.iter().all(|p| p.account_count == 0) {
		return Ok(());
	}

	let mut prev_store_items: Option<Vec<StressWorkItem>> = None;
	let mut is_first_batch = true;

	for (iter_idx, plan) in plans.iter().enumerate() {
		if plan.account_count == 0 {
			continue;
		}

		let ni = plan.account_count;
		log::info!(
			"pipeline: block-capacity iteration {} of {} starting ({ni} accounts)",
			iter_idx + 1,
			plans.len(),
		);

		let mut batch_start = 0u32;
		while batch_start < ni {
			let batch_end =
				batch_start.saturating_add(AUTHORIZE_BATCH_SIZE as u32).min(plan.account_count);

			let keypairs = keypairs_for_range(plan, batch_start, batch_end);
			let auth_batches = auth_batches_from_keypairs(&keypairs);
			let store_items = build_store_work_items(&client, &keypairs, payload_size).await?;

			// Send auth for this batch (reader spawns as background task).
			work_tx
				.send(StressWorkItem::Authorize { batches: auth_batches })
				.await
				.map_err(|_| anyhow::anyhow!("pipeline work channel closed (auth)"))?;

			if is_first_batch {
				// First batch: reader must await auth before any stores.
				work_tx
					.send(StressWorkItem::AwaitPendingAuth)
					.await
					.map_err(|_| anyhow::anyhow!("pipeline work channel closed (await)"))?;
				is_first_batch = false;
			}

			// Dispatch previous batch's stores (reader processes them while
			// this batch's authorization runs in the background).
			if let Some(prev_items) = prev_store_items.take() {
				for item in prev_items {
					work_tx
						.send(item)
						.await
						.map_err(|_| anyhow::anyhow!("pipeline work channel closed (store)"))?;
				}
				// Barrier: reader must await this batch's auth before its stores.
				work_tx
					.send(StressWorkItem::AwaitPendingAuth)
					.await
					.map_err(|_| anyhow::anyhow!("pipeline work channel closed (await)"))?;
			}

			prev_store_items = Some(store_items);
			batch_start = batch_end;
		}
	}

	// Send final batch's stores (auth already awaited by the last AwaitPendingAuth).
	if let Some(last_items) = prev_store_items.take() {
		for item in last_items {
			work_tx
				.send(item)
				.await
				.map_err(|_| anyhow::anyhow!("pipeline work channel closed (store)"))?;
		}
	}

	Ok(())
}
