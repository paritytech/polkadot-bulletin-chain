// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use std::{
	sync::{
		atomic::{AtomicBool, AtomicU64, Ordering},
		Arc,
	},
	time::{Duration, Instant},
};
use subxt_signer::sr25519::Keypair;
use tokio::sync::Mutex;

use crate::{
	client,
	hop::{self, RecipientKeypair},
	metrics::{metrics, LatencyKind},
	report::{self, ScenarioResult},
};

/// Payload sizes for the submit sweep.
const SUBMIT_PAYLOAD_SIZES: &[(usize, &str)] = &[
	(1024, "1KB"),
	(10 * 1024, "10KB"),
	(100 * 1024, "100KB"),
	(128 * 1024, "128KB"),
	(256 * 1024, "256KB"),
	(512 * 1024, "512KB"),
	(1024 * 1024, "1MB"),
	(2 * 1024 * 1024, "2MB"),
];

// Disjoint payload index spaces so scenarios run back-to-back without colliding
// on `DuplicateEntry` (HOP pool deduplicates by content hash).
const FULL_CYCLE_INDEX_BASE: u64 = 100_000_000;
const GROUP_INDEX_BASE: u64 = 200_000_000;
const POOL_FILL_INDEX_BASE: u64 = 300_000_000;

/// Safety cap on entries submitted to a single node by `pool-fill`.
const POOL_FILL_MAX_ENTRIES_PER_NODE: u64 = 100_000;
const MIXED_INDEX_BASE: u64 = 400_000_000;

/// Connect attempts before a worker stops using a node.
const CONNECT_ATTEMPTS: usize = 5;

/// Backoff range for a full pool. The pool frees space only when entries are acked or reach
/// `--hop-retention-secs`. Retrying without a delay re-sends payloads that the node rejects,
/// which wastes bandwidth. The maximum stays below the retention period so a worker resumes
/// soon after entries start to expire.
const POOL_FULL_BACKOFF_START: Duration = Duration::from_secs(1);
const POOL_FULL_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Submit failure categories that the fill loops handle differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmitFailure {
	/// The node pool is full. No account can submit until the pool frees space.
	PoolFull,
	/// This account used up its byte quota on this node. Other accounts still have quota.
	QuotaExceeded,
	/// The rate limit for this account is exceeded. Other accounts can submit now.
	RateLimited,
	/// The connection failed. Connect again before the next submit.
	Transport,
	Other,
}

fn classify_submit_error(err: &anyhow::Error) -> SubmitFailure {
	match hop::error_code(err) {
		Some(hop::HOP_ERR_POOL_FULL) => SubmitFailure::PoolFull,
		Some(hop::HOP_ERR_USER_QUOTA) => SubmitFailure::QuotaExceeded,
		Some(hop::HOP_ERR_RATE_LIMITED) => SubmitFailure::RateLimited,
		_ =>
			if hop::is_transport_error(err) {
				SubmitFailure::Transport
			} else {
				SubmitFailure::Other
			},
	}
}

/// Returns the next account with quota remaining on this node, searching from `from + 1` and
/// wrapping around. `from` is checked last, so the function returns `from` when it is the only
/// account with quota left. Returns `None` when all accounts used up their quota.
fn next_live_submitter(retired: &[bool], from: usize) -> Option<usize> {
	(1..=retired.len())
		.map(|off| (from + off) % retired.len())
		.find(|&i| !retired[i])
}

// ---------------------------------------------------------------------------
// S1: Submit throughput
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn run_submit_throughput(
	ws_urls: &[&str],
	items: u32,
	payload_size: usize,
	concurrency: usize,
	submitter: &Keypair,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	tracing::info!(
		"S1: Submit throughput — {} items × {} bytes, concurrency {}, {} collator(s)",
		items,
		payload_size,
		concurrency,
		ws_urls.len(),
	);

	let total_submitted = Arc::new(AtomicU64::new(0));
	let total_errors = Arc::new(AtomicU64::new(0));
	let total_bytes = Arc::new(AtomicU64::new(0));
	let latencies = Arc::new(Mutex::new(Vec::<Duration>::new()));

	let items_per_stream = (items as usize).div_ceil(concurrency);

	let start = Instant::now();

	let mut handles = Vec::new();
	for stream_idx in 0..concurrency {
		let url = ws_urls[stream_idx % ws_urls.len()].to_string();
		let range_start = stream_idx * items_per_stream;
		let range_end = ((stream_idx + 1) * items_per_stream).min(items as usize);
		let submitted = total_submitted.clone();
		let errors = total_errors.clone();
		let bytes = total_bytes.clone();
		let lats = latencies.clone();
		let cancel = cancel.clone();
		let submitter = submitter.clone();

		handles.push(tokio::spawn(async move {
			let ws = match client::connect_ws(&url).await {
				Ok(ws) => ws,
				Err(e) => {
					tracing::error!("Failed to connect to {url}: {e}");
					return;
				},
			};

			for i in range_start..range_end {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				let data = hop::generate_payload(payload_size, i as u64);
				let recipients = vec![RecipientKeypair::generate()];

				match hop::hop_submit(&ws, &data, &recipients, &submitter).await {
					Ok((_hash, _result, latency)) => {
						submitted.fetch_add(1, Ordering::Relaxed);
						bytes.fetch_add(payload_size as u64, Ordering::Relaxed);
						lats.lock().await.push(latency);
					},
					Err(e) => {
						errors.fetch_add(1, Ordering::Relaxed);
						let err_count = errors.load(Ordering::Relaxed);
						if err_count <= 5 {
							tracing::warn!("submit error [{i}]: {e}");
						}
					},
				}
			}
		}));
	}

	for h in handles {
		let _ = h.await;
	}
	let duration = start.elapsed();

	let submitted = total_submitted.load(Ordering::Relaxed);
	let errors = total_errors.load(Ordering::Relaxed);
	let bytes = total_bytes.load(Ordering::Relaxed);
	let mut lats = latencies.lock().await;

	let tps =
		if duration.as_secs_f64() > 0.0 { submitted as f64 / duration.as_secs_f64() } else { 0.0 };

	let label = format_payload_label(payload_size);
	let variant = format!("hop-submit-{label}");
	metrics().observe_latencies(&variant, LatencyKind::Inclusion, &lats);

	let result = ScenarioResult {
		name: format!("HOP submit {label}"),
		variant,
		duration,
		total_submitted: submitted,
		total_confirmed: submitted,
		total_errors: errors,
		payload_size,
		throughput_tps: tps,
		throughput_bytes_per_sec: bytes as f64 / duration.as_secs_f64(),
		inclusion_latency: report::compute_latency_stats(&mut lats),
		..Default::default()
	};

	results.push(result);
	on_result(results);

	// Print pool status
	if let Ok(ws) = client::connect_ws(ws_urls[0]).await {
		if let Ok(status) = hop::hop_pool_status(&ws).await {
			tracing::info!(
				"Pool: {} entries, {} / {} bytes",
				status.entry_count,
				status.total_bytes,
				status.max_bytes
			);
		}
	}

	Ok(())
}

