use anyhow::{anyhow, Result};
use std::{
	sync::{
		atomic::{AtomicBool, AtomicU64, Ordering},
		Arc,
	},
	time::{Duration, Instant},
};
use subxt::OnlineClient;

use crate::{
	client::BulletinConfig,
	fetch::{connect_fetchers, FetchMode, ReadSource},
	metrics::{metrics, LatencyKind},
	report::{compute_latency_stats, ScenarioResult},
	scenarios::bitswap_bulk_read::discover_all_items,
};

const VARIANT: &str = "bitswap-verify-all";
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const PROGRESS_EVERY_ITEMS: u64 = 100;

struct Problem {
	cid: cid::Cid,
	block_number: u64,
	reason: String,
}

pub async fn run_verify_all(
	client: &OnlineClient<BulletinConfig>,
	source: ReadSource<'_>,
	concurrency: usize,
	cancel: Arc<AtomicBool>,
) -> Result<ScenarioResult> {
	let items = discover_all_items(client).await?;
	if items.is_empty() {
		anyhow::bail!("No CIDs found in TransactionStorage");
	}
	let total_items = items.len() as u64;
	let known_bytes: u64 = items.iter().map(|item| item.size as u64).sum();
	let transport = source.transport();

	tracing::info!(
		"Verify-all: {total_items} CIDs on chain ({} MB), concurrency={concurrency}, \
		 transport={transport}, endpoints={}",
		known_bytes / (1024 * 1024),
		source.endpoints(),
	);

	let workers = connect_fetchers(&source, concurrency, FetchMode::Raw).await?;

	let work = Arc::new(items);
	let next_idx = Arc::new(AtomicU64::new(0));
	let processed = Arc::new(AtomicU64::new(0));
	let bytes_verified = Arc::new(AtomicU64::new(0));
	let failed = Arc::new(AtomicU64::new(0));
	let wall_start = Instant::now();

	let mut handles = Vec::with_capacity(workers.len());
	for (worker_idx, fetcher) in workers.into_iter().enumerate() {
		let work = Arc::clone(&work);
		let next_idx = Arc::clone(&next_idx);
		let cancel = Arc::clone(&cancel);
		let processed = Arc::clone(&processed);
		let bytes_verified = Arc::clone(&bytes_verified);
		let failed = Arc::clone(&failed);

		handles.push(tokio::spawn(async move {
			let mut durations: Vec<Duration> = Vec::new();
			let mut problems: Vec<Problem> = Vec::new();

			loop {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				let idx = next_idx.fetch_add(1, Ordering::Relaxed) as usize;
				if idx >= work.len() {
					break;
				}
				let item = &work[idx];

				let start = Instant::now();
				match fetcher.fetch_verified_block(item.cid, FETCH_TIMEOUT).await {
					Ok(data) => {
						let elapsed = start.elapsed();
						bytes_verified.fetch_add(data.len() as u64, Ordering::Relaxed);
						durations.push(elapsed);
						metrics().inc_reads(VARIANT, true, 1, data.len() as u64);
						metrics().observe_latency(VARIANT, LatencyKind::Retrieval, elapsed);
					},
					Err(reason) => {
						failed.fetch_add(1, Ordering::Relaxed);
						metrics().inc_reads(VARIANT, false, 1, 0);
						tracing::warn!("Worker {worker_idx}: {} (item #{idx}): {reason}", item.cid);
						problems.push(Problem {
							cid: item.cid,
							block_number: item.block_number,
							reason,
						});
					},
				}

				let done = processed.fetch_add(1, Ordering::Relaxed) + 1;
				if done.is_multiple_of(PROGRESS_EVERY_ITEMS) || done == total_items {
					let secs = wall_start.elapsed().as_secs_f64().max(0.001);
					tracing::info!(
						"[{:5.1}%] verified {done}/{total_items}, {:.1} MB, {} failed, \
						 {:.1} items/s",
						done as f64 / total_items as f64 * 100.0,
						bytes_verified.load(Ordering::Relaxed) as f64 / 1048576.0,
						failed.load(Ordering::Relaxed),
						done as f64 / secs,
					);
				}
			}
			(durations, problems)
		}));
	}

	let mut all_durations = Vec::new();
	let mut problems = Vec::new();
	for handle in handles {
		let (durations, worker_problems) =
			handle.await.map_err(|error| anyhow!("task panicked: {error}"))?;
		all_durations.extend(durations);
		problems.extend(worker_problems);
	}

	let wall_time = wall_start.elapsed();
	let verified_items = all_durations.len() as u64;
	let failed_items = problems.len() as u64;
	let nbytes = bytes_verified.load(Ordering::Relaxed);
	let attempted = processed.load(Ordering::Relaxed);
	let all_ok = !cancel.load(Ordering::Relaxed) && verified_items == total_items;

	tracing::info!(
		"Verify-all: {verified_items}/{total_items} items verified ({} MB), \
		 {failed_items} failed, wall={:.1}s, {}",
		nbytes / (1024 * 1024),
		wall_time.as_secs_f64(),
		if all_ok { "ALL VERIFIED OK" } else { "FAILED" },
	);
	if attempted < total_items {
		tracing::warn!("Verify-all: only {attempted}/{total_items} items attempted (cancelled?)");
	}

	if problems.is_empty() {
		tracing::info!("Verify-all: no problematic CIDs");
	} else {
		tracing::error!("Verify-all: {failed_items} CID(s) with problems:");
		for problem in &problems {
			tracing::error!(
				"  {} (block #{}): {}",
				problem.cid,
				problem.block_number,
				problem.reason
			);
		}
	}

	Ok(ScenarioResult {
		name: format!("Verify-all {transport} ({total_items} CIDs, {} MB)", nbytes / (1024 * 1024)),
		variant: VARIANT.to_string(),
		duration: wall_time,
		payload_size: nbytes as usize,
		retrieval_latency: compute_latency_stats(&mut all_durations),
		total_reads: Some(total_items),
		successful_reads: Some(verified_items),
		failed_reads: Some(failed_items),
		reads_per_sec: Some(verified_items as f64 / wall_time.as_secs_f64()),
		read_bytes_per_sec: Some(nbytes as f64 / wall_time.as_secs_f64()),
		data_verified: Some(all_ok),
		..Default::default()
	})
}
