use anyhow::Result;
use std::{
	sync::{
		atomic::{AtomicBool, AtomicU64, Ordering},
		Arc,
	},
	time::{Duration, Instant},
};
use tokio::sync::Mutex;

use crate::{
	client,
	hop::{self, RecipientKeypair},
	report::{self, ScenarioResult},
};

/// Payload sizes for the submit sweep.
const SUBMIT_PAYLOAD_SIZES: &[(usize, &str)] =
	&[(1024, "1KB"), (10 * 1024, "10KB"), (100 * 1024, "100KB"), (1024 * 1024, "1MB")];

// ---------------------------------------------------------------------------
// S1: Submit throughput
// ---------------------------------------------------------------------------

pub async fn run_submit_throughput(
	ws_urls: &[&str],
	items: u32,
	payload_size: usize,
	concurrency: usize,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	log::info!(
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

	let items_per_stream = (items as usize + concurrency - 1) / concurrency;

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

		handles.push(tokio::spawn(async move {
			let ws = match client::connect_ws(&url).await {
				Ok(ws) => ws,
				Err(e) => {
					log::error!("Failed to connect to {url}: {e}");
					return;
				},
			};

			for i in range_start..range_end {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				let data = hop::generate_payload(payload_size, i as u64);
				let recipients = vec![RecipientKeypair::generate()];

				match hop::hop_submit(&ws, &data, &recipients).await {
					Ok((_hash, _result, latency)) => {
						submitted.fetch_add(1, Ordering::Relaxed);
						bytes.fetch_add(payload_size as u64, Ordering::Relaxed);
						lats.lock().await.push(latency);
					},
					Err(e) => {
						errors.fetch_add(1, Ordering::Relaxed);
						let err_count = errors.load(Ordering::Relaxed);
						if err_count <= 5 {
							log::warn!("submit error [{i}]: {e}");
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

	let result = ScenarioResult {
		name: format!("HOP submit {}", format_payload_label(payload_size)),
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
			log::info!(
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

pub async fn run_full_cycle(
	ws_urls: &[&str],
	items: u32,
	payload_size: usize,
	concurrency: usize,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	log::info!(
		"S2: Full cycle — {} items × {} bytes, concurrency {}",
		items,
		payload_size,
		concurrency,
	);

	let entries = Arc::new(Mutex::new(Vec::<SubmittedEntry>::new()));
	let submit_lats = Arc::new(Mutex::new(Vec::<Duration>::new()));
	let submit_errors = Arc::new(AtomicU64::new(0));

	let items_per_stream = (items as usize + concurrency - 1) / concurrency;
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

		handles.push(tokio::spawn(async move {
			let ws = match client::connect_ws(&url).await {
				Ok(ws) => ws,
				Err(e) => {
					log::error!("Failed to connect to {url}: {e}");
					return;
				},
			};
			for i in range_start..range_end {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				let data = hop::generate_payload(payload_size, i as u64);
				let recipients = vec![RecipientKeypair::generate()];
				match hop::hop_submit(&ws, &data, &recipients).await {
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
							log::warn!("submit error [{i}]: {e}");
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
	log::info!("Submit phase done: {submitted} entries in {:.1}s", submit_duration.as_secs_f64());

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
						log::error!("Data mismatch! hash=0x{}", hex::encode(&entry.hash[..8]));
					}
					claim_lats.push(latency);
					claim_bytes += data.len() as u64;
				},
				Err(e) => {
					claim_errors += 1;
					if claim_errors <= 5 {
						log::warn!("claim error: {e}");
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

	let result = ScenarioResult {
		name: format!("HOP full-cycle {}", format_payload_label(payload_size)),
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

pub async fn run_group(
	ws_urls: &[&str],
	items: u32,
	payload_size: usize,
	num_recipients: usize,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	log::info!(
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
		let data = hop::generate_payload(payload_size, i as u64);
		let recipients: Vec<RecipientKeypair> =
			(0..num_recipients).map(|_| RecipientKeypair::generate()).collect();

		match hop::hop_submit(&ws, &data, &recipients).await {
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
				log::warn!("submit error [{i}]: {e}");
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
							log::error!("Data length mismatch in group claim");
						}
					},
					Err(e) => {
						errors.fetch_add(1, Ordering::Relaxed);
						if errors.load(Ordering::Relaxed) <= 5 {
							log::warn!("group claim error: {e}");
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

	let result = ScenarioResult {
		name: format!("HOP group ×{num_recipients} {}", format_payload_label(payload_size)),
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

	log::info!(
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
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	log::info!(
		"S4: Pool fill — {} byte payloads until PoolFull or UserQuotaExceeded",
		payload_size
	);

	let ws = client::connect_ws(ws_urls[0]).await?;

	if let Ok(status) = hop::hop_pool_status(&ws).await {
		log::info!(
			"Initial pool: {} entries, {} / {} bytes",
			status.entry_count,
			status.total_bytes,
			status.max_bytes
		);
	}

	let start = Instant::now();
	let mut submitted = 0u64;
	let mut errors = 0u64;
	let mut total_bytes = 0u64;
	let mut lats = Vec::new();
	let mut pool_full = false;

	for i in 0u64.. {
		if cancel.load(Ordering::Relaxed) || i >= 100_000 {
			if i >= 100_000 {
				log::info!("Hit 100k entries safety cap");
			}
			break;
		}

		let data = hop::generate_payload(payload_size, i);
		let recipients = vec![RecipientKeypair::generate()];

		match hop::hop_submit(&ws, &data, &recipients).await {
			Ok((_hash, result, latency)) => {
				submitted += 1;
				total_bytes += payload_size as u64;
				lats.push(latency);

				if submitted % 100 == 0 {
					log::info!(
						"  {} submitted, pool: {} entries, {} / {} bytes",
						submitted,
						result.pool_status.entry_count,
						result.pool_status.total_bytes,
						result.pool_status.max_bytes
					);
				}
			},
			Err(e) => {
				let err_str = e.to_string();
				// Check for PoolFull (1002) or UserQuotaExceeded (1013)
				if err_str.contains("1002") || err_str.contains("Pool full") {
					log::info!("PoolFull hit after {submitted} entries");
					pool_full = true;
					break;
				}
				if err_str.contains("1013") || err_str.contains("quota") {
					log::info!("UserQuotaExceeded hit after {submitted} entries");
					pool_full = true;
					break;
				}
				errors += 1;
				if errors <= 5 {
					log::warn!("pool-fill submit error [{i}]: {e}");
				}
				if errors > 10 {
					log::error!("Too many errors, stopping");
					break;
				}
			},
		}
	}

	let duration = start.elapsed();
	let tps =
		if duration.as_secs_f64() > 0.0 { submitted as f64 / duration.as_secs_f64() } else { 0.0 };

	let result = ScenarioResult {
		name: format!(
			"HOP pool-fill {}{}",
			format_payload_label(payload_size),
			if pool_full { " (full)" } else { "" }
		),
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

	if let Ok(status) = hop::hop_pool_status(&ws).await {
		log::info!(
			"Final pool: {} entries, {} / {} bytes",
			status.entry_count,
			status.total_bytes,
			status.max_bytes
		);
	}

	results.push(result);
	on_result(results);
	Ok(())
}

// ---------------------------------------------------------------------------
// S5: Mixed read/write
// ---------------------------------------------------------------------------

pub async fn run_mixed(
	ws_urls: &[&str],
	payload_size: usize,
	concurrency: usize,
	duration_secs: u64,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	log::info!(
		"S5: Mixed — {} byte payloads, concurrency {}, {}s duration",
		payload_size,
		concurrency,
		duration_secs,
	);

	let writer_count = std::cmp::max(1, concurrency / 2);
	let reader_count = std::cmp::max(1, concurrency - writer_count);

	let deadline = Instant::now() + Duration::from_secs(duration_secs);

	// Shared queue: writers push, readers pop
	let pending = Arc::new(Mutex::new(Vec::<SubmittedEntry>::new()));

	let submit_count = Arc::new(AtomicU64::new(0));
	let submit_errors = Arc::new(AtomicU64::new(0));
	let submit_bytes = Arc::new(AtomicU64::new(0));
	let submit_lats = Arc::new(Mutex::new(Vec::<Duration>::new()));

	let claim_count = Arc::new(AtomicU64::new(0));
	let claim_errors = Arc::new(AtomicU64::new(0));
	let claim_bytes = Arc::new(AtomicU64::new(0));
	let claim_lats = Arc::new(Mutex::new(Vec::<Duration>::new()));

	let writers_done = Arc::new(AtomicBool::new(false));

	let start = Instant::now();

	// Spawn writers
	let mut writer_handles = Vec::new();
	for w_idx in 0..writer_count {
		let url = ws_urls[w_idx % ws_urls.len()].to_string();
		let pending = pending.clone();
		let count = submit_count.clone();
		let errors = submit_errors.clone();
		let bytes = submit_bytes.clone();
		let lats = submit_lats.clone();
		let cancel = cancel.clone();

		writer_handles.push(tokio::spawn(async move {
			let ws = match client::connect_ws(&url).await {
				Ok(ws) => ws,
				Err(e) => {
					log::error!("Writer {w_idx} connect failed: {e}");
					return;
				},
			};

			let mut idx = w_idx as u64 * 1_000_000;
			while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
				let data = hop::generate_payload(payload_size, idx);
				let recipients = vec![RecipientKeypair::generate()];
				idx += 1;

				match hop::hop_submit(&ws, &data, &recipients).await {
					Ok((hash, _result, latency)) => {
						count.fetch_add(1, Ordering::Relaxed);
						bytes.fetch_add(payload_size as u64, Ordering::Relaxed);
						lats.lock().await.push(latency);
						pending.lock().await.push(SubmittedEntry {
							hash,
							data,
							recipients,
							collator_url: url.clone(),
						});
					},
					Err(_) => {
						errors.fetch_add(1, Ordering::Relaxed);
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
		let bytes = claim_bytes.clone();
		let lats = claim_lats.clone();
		let cancel = cancel.clone();
		let writers_done = writers_done.clone();

		reader_handles.push(tokio::spawn(async move {
			loop {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				let entry = pending.lock().await.pop();
				match entry {
					Some(entry) => {
						let ws = match client::connect_ws(&entry.collator_url).await {
							Ok(ws) => ws,
							Err(_) => continue,
						};
						let kp = &entry.recipients[0];
						match hop::hop_claim(&ws, &entry.hash, kp).await {
							Ok((data, latency)) => {
								count.fetch_add(1, Ordering::Relaxed);
								bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
								lats.lock().await.push(latency);
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
			log::info!(
				"[{:.0}s] submitted: {}, claimed: {}, pending: {}, errors: {}/{}",
				elapsed,
				s_count.load(Ordering::Relaxed),
				c_count.load(Ordering::Relaxed),
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

	let result = ScenarioResult {
		name: format!("HOP mixed {}s {}", duration_secs, format_payload_label(payload_size)),
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

pub async fn run_error_tests(ws_urls: &[&str]) -> Result<bool> {
	log::info!("Error handling tests");

	let ws = client::connect_ws(ws_urls[0]).await?;
	let mut passed = 0u32;
	let mut failed = 0u32;

	// Helper: expect a specific error code from hop_submit
	macro_rules! expect_submit_error {
		($name:expr, $code:expr, $data:expr, $recipients:expr) => {{
			match hop::try_hop_submit(&ws, $data, $recipients).await {
				Some(code) if code == $code => {
					log::info!("  PASS: {} (code {})", $name, code);
					passed += 1;
				},
				Some(code) => {
					log::error!("  FAIL: {} — expected {}, got {}", $name, $code, code);
					failed += 1;
				},
				None => {
					log::error!("  FAIL: {} — expected error {}, got success", $name, $code);
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
		match hop::try_hop_claim(&ws, &fake_hash, &fake_kp).await {
			Some(code) if code == 1004 => {
				log::info!("  PASS: NotFound (code 1004)");
				passed += 1;
			},
			Some(code) => {
				log::error!("  FAIL: NotFound — expected 1004, got {code}");
				failed += 1;
			},
			None => {
				log::error!("  FAIL: NotFound — expected error, got success");
				failed += 1;
			},
		}
	}

	// 4. Claim with wrong keypair -> 1010
	{
		let data = hop::generate_payload(1024, 999_999);
		let valid_kp = RecipientKeypair::generate();
		let wrong_kp = RecipientKeypair::generate();

		match hop::hop_submit(&ws, &data, &[valid_kp.clone()]).await {
			Ok((hash, _, _)) => {
				match hop::try_hop_claim(&ws, &hash, &wrong_kp).await {
					Some(code) if code == 1010 => {
						log::info!("  PASS: NotRecipient (code 1010)");
						passed += 1;
					},
					Some(code) => {
						log::error!("  FAIL: NotRecipient — expected 1010, got {code}");
						failed += 1;
					},
					None => {
						log::error!("  FAIL: NotRecipient — expected error, got success");
						failed += 1;
					},
				}
				// Clean up
				let _ = hop::hop_claim(&ws, &hash, &valid_kp).await;
			},
			Err(e) => {
				log::warn!("  SKIP: NotRecipient — submit failed: {e}");
			},
		}
	}

	// 5. Duplicate entry -> 1003
	{
		let data = hop::generate_payload(512, 998_998);
		let kp = RecipientKeypair::generate();

		match hop::hop_submit(&ws, &data, &[kp]).await {
			Ok(_) => {
				let kp2 = RecipientKeypair::generate();
				expect_submit_error!("DuplicateEntry", 1003, &data, &[kp2]);
			},
			Err(e) => {
				log::warn!("  SKIP: DuplicateEntry — submit failed: {e}");
			},
		}
	}

	// 6. DataTooLarge -> skip (64 MiB impractical over WS)
	log::info!("  SKIP: DataTooLarge (65 MiB payload too large for WS transport)");

	// 7. Invalid hash length -> 1008
	{
		let short_hash = [0xCCu8; 16];
		let kp = RecipientKeypair::generate();
		match hop::try_hop_claim(&ws, &short_hash, &kp).await {
			Some(code) if code == 1008 => {
				log::info!("  PASS: InvalidHashLength (code 1008)");
				passed += 1;
			},
			Some(code) => {
				log::error!("  FAIL: InvalidHashLength — expected 1008, got {code}");
				failed += 1;
			},
			None => {
				log::error!("  FAIL: InvalidHashLength — expected error, got success");
				failed += 1;
			},
		}
	}

	log::info!("Results: {passed} passed, {failed} failed");
	Ok(failed == 0)
}

// ---------------------------------------------------------------------------
// Sweep runner (called from main)
// ---------------------------------------------------------------------------

pub async fn run_hop_sweep(
	ws_urls: &[&str],
	scenario: &str,
	items: u32,
	payload_size: Option<usize>,
	concurrency: usize,
	num_recipients: usize,
	duration_secs: u64,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
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
					results,
					on_result,
					cancel,
				)
				.await?;
			}
		},
		"full-cycle" => {
			let size = payload_size.unwrap_or(100 * 1024);
			run_full_cycle(ws_urls, items, size, concurrency, results, on_result, cancel).await?;
		},
		"group" => {
			let size = payload_size.unwrap_or(100 * 1024);
			run_group(ws_urls, items, size, num_recipients, results, on_result, cancel).await?;
		},
		"pool-fill" => {
			let size = payload_size.unwrap_or(10 * 1024);
			run_pool_fill(ws_urls, size, results, on_result, cancel).await?;
		},
		"mixed" => {
			let size = payload_size.unwrap_or(10 * 1024);
			run_mixed(ws_urls, size, concurrency, duration_secs, results, on_result, cancel)
				.await?;
		},
		"errors" | "error-handling" => {
			let ok = run_error_tests(ws_urls).await?;
			if !ok {
				anyhow::bail!("Error handling tests failed");
			}
		},
		"all" => {
			// Run all scenarios in sequence
			for (size, _label) in SUBMIT_PAYLOAD_SIZES {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				run_submit_throughput(
					ws_urls,
					items,
					*size,
					concurrency,
					results,
					on_result,
					cancel,
				)
				.await?;
			}
			if !cancel.load(Ordering::Relaxed) {
				let size = payload_size.unwrap_or(100 * 1024);
				run_full_cycle(ws_urls, items, size, concurrency, results, on_result, cancel)
					.await?;
			}
			if !cancel.load(Ordering::Relaxed) {
				let size = payload_size.unwrap_or(100 * 1024);
				run_group(ws_urls, items, size, num_recipients, results, on_result, cancel).await?;
			}
			if !cancel.load(Ordering::Relaxed) {
				let size = payload_size.unwrap_or(10 * 1024);
				run_mixed(ws_urls, size, concurrency, duration_secs, results, on_result, cancel)
					.await?;
			}
			if !cancel.load(Ordering::Relaxed) {
				let _ = run_error_tests(ws_urls).await;
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