// ---------------------------------------------------------------------------
// S2: Full cycle (submit + claim)
// ---------------------------------------------------------------------------

struct SubmittedEntry {
	hash: [u8; 32],
	data: Vec<u8>,
	recipients: Vec<RecipientKeypair>,
	collator_url: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_full_cycle(
	ws_urls: &[&str],
	items: u32,
	payload_size: usize,
	concurrency: usize,
	submitter: &Keypair,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	tracing::info!(
		"S2: Full cycle — {} items × {} bytes, concurrency {}",
		items,
		payload_size,
		concurrency,
	);

	let entries = Arc::new(Mutex::new(Vec::<SubmittedEntry>::new()));
	let submit_lats = Arc::new(Mutex::new(Vec::<Duration>::new()));
	let submit_errors = Arc::new(AtomicU64::new(0));

	let items_per_stream = (items as usize).div_ceil(concurrency);
	let start = Instant::now();

	// Submit phase
	let mut handles = Vec::new();
	for stream_idx in 0..concurrency {
		let url = ws_urls[stream_idx % ws_urls.len()].to_string();
		let range_start = stream_idx * items_per_stream;
		let range_end = ((stream_idx + 1) * items_per_stream).min(items as usize);
		let entries = entries.clone();
		let lats = submit_lats.clone();
		let errors = submit_errors.clone();
		let cancel = cancel.clone();
		let submitter = submitter.clone();

		handles.push(tokio::spawn(async move {
			let ws = match client::connect_ws(&url).await {
				Ok(ws) => ws,
				Err(e) => {
					tracing::error!("Failed to connect to {url}: {e}");
					return;
				},
			};
			for i in range_start..range_end {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				let data = hop::generate_payload(payload_size, FULL_CYCLE_INDEX_BASE + i as u64);
				let recipients = vec![RecipientKeypair::generate()];
				match hop::hop_submit(&ws, &data, &recipients, &submitter).await {
					Ok((hash, _result, latency)) => {
						lats.lock().await.push(latency);
						entries.lock().await.push(SubmittedEntry {
							hash,
							data,
							recipients,
							collator_url: url.clone(),
						});
					},
					Err(e) => {
						errors.fetch_add(1, Ordering::Relaxed);
						if errors.load(Ordering::Relaxed) <= 5 {
							tracing::warn!("submit error [{i}]: {e}");
						}
					},
				}
			}
		}));
	}
	for h in handles {
		let _ = h.await;
	}

	let submit_duration = start.elapsed();
	let mut submit_lats = submit_lats.lock().await;
	let submitted = entries.lock().await.len() as u64;
	tracing::info!(
		"Submit phase done: {submitted} entries in {:.1}s",
		submit_duration.as_secs_f64()
	);

	// Claim phase
	let claim_start = Instant::now();
	let mut claim_lats = Vec::new();
	let mut claim_errors = 0u64;
	let mut claim_bytes = 0u64;

	let entries_guard = entries.lock().await;
	for entry in entries_guard.iter() {
		if cancel.load(Ordering::Relaxed) {
			break;
		}
		let ws = client::connect_ws(&entry.collator_url).await?;
		for kp in &entry.recipients {
			match hop::hop_claim(&ws, &entry.hash, kp).await {
				Ok((data, latency)) => {
					if data != entry.data {
						tracing::error!("Data mismatch! hash=0x{}", hex::encode(&entry.hash[..8]));
					}
					claim_lats.push(latency);
					claim_bytes += data.len() as u64;
					// A claim only reads the entry. The node removes it after every
					// recipient acks. Without the ack, the entry stays in the pool and
					// counts against the submitter's byte quota until it expires.
					if let Err(e) = hop::hop_ack(&ws, &entry.hash, kp).await {
						claim_errors += 1;
						if claim_errors <= 5 {
							tracing::warn!("ack error: {e}");
						}
					}
				},
				Err(e) => {
					claim_errors += 1;
					if claim_errors <= 5 {
						tracing::warn!("claim error: {e}");
					}
				},
			}
		}
	}
	let claim_duration = claim_start.elapsed();
	let total_duration = start.elapsed();

	let claimed = claim_lats.len() as u64;
	let claim_tps = if claim_duration.as_secs_f64() > 0.0 {
		claimed as f64 / claim_duration.as_secs_f64()
	} else {
		0.0
	};

	let variant = "hop-full-cycle";
	metrics().observe_latencies(variant, LatencyKind::Inclusion, &submit_lats);
	metrics().observe_latencies(variant, LatencyKind::Retrieval, &claim_lats);

	let result = ScenarioResult {
		name: format!("HOP full-cycle {}", format_payload_label(payload_size)),
		variant: variant.into(),
		duration: total_duration,
		total_submitted: submitted,
		total_confirmed: claimed,
		total_errors: submit_errors.load(Ordering::Relaxed) + claim_errors,
		payload_size,
		throughput_tps: claim_tps,
		throughput_bytes_per_sec: claim_bytes as f64 / claim_duration.as_secs_f64(),
		inclusion_latency: report::compute_latency_stats(&mut submit_lats),
		retrieval_latency: report::compute_latency_stats(&mut claim_lats),
		..Default::default()
	};

	results.push(result);
	on_result(results);
	Ok(())
}

// ---------------------------------------------------------------------------
// S3: Group recipients
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn run_group(
	ws_urls: &[&str],
	items: u32,
	payload_size: usize,
	num_recipients: usize,
	submitter: &Keypair,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	tracing::info!(
		"S3: Group — {} items × {} bytes, {} recipients each",
		items,
		payload_size,
		num_recipients,
	);

	let ws = client::connect_ws(ws_urls[0]).await?;
	let mut submitted = Vec::new();
	let mut submit_lats = Vec::new();

	// Submit
	for i in 0..items {
		if cancel.load(Ordering::Relaxed) {
			break;
		}
		let data = hop::generate_payload(payload_size, GROUP_INDEX_BASE + i as u64);
		let recipients: Vec<RecipientKeypair> =
			(0..num_recipients).map(|_| RecipientKeypair::generate()).collect();

		match hop::hop_submit(&ws, &data, &recipients, submitter).await {
			Ok((hash, _result, latency)) => {
				submit_lats.push(latency);
				submitted.push(SubmittedEntry {
					hash,
					data,
					recipients,
					collator_url: ws_urls[0].to_string(),
				});
			},
			Err(e) => {
				tracing::warn!("submit error [{i}]: {e}");
			},
		}
	}

	// Parallel claim: all recipients claim concurrently per entry
	let claim_start = Instant::now();
	let claim_lats = Arc::new(Mutex::new(Vec::<Duration>::new()));
	let claim_errors = Arc::new(AtomicU64::new(0));
	let claim_bytes = Arc::new(AtomicU64::new(0));

	for entry in &submitted {
		if cancel.load(Ordering::Relaxed) {
			break;
		}
		let mut handles = Vec::new();
		for kp in &entry.recipients {
			let url = entry.collator_url.clone();
			let hash = entry.hash;
			let expected_len = entry.data.len();
			let kp = kp.clone();
			let lats = claim_lats.clone();
			let errors = claim_errors.clone();
			let bytes = claim_bytes.clone();

			handles.push(tokio::spawn(async move {
				let ws = match client::connect_ws(&url).await {
					Ok(ws) => ws,
					Err(_) => return,
				};
				match hop::hop_claim(&ws, &hash, &kp).await {
					Ok((data, latency)) => {
						lats.lock().await.push(latency);
						bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
						if data.len() != expected_len {
							tracing::error!("Data length mismatch in group claim");
						}
						// The node removes the entry only after all recipients ack. In
						// this scenario each spawned task acks for one recipient.
						if let Err(e) = hop::hop_ack(&ws, &hash, &kp).await {
							errors.fetch_add(1, Ordering::Relaxed);
							if errors.load(Ordering::Relaxed) <= 5 {
								tracing::warn!("group ack error: {e}");
							}
						}
					},
					Err(e) => {
						errors.fetch_add(1, Ordering::Relaxed);
						if errors.load(Ordering::Relaxed) <= 5 {
							tracing::warn!("group claim error: {e}");
						}
					},
				}
			}));
		}
		for h in handles {
			let _ = h.await;
		}
	}

	let claim_duration = claim_start.elapsed();
	let mut claim_lats = claim_lats.lock().await;
	let claimed = claim_lats.len() as u64;
	let total_claims_expected = submitted.len() as u64 * num_recipients as u64;

	let claim_tps = if claim_duration.as_secs_f64() > 0.0 {
		claimed as f64 / claim_duration.as_secs_f64()
	} else {
		0.0
	};

	let variant = "hop-group";
	metrics().observe_latencies(variant, LatencyKind::Inclusion, &submit_lats);
	metrics().observe_latencies(variant, LatencyKind::Retrieval, &claim_lats);

	let result = ScenarioResult {
		name: format!("HOP group ×{num_recipients} {}", format_payload_label(payload_size)),
		variant: variant.into(),
		duration: claim_duration,
		total_submitted: submitted.len() as u64,
		total_confirmed: claimed,
		total_errors: claim_errors.load(Ordering::Relaxed),
		payload_size,
		throughput_tps: claim_tps,
		throughput_bytes_per_sec: claim_bytes.load(Ordering::Relaxed) as f64 /
			claim_duration.as_secs_f64(),
		inclusion_latency: report::compute_latency_stats(&mut submit_lats),
		retrieval_latency: report::compute_latency_stats(&mut claim_lats),
		..Default::default()
	};

	tracing::info!(
		"Group: {claimed}/{total_claims_expected} claims OK, {} errors",
		claim_errors.load(Ordering::Relaxed)
	);

	results.push(result);
	on_result(results);
	Ok(())
}

// ---------------------------------------------------------------------------
// S4: Pool fill
// ---------------------------------------------------------------------------

pub async fn run_pool_fill(
	ws_urls: &[&str],
	payload_size: usize,
	submitters: &[Keypair],
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	tracing::info!(
		"S4: Pool fill — {} byte payloads on {} node(s) with {} submitter(s), until PoolFull",
		payload_size,
		ws_urls.len(),
		submitters.len()
	);

	let start = Instant::now();
	let mut submitted = 0u64;
	let mut errors = 0u64;
	let mut total_bytes = 0u64;
	let mut lats = Vec::new();
	let mut any_pool_full = false;

	// The byte quota applies per `(node, submitter)` pair and each node has its own pool, so
	// this loop fills one node at a time and every submitter adds its own quota to that node.
	// UserQuotaExceeded means one account used up its quota on this node, not that the pool
	// is full. Only PoolFull ends the loop for a node.
	for (node_idx, url) in ws_urls.iter().enumerate() {
		if cancel.load(Ordering::Relaxed) {
			break;
		}

		let mut ws = match client::connect_ws_retry(url, CONNECT_ATTEMPTS).await {
			Ok(ws) => ws,
			Err(e) => {
				tracing::warn!("pool-fill: cannot connect to {url}: {e}");
				errors += 1;
				continue;
			},
		};

		if let Ok(status) = hop::hop_pool_status(&ws).await {
			tracing::info!(
				"[{url}] initial pool: {} entries, {} / {} bytes",
				status.entry_count,
				status.total_bytes,
				status.max_bytes
			);
		}

		let mut node_submitted = 0u64;
		let mut sub_idx = 0usize;
		// Accounts that used up their quota on this node. A rate limit switches to another
		// account. Only an exceeded quota marks an account as retired for this node.
		let mut retired = vec![false; submitters.len()];
		let mut node_pool_full = false;
		let mut i = 0u64;
		// Reset per node, so failures on one node do not affect the next node.
		let mut node_errors = 0u64;
		let mut throttled = 0u64;
		// Number of consecutive rate limits since the last successful submit. When it
		// reaches the number of accounts with quota left, every account is rate limited.
		let mut throttle_streak = 0usize;

		loop {
			if cancel.load(Ordering::Relaxed) || i >= POOL_FILL_MAX_ENTRIES_PER_NODE {
				if i >= POOL_FILL_MAX_ENTRIES_PER_NODE {
					tracing::info!("[{url}] hit the {POOL_FILL_MAX_ENTRIES_PER_NODE} entry cap");
				}
				break;
			}

			// Disjoint index space per node so payloads stay unique per pool.
			let index = POOL_FILL_INDEX_BASE + (node_idx as u64) * 1_000_000 + i;
			let data = hop::generate_payload(payload_size, index);
			let recipients = vec![RecipientKeypair::generate()];

			match hop::hop_submit(&ws, &data, &recipients, &submitters[sub_idx]).await {
				Ok((_hash, result, latency)) => {
					submitted += 1;
					node_submitted += 1;
					i += 1;
					total_bytes += payload_size as u64;
					lats.push(latency);
					throttle_streak = 0;

					if node_submitted.is_multiple_of(100) {
						tracing::info!(
							"  [{url}] {} submitted (submitter {}/{}), pool: {} entries, {} / {} bytes",
							node_submitted,
							sub_idx + 1,
							submitters.len(),
							result.pool_status.entry_count,
							result.pool_status.total_bytes,
							result.pool_status.max_bytes
						);
					}
				},
				Err(e) => match classify_submit_error(&e) {
					// The pool for this node is full. Move to the next node.
					SubmitFailure::PoolFull => {
						tracing::info!(
							"[{url}] PoolFull after {node_submitted} entries \
							 ({}/{} submitters retired)",
							retired.iter().filter(|r| **r).count(),
							submitters.len()
						);
						node_pool_full = true;
						any_pool_full = true;
						break;
					},
					// Rate limits apply per account, so switch to another account instead
					// of sleeping. Sleep only after every account with quota left returned
					// a rate limit.
					SubmitFailure::RateLimited => {
						throttled += 1;
						throttle_streak += 1;
						match next_live_submitter(&retired, sub_idx) {
							Some(next) => sub_idx = next,
							None => break,
						}
						let live = retired.iter().filter(|r| !**r).count();
						if throttle_streak >= live {
							throttle_streak = 0;
							tokio::time::sleep(Duration::from_secs(1)).await;
						}
						continue;
					},
					// This account used up its byte quota on this node. Mark it retired and
					// continue with the next account.
					SubmitFailure::QuotaExceeded => {
						retired[sub_idx] = true;
						tracing::info!(
							"[{url}] submitter {}/{} exhausted its quota after \
							 {node_submitted} entries; rotating",
							sub_idx + 1,
							submitters.len()
						);
						match next_live_submitter(&retired, sub_idx) {
							Some(next) => sub_idx = next,
							None => break,
						}
						continue;
					},
					SubmitFailure::Transport => {
						tracing::warn!(
							"[{url}] connection lost after {node_submitted} entries: {e}"
						);
						match client::connect_ws_retry(url, CONNECT_ATTEMPTS).await {
							Ok(fresh) => {
								ws = fresh;
								continue;
							},
							Err(e) => {
								tracing::error!("[{url}] redial failed: {e}; moving to next node");
								errors += 1;
								break;
							},
						}
					},
					SubmitFailure::Other => {
						errors += 1;
						node_errors += 1;
						if node_errors <= 5 {
							tracing::warn!("[{url}] pool-fill submit error [{i}]: {e}");
						}
						if node_errors > 10 {
							tracing::error!("[{url}] too many errors, moving to the next node");
							break;
						}
					},
				},
			}
		}

		if !node_pool_full && retired.iter().all(|r| *r) {
			tracing::warn!(
				"[{url}] all {} submitters exhausted without reaching PoolFull — \
				 more submitters are needed to fill this pool",
				submitters.len()
			);
		}

		if throttled > 0 {
			tracing::info!("[{url}] rate-limited {throttled} time(s) while filling");
		}

		if let Ok(status) = hop::hop_pool_status(&ws).await {
			tracing::info!(
				"[{url}] final pool: {} entries, {} / {} bytes",
				status.entry_count,
				status.total_bytes,
				status.max_bytes
			);
		}
	}

	let duration = start.elapsed();
	let tps =
		if duration.as_secs_f64() > 0.0 { submitted as f64 / duration.as_secs_f64() } else { 0.0 };

	let variant = "hop-pool-fill";
	metrics().observe_latencies(variant, LatencyKind::Inclusion, &lats);

	let result = ScenarioResult {
		name: format!(
			"HOP pool-fill {}{}",
			format_payload_label(payload_size),
			if any_pool_full { " (full)" } else { "" }
		),
		variant: variant.into(),
		duration,
		total_submitted: submitted,
		total_confirmed: submitted,
		total_errors: errors,
		payload_size,
		throughput_tps: tps,
		throughput_bytes_per_sec: total_bytes as f64 / duration.as_secs_f64(),
		inclusion_latency: report::compute_latency_stats(&mut lats),
		..Default::default()
	};

	result.print_text();

	results.push(result);
	on_result(results);
	Ok(())
}

// S5: Mixed read/write
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn run_mixed(
	ws_urls: &[&str],
	payload_size: usize,
	concurrency: usize,
	writers: Option<usize>,
	ack_ratio: f64,
	duration_secs: u64,
	submitters: &[Keypair],
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	// Each writer submits to one node, `ws_urls[w_idx % len]`. Each reader uses the URL
	// recorded with the entry. The scenario covers every node only with at least one writer
	// per node.
	let writer_count = writers.unwrap_or_else(|| std::cmp::max(1, concurrency / 2)).max(1);
	let reader_count = std::cmp::max(1, concurrency.saturating_sub(writer_count));

	// An ack removes the entry. At ratio 1.0 the submit rate and the removal rate are equal,
	// so the pool stays at 0 entries for any writer count. A ratio below 1.0 leaves the
	// remaining entries in the pool until `--hop-retention-secs` expires them, which is what
	// increases pool size.
	let ack_ratio = ack_ratio.clamp(0.0, 1.0);
	let ack_permille = (ack_ratio * 1000.0).round() as u64;

	tracing::info!(
		"S5: Mixed — {} byte payloads, {} writer(s) / {} reader(s) over {} node(s) with {} \
		 submitter(s), acking {:.0}% of claims, {}s duration",
		payload_size,
		writer_count,
		reader_count,
		ws_urls.len(),
		submitters.len(),
		ack_ratio * 100.0,
		duration_secs,
	);

	if writer_count < ws_urls.len() {
		tracing::warn!(
			"{} writer(s) for {} node(s): nodes {}.. will receive no submissions",
			writer_count,
			ws_urls.len(),
			writer_count
		);
	}

	let deadline = Instant::now() + Duration::from_secs(duration_secs);

	// Shared queue: writers push, readers pop
	let pending = Arc::new(Mutex::new(Vec::<SubmittedEntry>::new()));

	let submit_count = Arc::new(AtomicU64::new(0));
	let submit_errors = Arc::new(AtomicU64::new(0));
	let submit_bytes = Arc::new(AtomicU64::new(0));
	let submit_lats = Arc::new(Mutex::new(Vec::<Duration>::new()));

	let claim_count = Arc::new(AtomicU64::new(0));
	let claim_errors = Arc::new(AtomicU64::new(0));
	let ack_count = Arc::new(AtomicU64::new(0));
	let claim_bytes = Arc::new(AtomicU64::new(0));
	let claim_lats = Arc::new(Mutex::new(Vec::<Duration>::new()));

	let writers_done = Arc::new(AtomicBool::new(false));

	let start = Instant::now();

	// Spawn writers
	let mut writer_handles = Vec::new();
	for w_idx in 0..writer_count {
		let url = ws_urls[w_idx % ws_urls.len()].to_string();
		// Each writer submits to one node and the byte quota applies per (node, submitter)
		// pair, so one account covers only --hop-max-user-size on that node. Give every
		// writer all accounts and switch on UserQuotaExceeded, so one writer can fill a pool
		// larger than a single quota.
		let writer_submitters: Vec<Keypair> = submitters.to_vec();
		let pending = pending.clone();
		let count = submit_count.clone();
		let errors = submit_errors.clone();
		let bytes = submit_bytes.clone();
		let lats = submit_lats.clone();
		let cancel = cancel.clone();

		writer_handles.push(tokio::spawn(async move {
			let mut ws = match client::connect_ws_retry(&url, CONNECT_ATTEMPTS).await {
				Ok(ws) => ws,
				Err(e) => {
					tracing::error!("Writer {w_idx} connect failed: {e}");
					return;
				},
			};

			let mut idx = MIXED_INDEX_BASE + (w_idx as u64) * 1_000_000;
			let mut sub_idx = 0usize;
			// A retired account used up its byte quota on this node. A rate limit switches
			// to another account and does not retire any account.
			let mut retired = vec![false; writer_submitters.len()];
			let mut throttle_streak = 0usize;
			let mut pool_full_backoff = POOL_FULL_BACKOFF_START;
			let mut pool_full_logged = false;
			while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
				let data = hop::generate_payload(payload_size, idx);
				let recipients = vec![RecipientKeypair::generate()];
				idx += 1;

				match hop::hop_submit(&ws, &data, &recipients, &writer_submitters[sub_idx]).await {
					Ok((hash, _result, latency)) => {
						count.fetch_add(1, Ordering::Relaxed);
						bytes.fetch_add(payload_size as u64, Ordering::Relaxed);
						lats.lock().await.push(latency);
						throttle_streak = 0;
						pool_full_backoff = POOL_FULL_BACKOFF_START;
						pool_full_logged = false;
						pending.lock().await.push(SubmittedEntry {
							hash,
							data,
							recipients,
							collator_url: url.clone(),
						});
					},
					Err(e) => match classify_submit_error(&e) {
						// The limit applies to the whole node, so switching accounts does
						// not help, and an immediate retry re-sends a payload the node
						// rejects. Wait for acks or expiry to free space, increasing the
						// delay up to POOL_FULL_BACKOFF_MAX.
						SubmitFailure::PoolFull => {
							if !pool_full_logged {
								tracing::warn!(
									"Writer {w_idx} on {url}: pool full, backing off (up to {}s) \
									 until entries are acked or expire",
									POOL_FULL_BACKOFF_MAX.as_secs()
								);
								pool_full_logged = true;
							}
							tokio::time::sleep(pool_full_backoff).await;
							pool_full_backoff = (pool_full_backoff * 2).min(POOL_FULL_BACKOFF_MAX);
							continue;
						},
						// Rate limits apply per account, so another account can submit now.
						// Sleep only after every account with quota left returned a rate
						// limit.
						SubmitFailure::RateLimited => {
							throttle_streak += 1;
							match next_live_submitter(&retired, sub_idx) {
								Some(next) => sub_idx = next,
								None => break,
							}
							let live = retired.iter().filter(|r| !**r).count();
							if throttle_streak >= live {
								throttle_streak = 0;
								tokio::time::sleep(Duration::from_secs(1)).await;
							}
							continue;
						},
						// This account used up its quota on this node. Mark it retired and
						// continue with the next account.
						SubmitFailure::QuotaExceeded => {
							retired[sub_idx] = true;
							match next_live_submitter(&retired, sub_idx) {
								Some(next) => sub_idx = next,
								None => {
									tracing::warn!(
										"Writer {w_idx} on {url}: all {} submitter(s) exhausted \
										 their quota; acks are not releasing bytes fast enough, \
										 or more are needed",
										writer_submitters.len()
									);
									break;
								},
							}
							continue;
						},
						// The node restarted or the connection dropped. Connect again,
						// because every later request on this client fails otherwise.
						SubmitFailure::Transport => {
							tracing::warn!("Writer {w_idx} on {url}: connection lost: {e}");
							match client::connect_ws_retry(&url, CONNECT_ATTEMPTS).await {
								Ok(fresh) => ws = fresh,
								Err(e) => {
									tracing::error!("Writer {w_idx} on {url}: redial failed: {e}");
									errors.fetch_add(1, Ordering::Relaxed);
									break;
								},
							}
							continue;
						},
						SubmitFailure::Other => {
							errors.fetch_add(1, Ordering::Relaxed);
						},
					},
				}
			}
		}));
	}

	// Spawn readers
	let mut reader_handles = Vec::new();
	for _r_idx in 0..reader_count {
		let pending = pending.clone();
		let count = claim_count.clone();
		let errors = claim_errors.clone();
		let acked = ack_count.clone();
		let bytes = claim_bytes.clone();
		let lats = claim_lats.clone();
		let cancel = cancel.clone();
		let writers_done = writers_done.clone();

		reader_handles.push(tokio::spawn(async move {
			// Accumulate the ack ratio in permille and ack when the total reaches 1000.
			// This spreads the acks over the claim sequence instead of acking the first N
			// claims of every 1000.
			let mut ack_credit = 0u64;
			loop {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				let entry = pending.lock().await.pop();
				match entry {
					Some(entry) => {
						let ws =
							match client::connect_ws_retry(&entry.collator_url, CONNECT_ATTEMPTS)
								.await
							{
								Ok(ws) => ws,
								Err(e) => {
									tracing::warn!(
										"Reader cannot reach {}: {e}",
										entry.collator_url
									);
									errors.fetch_add(1, Ordering::Relaxed);
									continue;
								},
							};
						let kp = &entry.recipients[0];
						match hop::hop_claim(&ws, &entry.hash, kp).await {
							Ok((data, latency)) => {
								count.fetch_add(1, Ordering::Relaxed);
								bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
								lats.lock().await.push(latency);
								// Each entry has one recipient, so this ack removes the
								// entry and frees the pool bytes and the per-user quota.
								// Skipping the ack keeps the entry in the pool until
								// `--hop-retention-secs` expires it, which increases pool
								// size.
								ack_credit += ack_permille;
								if ack_credit >= 1000 {
									ack_credit -= 1000;
									if hop::hop_ack(&ws, &entry.hash, kp).await.is_err() {
										errors.fetch_add(1, Ordering::Relaxed);
									} else {
										acked.fetch_add(1, Ordering::Relaxed);
									}
								}
							},
							Err(_) => {
								errors.fetch_add(1, Ordering::Relaxed);
							},
						}
					},
					None => {
						if writers_done.load(Ordering::Relaxed) {
							break;
						}
						tokio::time::sleep(Duration::from_millis(10)).await;
					},
				}
			}
		}));
	}

	// Progress ticker
	let progress_cancel = cancel.clone();
	let s_count = submit_count.clone();
	let c_count = claim_count.clone();
	let a_count = ack_count.clone();
	let s_err = submit_errors.clone();
	let c_err = claim_errors.clone();
	let p_ref = pending.clone();
	let progress = tokio::spawn(async move {
		let mut interval = tokio::time::interval(Duration::from_secs(5));
		loop {
			interval.tick().await;
			if progress_cancel.load(Ordering::Relaxed) {
				break;
			}
			let elapsed = start.elapsed().as_secs_f64();
			let plen = p_ref.lock().await.len();
			// submitted minus acked is the number of entries left in the pools. Both
			// counts show whether the run increases pool size.
			tracing::info!(
				"[{:.0}s] submitted: {}, claimed: {}, acked: {}, pending: {}, errors: {}/{}",
				elapsed,
				s_count.load(Ordering::Relaxed),
				c_count.load(Ordering::Relaxed),
				a_count.load(Ordering::Relaxed),
				plen,
				s_err.load(Ordering::Relaxed),
				c_err.load(Ordering::Relaxed),
			);
		}
	});

	// Wait for writers
	for h in writer_handles {
		let _ = h.await;
	}
	writers_done.store(true, Ordering::Relaxed);

	// Wait for readers to drain
	for h in reader_handles {
		let _ = h.await;
	}
	progress.abort();

	let duration = start.elapsed();
	let submitted = submit_count.load(Ordering::Relaxed);
	let claimed = claim_count.load(Ordering::Relaxed);
	let mut s_lats = submit_lats.lock().await;
	let mut c_lats = claim_lats.lock().await;

	let submit_tps = submitted as f64 / duration.as_secs_f64();
	let claim_tps = claimed as f64 / duration.as_secs_f64();

	let variant = "hop-mixed";
	metrics().observe_latencies(variant, LatencyKind::Inclusion, &s_lats);
	metrics().observe_latencies(variant, LatencyKind::Retrieval, &c_lats);

	let result = ScenarioResult {
		name: format!("HOP mixed {}s {}", duration_secs, format_payload_label(payload_size)),
		variant: variant.into(),
		duration,
		total_submitted: submitted,
		total_confirmed: claimed,
		total_errors: submit_errors.load(Ordering::Relaxed) + claim_errors.load(Ordering::Relaxed),
		payload_size,
		throughput_tps: submit_tps,
		throughput_bytes_per_sec: submit_bytes.load(Ordering::Relaxed) as f64 /
			duration.as_secs_f64(),
		reads_per_sec: Some(claim_tps),
		read_bytes_per_sec: Some(
			claim_bytes.load(Ordering::Relaxed) as f64 / duration.as_secs_f64(),
		),
		total_reads: Some(claimed),
		successful_reads: Some(claimed),
		failed_reads: Some(claim_errors.load(Ordering::Relaxed)),
		inclusion_latency: report::compute_latency_stats(&mut s_lats),
		retrieval_latency: report::compute_latency_stats(&mut c_lats),
		..Default::default()
	};

	results.push(result);
	on_result(results);
	Ok(())
}

// ---------------------------------------------------------------------------
// S6: Error handling
// ---------------------------------------------------------------------------

pub async fn run_error_tests(ws_urls: &[&str], submitter: &Keypair) -> Result<bool> {
	tracing::info!("Error handling tests");

	let ws = client::connect_ws(ws_urls[0]).await?;
	let mut passed = 0u32;
	let mut failed = 0u32;

	macro_rules! expect_submit_error {
		($name:expr, $code:expr, $data:expr, $recipients:expr) => {{
			match hop::hop_submit(&ws, $data, $recipients, submitter).await {
				Err(e) if hop::error_code(&e) == Some($code) => {
					tracing::info!("  PASS: {} (code {})", $name, $code);
					passed += 1;
				},
				Err(e) => {
					tracing::error!(
						"  FAIL: {} — expected {}, got {:?}",
						$name,
						$code,
						hop::error_code(&e)
					);
					failed += 1;
				},
				Ok(_) => {
					tracing::error!("  FAIL: {} — expected error {}, got success", $name, $code);
					failed += 1;
				},
			}
		}};
	}

	macro_rules! expect_claim_error {
		($name:expr, $code:expr, $hash:expr, $kp:expr) => {{
			match hop::hop_claim(&ws, $hash, $kp).await {
				Err(e) if hop::error_code(&e) == Some($code) => {
					tracing::info!("  PASS: {} (code {})", $name, $code);
					passed += 1;
				},
				Err(e) => {
					tracing::error!(
						"  FAIL: {} — expected {}, got {:?}",
						$name,
						$code,
						hop::error_code(&e)
					);
					failed += 1;
				},
				Ok(_) => {
					tracing::error!("  FAIL: {} — expected error {}, got success", $name, $code);
					failed += 1;
				},
			}
		}};
	}

	// 1. Empty data -> 1005
	expect_submit_error!("EmptyData", 1005, &[], &[RecipientKeypair::generate()]);

	// 2. No recipients -> 1011
	let no_recip: &[RecipientKeypair] = &[];
	expect_submit_error!("NoRecipients", 1011, &[1, 2, 3], no_recip);

	// 3. Claim non-existent hash -> 1004
	{
		let fake_hash = [0xABu8; 32];
		let fake_kp = RecipientKeypair::generate();
		expect_claim_error!("NotFound", 1004, &fake_hash, &fake_kp);
	}

	// 4. Claim with wrong keypair -> 1010
	{
		let data = hop::generate_payload(1024, 999_999);
		let valid_kp = RecipientKeypair::generate();
		let wrong_kp = RecipientKeypair::generate();

		match hop::hop_submit(&ws, &data, std::slice::from_ref(&valid_kp), submitter).await {
			Ok((hash, _, _)) => {
				expect_claim_error!("NotRecipient", 1010, &hash, &wrong_kp);
				// Clean up
				let _ = hop::hop_claim(&ws, &hash, &valid_kp).await;
			},
			Err(e) => {
				tracing::warn!("  SKIP: NotRecipient — submit failed: {e}");
			},
		}
	}

	// 5. Duplicate entry -> 1003
	{
		let data = hop::generate_payload(512, 998_998);
		let kp = RecipientKeypair::generate();

		match hop::hop_submit(&ws, &data, &[kp], submitter).await {
			Ok(_) => {
				let kp2 = RecipientKeypair::generate();
				expect_submit_error!("DuplicateEntry", 1003, &data, &[kp2]);
			},
			Err(e) => {
				tracing::warn!("  SKIP: DuplicateEntry — submit failed: {e}");
			},
		}
	}

	// 6. DataTooLarge -> skip (64 MiB impractical over WS)
	tracing::info!("  SKIP: DataTooLarge (65 MiB payload too large for WS transport)");

	// 7. Invalid hash length -> 1008
	{
		let short_hash = [0xCCu8; 16];
		let kp = RecipientKeypair::generate();
		expect_claim_error!("InvalidHashLength", 1008, &short_hash, &kp);
	}

	tracing::info!("Results: {passed} passed, {failed} failed");
	Ok(failed == 0)
}

// ---------------------------------------------------------------------------
// Sweep runner (called from main)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn run_hop_sweep(
	ws_urls: &[&str],
	scenario: &str,
	items: u32,
	payload_size: Option<usize>,
	concurrency: usize,
	num_recipients: usize,
	duration_secs: u64,
	writers: Option<usize>,
	ack_ratio: f64,
	submitters: &[Keypair],
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	// Only pool-fill and mixed use more than one submitter. In the other scenarios each
	// writer submits to a different node, and the per-user limit applies per
	// `(node, submitter)` pair, so one account gives each writer a separate quota.
	let submitter = submitters.first().expect("at least one HOP submitter is derived");
	match scenario {
		"submit-only" | "submit" => {
			let sizes: Vec<(usize, &str)> = match payload_size {
				Some(s) => vec![(s, "custom")],
				None => SUBMIT_PAYLOAD_SIZES.to_vec(),
			};
			for (size, _label) in &sizes {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				run_submit_throughput(
					ws_urls,
					items,
					*size,
					concurrency,
					submitter,
					results,
					on_result,
					cancel,
				)
				.await?;
			}
		},
		"full-cycle" => {
			let size = payload_size.unwrap_or(100 * 1024);
			run_full_cycle(
				ws_urls,
				items,
				size,
				concurrency,
				submitter,
				results,
				on_result,
				cancel,
			)
			.await?;
		},
		"group" => {
			let size = payload_size.unwrap_or(100 * 1024);
			run_group(ws_urls, items, size, num_recipients, submitter, results, on_result, cancel)
				.await?;
		},
		"pool-fill" => {
			let size = payload_size.unwrap_or(10 * 1024);
			run_pool_fill(ws_urls, size, submitters, results, on_result, cancel).await?;
		},
		"mixed" => {
			let size = payload_size.unwrap_or(10 * 1024);
			run_mixed(
				ws_urls,
				size,
				concurrency,
				writers,
				ack_ratio,
				duration_secs,
				submitters,
				results,
				on_result,
				cancel,
			)
			.await?;
		},
		"errors" | "error-handling" => {
			let ok = run_error_tests(ws_urls, submitter).await?;
			if !ok {
				anyhow::bail!("Error handling tests failed");
			}
		},
		"all" => {
			for (size, _label) in SUBMIT_PAYLOAD_SIZES {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				run_submit_throughput(
					ws_urls,
					items,
					*size,
					concurrency,
					submitter,
					results,
					on_result,
					cancel,
				)
				.await?;
			}
			if !cancel.load(Ordering::Relaxed) {
				let size = payload_size.unwrap_or(100 * 1024);
				run_full_cycle(
					ws_urls,
					items,
					size,
					concurrency,
					submitter,
					results,
					on_result,
					cancel,
				)
				.await?;
			}
			if !cancel.load(Ordering::Relaxed) {
				let size = payload_size.unwrap_or(100 * 1024);
				run_group(
					ws_urls,
					items,
					size,
					num_recipients,
					submitter,
					results,
					on_result,
					cancel,
				)
				.await?;
			}
			if !cancel.load(Ordering::Relaxed) {
				let size = payload_size.unwrap_or(10 * 1024);
				run_mixed(
					ws_urls,
					size,
					concurrency,
					writers,
					ack_ratio,
					duration_secs,
					submitters,
					results,
					on_result,
					cancel,
				)
				.await?;
			}
			if !cancel.load(Ordering::Relaxed) {
				let _ = run_error_tests(ws_urls, submitter).await;
			}
		},
		other => anyhow::bail!("Unknown HOP scenario: {other}"),
	}
	Ok(())
}

fn format_payload_label(size: usize) -> String {
	if size >= 1024 * 1024 {
		format!("{}MB", size / (1024 * 1024))
	} else if size >= 1024 {
		format!("{}KB", size / 1024)
	} else {
		format!("{size}B")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rotation_advances_to_the_next_live_submitter() {
		let retired = vec![false; 3];
		assert_eq!(next_live_submitter(&retired, 0), Some(1));
		assert_eq!(next_live_submitter(&retired, 1), Some(2));
		// The search wraps around, so rate-limited accounts are used again after their
		// rate limit resets.
		assert_eq!(next_live_submitter(&retired, 2), Some(0));
	}

	#[test]
	fn rotation_skips_retired_submitters() {
		let retired = vec![false, true, true, false];
		assert_eq!(next_live_submitter(&retired, 0), Some(3));
		assert_eq!(next_live_submitter(&retired, 3), Some(0));
	}

	#[test]
	fn rotation_keeps_a_lone_live_submitter() {
		// The function checks `from` last, so it returns the only account with quota left
		// instead of None.
		let retired = vec![true, false, true];
		assert_eq!(next_live_submitter(&retired, 1), Some(1));
	}

	#[test]
	fn rotation_reports_exhaustion_when_every_submitter_is_retired() {
		assert_eq!(next_live_submitter(&[true, true], 0), None);
		assert_eq!(next_live_submitter(&[], 0), None);
	}

	/// The reader's ack decision, extracted so the ratio is testable without a node.
	fn ack_decisions(ack_permille: u64, claims: usize) -> usize {
		let mut credit = 0u64;
		let mut acks = 0;
		for _ in 0..claims {
			credit += ack_permille;
			if credit >= 1000 {
				credit -= 1000;
				acks += 1;
			}
		}
		acks
	}

	#[test]
	fn ack_ratio_governs_how_many_claims_are_acked() {
		// 1.0 acks everything, which is why the pool cannot grow at the default.
		assert_eq!(ack_decisions(1000, 100), 100);
		assert_eq!(ack_decisions(250, 100), 25);
		assert_eq!(ack_decisions(500, 100), 50);
		// 0.0 never releases: every entry stays until it expires.
		assert_eq!(ack_decisions(0, 100), 0);
	}

	#[test]
	fn ack_decisions_are_spread_not_front_loaded() {
		// At ratio 0.25 the accumulator acks one claim in every four, instead of acking
		// the first 250 claims of every 1000 and then none.
		let mut credit = 0u64;
		let mut acked_at = Vec::new();
		for i in 0..12u64 {
			credit += 250;
			if credit >= 1000 {
				credit -= 1000;
				acked_at.push(i);
			}
		}
		assert_eq!(acked_at, vec![3, 7, 11]);
	}
}
