use anyhow::Result;
use std::sync::{
	atomic::{AtomicBool, Ordering},
	Arc,
};
use subxt::OnlineClient;
use subxt_signer::sr25519::Keypair;

use crate::{
	accounts::NonceTracker,
	chain_info::ChainLimits,
	client::BulletinConfig,
	pipeline::{self, IterationPlan, PayloadSizeMix, StorePayloadMode, StressWorkItem},
	report::{BlockStats, ScenarioResult, SubmissionStats},
	store,
};

/// Maximum raw store payload size for stress-test variants (matches chain / runtime limit, ~2 MiB).
pub const MAX_STORE_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;

/// Canonical payload table for fixed variants and for **`MIXED`** weights (subset of labels).
/// Every `usize` is ≤ [`MAX_STORE_PAYLOAD_BYTES`].
const ALL_PAYLOAD_SIZES: &[(usize, &str)] = &[
	(1024, "1KB"),
	(4096, "4KB"),
	(32 * 1024, "32KB"),
	(128 * 1024, "128KB"),
	(512 * 1024, "512KB"),
	(1024 * 1024, "1MB"),
	(MAX_STORE_PAYLOAD_BYTES, "2MB"),
];

/// Weighted mix for **`MIXED`**: labels must exist in [`ALL_PAYLOAD_SIZES`] (same strings as
/// `--variants`). Weights sum to 1000 (~basis points).
const REAL_WORLD_MIX_LABEL_WEIGHTS: &[(&str, u32)] = &[
	("1KB", 230),
	("4KB", 150),
	("32KB", 120),
	("128KB", 175),
	("512KB", 155),
	("1MB", 90),
	("2MB", 80),
];

fn real_world_payload_mix() -> anyhow::Result<PayloadSizeMix> {
	let pairs: Vec<(usize, u32)> = REAL_WORLD_MIX_LABEL_WEIGHTS
		.iter()
		.map(|&(label, w)| {
			let size = ALL_PAYLOAD_SIZES
				.iter()
				.find(|(_, l)| *l == label)
				.map(|(s, _)| *s)
				.ok_or_else(|| {
					anyhow::anyhow!(
						"REAL_WORLD_MIX_LABEL_WEIGHTS: label {label:?} not in ALL_PAYLOAD_SIZES"
					)
				})?;
			Ok((size, w))
		})
		.collect::<anyhow::Result<_>>()?;
	PayloadSizeMix::from_weighted_sizes(&pairs)
}

enum BlockCapacitySweepStep {
	Fixed { size: usize, label: &'static str },
	Mixed { mix: PayloadSizeMix },
}

fn build_sweep_steps(variant_filter: Option<&str>) -> anyhow::Result<Vec<BlockCapacitySweepStep>> {
	match variant_filter {
		None => Ok(ALL_PAYLOAD_SIZES
			.iter()
			.map(|&(size, label)| BlockCapacitySweepStep::Fixed { size, label })
			.collect()),
		Some(f) => {
			let tokens: Vec<String> = f
				.split(',')
				.map(|s| s.trim().to_uppercase())
				.filter(|s| !s.is_empty())
				.collect();
			if tokens.is_empty() {
				anyhow::bail!("Empty --variants string");
			}
			let mix = if tokens.iter().any(|t| t == "MIXED") {
				Some(real_world_payload_mix()?)
			} else {
				None
			};
			let mut steps = Vec::with_capacity(tokens.len());
			for t in tokens {
				if t == "MIXED" {
					steps.push(BlockCapacitySweepStep::Mixed { mix: mix.clone().unwrap() });
					continue;
				}
				let found =
					ALL_PAYLOAD_SIZES.iter().find(|(_, label)| label.to_uppercase() == t).copied();
				match found {
					Some((size, label)) =>
						steps.push(BlockCapacitySweepStep::Fixed { size, label }),
					None => {
						let available: Vec<&str> =
							ALL_PAYLOAD_SIZES.iter().map(|(_, l)| *l).collect();
						anyhow::bail!(
							"Unknown variant {t:?}. Available: {}, MIXED",
							available.join(", ")
						);
					},
				}
			}
			Ok(steps)
		},
	}
}

fn scenario_result_from_bulk(
	result: &store::BulkStoreResult,
	account_count: usize,
	payload_size: usize,
	label: &str,
	chain_limits: &ChainLimits,
) -> ScenarioResult {
	let all_blocks = &result.blocks;
	let measured: Vec<_> = all_blocks.iter().filter(|b| !b.prefill).collect();
	let with_txs = measured.iter().filter(|b| b.tx_count > 0).count();
	tracing::debug!(
		"scenario_result_from_bulk({label}): {} total blocks, {} measured, \
		 {} with txs, {} finalized",
		all_blocks.len(),
		measured.len(),
		with_txs,
		all_blocks.iter().filter(|b| b.finalized).count(),
	);

	let first_with_txs = measured.iter().position(|b| b.tx_count > 0);
	let last_with_txs = measured.iter().rposition(|b| b.tx_count > 0);
	let steady: Vec<_> = match (first_with_txs, last_with_txs) {
		(Some(first), Some(last)) if last > first + 1 => measured[first + 1..last].to_vec(),
		_ => measured.clone(),
	};

	let steady_bytes: u64 = steady.iter().map(|b| b.payload_bytes).sum();
	let peak = steady.iter().map(|b| b.tx_count).max().unwrap_or(0);
	let avg = if !steady.is_empty() {
		steady.iter().map(|b| b.tx_count).sum::<u64>() as f64 / steady.len() as f64
	} else {
		0.0
	};
	let measured_confirmed: u64 = measured.iter().map(|b| b.tx_count).sum();
	let prefill_count = all_blocks.iter().filter(|b| b.prefill).count();
	let empty_count = steady.iter().filter(|b| b.tx_count == 0).count();

	let intervals: Vec<u64> = steady.iter().filter_map(|b| b.interval_ms).collect();
	let avg_block_interval_ms = if !intervals.is_empty() {
		Some(intervals.iter().sum::<u64>() as f64 / intervals.len() as f64)
	} else {
		None
	};

	// Use client-side wall clock of steady-state blocks for throughput.
	// On-chain timestamps are unreliable with elastic scaling — multiple blocks
	// per relay slot share the same pallet_timestamp::Now value.
	let steady_wall_ms = match (
		steady.first().and_then(|b| b.received_at_ms),
		steady.last().and_then(|b| b.received_at_ms),
	) {
		(Some(first), Some(last)) if last > first => last - first,
		_ => 0,
	};
	// Add one block interval estimate so we don't undercount (N blocks span N-1 gaps).
	let steady_wall_secs = if steady_wall_ms > 0 && !steady.is_empty() {
		let per_block_ms = steady_wall_ms as f64 / (steady.len() - 1).max(1) as f64;
		(steady_wall_ms as f64 + per_block_ms) / 1000.0
	} else {
		result.duration.as_secs_f64()
	};
	let (tps, bps) = if steady_wall_secs > 0.0 {
		(measured_confirmed as f64 / steady_wall_secs, steady_bytes as f64 / steady_wall_secs)
	} else {
		(0.0, 0.0)
	};

	tracing::info!(
		"block-cap: {} total blocks ({} prefill, {} measured), {} steady-state \
		 ({} empty), avg={:.1}, peak={}, steady_wall={:.1}s",
		all_blocks.len(),
		prefill_count,
		measured.len(),
		steady.len(),
		empty_count,
		avg,
		peak,
		steady_wall_secs
	);

	ScenarioResult {
		name: format!("block-cap: Block Capacity ({label}, {account_count} accounts)"),
		duration: result.duration,
		total_submitted: result.total_submitted,
		total_confirmed: result.total_confirmed,
		total_errors: result.total_errors,
		payload_size,
		throughput_tps: tps,
		throughput_bytes_per_sec: bps,
		avg_tx_per_block: avg,
		peak_tx_per_block: peak,
		inclusion_latency: None,
		finalization_latency: None,
		retrieval_latency: None,
		theoretical: Some(chain_limits.compute_theoretical_limits(payload_size)),
		chain_limits: None,
		environment: None,
		blocks: all_blocks.clone(),
		submission_stats: Some(SubmissionStats {
			stale_nonces: result.stale_nonces,
			pool_full_retries: result.pool_full_retries,
			errors: result.total_errors,
			remaining_in_queue: result.remaining_in_queue,
			nonces_initialized: result.nonces_initialized,
			nonces_failed: result.nonces_failed,
			gap_repairs: None,
			waves_submitted: None,
		}),
		avg_block_interval_ms,
		fork_detections: result.fork_detections,
		onchain_timing: false,
		total_reads: None,
		successful_reads: None,
		failed_reads: None,
		reads_per_sec: None,
		read_bytes_per_sec: None,
		data_verified: None,
	}
}

/// Block capacity measurement across multiple payload sizes (including a weighted **MIXED** mode).
///
/// For each step, splits one-shot accounts into iterations (~`iteration_blocks` measured blocks
/// worth of txs per iteration), then runs the producer/consumer pipeline.  Authorization of each
/// batch is interleaved with store dispatch of the previous batch (see
/// [`generate_block_capacity_work`](pipeline::generate_block_capacity_work)).  The reader applies
/// txpool backpressure every
/// [`POOL_PENDING_PAUSE_THRESHOLD`](pipeline::POOL_PENDING_PAUSE_THRESHOLD) dispatches.  Drains the
/// pool between variants.
#[allow(clippy::too_many_arguments)]
pub async fn run_block_capacity_sweep(
	client: &OnlineClient<BulletinConfig>,
	authorizer_signer: &Keypair,
	nonce_tracker: &NonceTracker,
	ws_urls: &[&str],
	chain_limits: &ChainLimits,
	submitters: usize,
	target_blocks: u32,
	iteration_blocks: u32,
	variant_filter: Option<&str>,
	mix_seed: Option<u64>,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	let iteration_blocks = iteration_blocks.max(1);
	let block_usable_bytes = chain_limits.normal_block_length as usize;
	let extrinsic_overhead = chain_limits.extrinsic_length_overhead as usize;
	let max_block_txs = chain_limits.max_block_transactions as usize;

	let sweep_steps = build_sweep_steps(variant_filter)?;

	let step_labels: Vec<String> = sweep_steps
		.iter()
		.map(|s| match s {
			BlockCapacitySweepStep::Fixed { label, .. } => (*label).to_string(),
			BlockCapacitySweepStep::Mixed { .. } => "mixed".to_string(),
		})
		.collect();
	tracing::info!(
		"Running {} block-capacity step(s): {}",
		sweep_steps.len(),
		step_labels.join(", ")
	);

	for step in &sweep_steps {
		if cancel.load(Ordering::Relaxed) {
			tracing::warn!("block-capacity sweep: cancelled, skipping remaining variants");
			break;
		}
		let (
			label,
			payload_size_report,
			est_block_cap,
			store_payload,
			est_pool_bytes_per_account,
			largest_payload_in_step,
		) = match step {
			BlockCapacitySweepStep::Fixed { size, label } => {
				let cap =
					(block_usable_bytes / (*size + extrinsic_overhead).max(1)).min(max_block_txs);
				(
					*label,
					*size,
					cap,
					StorePayloadMode::Fixed(*size),
					*size + extrinsic_overhead,
					*size,
				)
			},
			BlockCapacitySweepStep::Mixed { mix } => {
				// Use the smallest payload in the mix to estimate max accounts needed
				// per block. This ensures we always generate enough accounts to fill
				// blocks even when small payloads dominate a given block.
				let min_b = mix.min_payload_bytes().max(1);
				let cap =
					(block_usable_bytes / (min_b + extrinsic_overhead).max(1)).min(max_block_txs);
				let mean = mix.mean_payload_bytes().round().max(1.0) as usize;
				let max_b = mix.max_payload_bytes();
				let seed_note = match mix_seed {
					Some(s) => format!("--mix-seed {s}"),
					None => "OS entropy (use --mix-seed to reproduce)".to_string(),
				};
				tracing::info!(
					"mixed: weighted payload mix — mean ≈ {mean} B, min ≈ {min_b} B, \
					 max ≈ {max_b} B, est ≤ {cap} txs/block (worst case); draws: {seed_note}",
				);
				(
					"mixed",
					mean,
					cap,
					StorePayloadMode::Mixed(mix.clone()),
					min_b + extrinsic_overhead,
					max_b,
				)
			},
		};

		let total_block_slots = (target_blocks + 2) as usize * est_block_cap;
		let backpressure_buffer = est_block_cap * 3;
		let accounts_needed = ((total_block_slots + backpressure_buffer) * 3 / 2).max(1) as u32;
		let est_pool_mb = (accounts_needed as usize * est_pool_bytes_per_account) / (1024 * 1024);
		let accounts_per_iter =
			pipeline::block_capacity_accounts_per_iteration(est_block_cap, iteration_blocks);
		let n_iterations = accounts_needed.div_ceil(accounts_per_iter);
		tracing::info!(
			"=== block-capacity variant: {label} payload, {accounts_needed} one-shot accounts \
			 in {n_iterations} iteration(s) (~{iteration_blocks} measured blocks/iter × ~{est_block_cap} txs/block \
			 ≈ {accounts_per_iter} accounts/iter), est. pool demand ~{est_pool_mb} MB ===",
		);

		let run_id = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap_or_default()
			.as_millis();
		let seed = format!("T2sweep_{label}_{run_id}");
		let plans: Vec<IterationPlan> =
			pipeline::build_iteration_plans(accounts_needed, accounts_per_iter, &seed);
		let txpool_pause = pipeline::POOL_PENDING_PAUSE_THRESHOLD;
		tracing::info!(
			"{label}: iteration layout ready ({} iterations; interleaved Authorize + Store; \
			 txpool gate every {txpool_pause} dispatches (pause if pool > {txpool_pause}); \
			 authorize chunks of {})",
			plans.len(),
			crate::authorize::AUTHORIZE_BATCH_SIZE,
		);
		let variant_result: Result<ScenarioResult> = async {
			let dual = store::subscribe_blocks_dual(ws_urls[0]).await?;
			let (work_tx, work_rx) =
				tokio::sync::mpsc::channel::<StressWorkItem>(pipeline::WORK_CHANNEL_CAPACITY);

			let gen_plans = plans.clone();
			let gen_store = store_payload.clone();
			let gen_mix_seed = mix_seed;
			let gen_client = std::sync::Arc::new(client.clone());
			let generator = tokio::spawn(async move {
				pipeline::generate_block_capacity_work(
					work_tx,
					&gen_plans,
					gen_store,
					gen_mix_seed,
					gen_client,
				)
				.await
			});

			let pipeline_out = pipeline::run_block_capacity_pipeline(
				work_rx,
				dual,
				ws_urls,
				submitters,
				store_payload.clone(),
				client,
				authorizer_signer,
				nonce_tracker,
				cancel,
				Some(target_blocks),
			)
			.await;

			// Always abort the generator — it may still be producing work items
			// after the pipeline stopped (target reached, cancel, or error).
			generator.abort();
			let _ = generator.await;

			let bulk = match pipeline_out {
				Ok(b) => b,
				Err(e) => return Err(e),
			};

			let total_accounts: u32 = plans.iter().map(|p| p.account_count).sum();
			let result = scenario_result_from_bulk(
				&bulk,
				total_accounts as usize,
				payload_size_report,
				label,
				chain_limits,
			);

			if largest_payload_in_step >= MAX_STORE_PAYLOAD_BYTES && result.total_confirmed == 0 {
				tracing::warn!(
					"{label}: 0 txs confirmed (may be expected due to WASM heap limits) - \
					 including result"
				);
			}

			Ok(result)
		}
		.await;

		match variant_result {
			Ok(result) => {
				results.push(result);
				on_result(results);
			},
			Err(e) => {
				tracing::error!("{label}: variant failed: {e}");
				results.push(ScenarioResult {
					name: format!("block-cap: Block Capacity ({label}) — ERROR"),
					payload_size: payload_size_report,
					theoretical: Some(chain_limits.compute_theoretical_limits(payload_size_report)),
					..Default::default()
				});
				on_result(results);
			},
		}

		// Drain the transaction pool before the next variant (skip if stopping).
		if cancel.load(Ordering::Relaxed) {
			break;
		}
		tracing::info!("Draining pool after {label} variant...");
		if let Ok(mut blocks_sub) = client.blocks().subscribe_best().await {
			let mut consecutive_empty = 0u32;
			let drain_deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
			loop {
				if std::time::Instant::now() > drain_deadline {
					tracing::warn!("Pool drain timed out after 120s, continuing");
					break;
				}
				if let Some(Ok(block)) = blocks_sub.next().await {
					let stored_count = store::stored_content_hashes(&block).await.len();
					if stored_count == 0 {
						consecutive_empty += 1;
						if consecutive_empty >= 2 {
							tracing::info!(
								"Pool drained (2 consecutive empty blocks, last #{}) \
								 - safe for next variant",
								block.number()
							);
							break;
						}
					} else {
						consecutive_empty = 0;
						tracing::info!(
							"Block #{} still has {stored_count} Stored events, waiting...",
							block.number()
						);
					}
				}
			}
		} else {
			tracing::warn!("Could not subscribe to blocks for pool drain, skipping");
		}
	}

	Ok(())
}

/// Sequential nonce upload: single account uploads `total_bytes` of data
/// split into `chunk_size`-byte store transactions with wave-based pool feeding.
///
/// Measures throughput (MB/s, tx/s) including re-sign overhead.
/// When `instances > 1`, runs multiple uploads in parallel with different
/// accounts. The total payload is divided evenly across instances.
#[allow(clippy::too_many_arguments)]
pub async fn run_sequential_upload(
	client: &OnlineClient<BulletinConfig>,
	authorizer_signer: &Keypair,
	nonce_tracker: &NonceTracker,
	ws_urls: &[&str],
	chain_limits: &ChainLimits,
	total_bytes: usize,
	chunk_size: usize,
	instances: usize,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
) -> Result<()> {
	let instances = instances.max(1);
	let bytes_per_instance = total_bytes / instances;
	let num_txs_per_instance = bytes_per_instance.div_ceil(chunk_size);
	let total_txs = num_txs_per_instance * instances;

	tracing::info!(
		"=== sequential-upload: {instances} instance(s), {total_txs} txs total \
		 ({num_txs_per_instance}×{} KB per instance = {} MB each, {} MB total) ===",
		chunk_size / 1024,
		bytes_per_instance / (1024 * 1024),
		total_bytes / (1024 * 1024),
	);

	// Generate a fresh random account per instance.
	let signers: Vec<subxt_signer::sr25519::Keypair> = (0..instances)
		.map(|_| {
			subxt_signer::sr25519::Keypair::from_secret_key(rand::random())
				.expect("valid keypair from random bytes")
		})
		.collect();
	let account_ids: Vec<subxt::utils::AccountId32> =
		signers.iter().map(|s| s.public_key().to_account_id()).collect();

	for (i, id) in account_ids.iter().enumerate() {
		tracing::info!("Instance {i}: account {id}");
	}

	// Authorize all accounts in one batch.
	tracing::info!(
		"Authorizing {instances} accounts for {num_txs_per_instance} txs, {} MB each...",
		bytes_per_instance / (1024 * 1024)
	);
	crate::authorize::authorize_accounts(
		client,
		authorizer_signer,
		nonce_tracker,
		&account_ids,
		num_txs_per_instance as u32 + 10,
		(bytes_per_instance as u64) + 100 * 1024 * 1024,
	)
	.await?;
	tracing::info!("All {instances} accounts authorized");

	// Generate payloads for each instance.
	let mut all_payloads = Vec::new();
	for i in 0..instances {
		tracing::info!(
			"Instance {i}: generating {num_txs_per_instance} payloads ({chunk_size} bytes each)..."
		);
		let payloads: Vec<Vec<u8>> = (0..num_txs_per_instance)
			.map(|j| {
				store::generate_indexed_payload(chunk_size, (i * num_txs_per_instance + j) as u32)
			})
			.collect();
		all_payloads.push(payloads);
	}

	// Launch all instances concurrently (no tokio::spawn — futures run
	// on the current task, which avoids 'static lifetime requirements).
	let ws_owned: Vec<String> = ws_urls.iter().map(|s| s.to_string()).collect();
	let futures: Vec<_> = signers
		.iter()
		.zip(all_payloads.into_iter())
		.enumerate()
		.map(|(i, (signer, payloads))| {
			let ws = ws_owned.clone();
			async move {
				let result =
					store::sequential_nonce_upload(client, signer, payloads, ws, chain_limits)
						.await;
				(i, result)
			}
		})
		.collect();

	let outcomes = futures::future::join_all(futures).await;

	let mut upload_results = Vec::new();
	for (i, result) in outcomes {
		match result {
			Ok(r) => {
				tracing::info!(
					"Instance {i}: confirmed {}/{}",
					r.total_confirmed,
					num_txs_per_instance
				);
				upload_results.push(r);
			},
			Err(e) => {
				tracing::error!("Instance {i}: failed: {e}");
				return Err(e);
			},
		}
	}

	// Merge results: combine blocks across instances, sum stats.
	let result = if upload_results.len() == 1 {
		upload_results.into_iter().next().unwrap()
	} else {
		// Merge: sum stats, combine block lists, take max duration.
		let total_submitted: u64 = upload_results.iter().map(|r| r.total_submitted).sum();
		let total_confirmed: u64 = upload_results.iter().map(|r| r.total_confirmed).sum();
		let total_errors: u64 = upload_results.iter().map(|r| r.total_errors).sum();
		let waves: u64 = upload_results.iter().map(|r| r.waves_submitted).sum();
		let duration = upload_results.iter().map(|r| r.duration).max().unwrap_or_default();

		// Merge block stats: group by block number, sum tx counts.
		let mut block_map: std::collections::BTreeMap<u64, BlockStats> =
			std::collections::BTreeMap::new();
		for r in &upload_results {
			for b in &r.blocks {
				let entry = block_map.entry(b.number).or_insert_with(|| BlockStats {
					number: b.number,
					tx_count: 0,
					payload_bytes: 0,
					prefill: false,
					timestamp_ms: b.timestamp_ms,
					hash: b.hash.clone(),
					finalized: b.finalized,
					received_at_ms: b.received_at_ms,
					interval_ms: b.interval_ms,
				});
				entry.tx_count += b.tx_count;
				entry.payload_bytes += b.payload_bytes;
			}
		}

		store::SequentialUploadResult {
			total_submitted,
			total_confirmed,
			total_errors,
			gap_repairs: 0,
			waves_submitted: waves,
			duration,
			blocks: block_map.into_values().collect(),
			fork_detections: 0,
		}
	};

	// Compute stats from block data.
	// No steady-state trimming for sequential upload — all blocks with txs are real data,
	// not ramp-up/ramp-down artifacts.
	let all_blocks = &result.blocks;
	let blocks_with_txs: Vec<_> = all_blocks.iter().filter(|b| b.tx_count > 0).collect();

	let avg = if !blocks_with_txs.is_empty() {
		blocks_with_txs.iter().map(|b| b.tx_count).sum::<u64>() as f64 /
			blocks_with_txs.len() as f64
	} else {
		0.0
	};
	let peak = blocks_with_txs.iter().map(|b| b.tx_count).max().unwrap_or(0);
	let total_tx_count: u64 = blocks_with_txs.iter().map(|b| b.tx_count).sum();
	let total_payload_bytes: u64 = blocks_with_txs.iter().map(|b| b.payload_bytes).sum();

	let intervals: Vec<u64> = all_blocks.iter().filter_map(|b| b.interval_ms).collect();
	let avg_block_interval_ms = if !intervals.is_empty() {
		Some(intervals.iter().sum::<u64>() as f64 / intervals.len() as f64)
	} else {
		None
	};

	// Duration: first block with txs → last block with txs (including empty blocks in between).
	let onchain_duration_ms = match (
		blocks_with_txs.first().and_then(|b| b.timestamp_ms),
		blocks_with_txs.last().and_then(|b| b.timestamp_ms),
	) {
		(Some(first_ts), Some(last_ts)) if last_ts > first_ts => Some(last_ts - first_ts),
		_ => None,
	};

	let effective_duration = onchain_duration_ms
		.map(std::time::Duration::from_millis)
		.unwrap_or(result.duration);
	let secs = effective_duration.as_secs_f64().max(0.001);
	let throughput_tx_s = total_tx_count as f64 / secs;
	let throughput_bytes_s = total_payload_bytes as f64 / secs;

	let blocks_with_data = blocks_with_txs.len();
	let total_blocks_observed = all_blocks.len();
	tracing::info!(
		"sequential-upload: {total_tx_count} txs in {blocks_with_data} blocks ({total_blocks_observed} total), \
		 avg={avg:.1} tx/block, peak={peak}, throughput={throughput_tx_s:.1} tx/s, \
		 {:.2} MB/s, on-chain={:.1}s, wall={:.1}s, repairs={}",
		throughput_bytes_s / 1048576.0,
		secs,
		result.duration.as_secs_f64(),
		result.gap_repairs,
	);

	let scenario = ScenarioResult {
		name: format!(
			"sequential-upload: {} MB via {}×{}KB ({instances} account{})",
			total_bytes / (1024 * 1024),
			total_txs,
			chunk_size / 1024,
			if instances > 1 { "s" } else { "" },
		),
		duration: result.duration,
		total_submitted: result.total_submitted,
		total_confirmed: result.total_confirmed,
		total_errors: result.total_errors,
		avg_tx_per_block: avg,
		peak_tx_per_block: peak,
		throughput_tps: throughput_tx_s,
		throughput_bytes_per_sec: throughput_bytes_s,
		avg_block_interval_ms,
		payload_size: chunk_size,
		blocks: all_blocks.clone(),
		chain_limits: Some(chain_limits.clone()),
		environment: None,
		submission_stats: Some(SubmissionStats {
			stale_nonces: 0,
			pool_full_retries: 0,
			errors: result.total_errors,
			remaining_in_queue: 0,
			nonces_initialized: 1,
			nonces_failed: 0,
			gap_repairs: Some(result.gap_repairs),
			waves_submitted: Some(result.waves_submitted),
		}),
		fork_detections: result.fork_detections,
		onchain_timing: onchain_duration_ms.is_some(),
		inclusion_latency: None,
		finalization_latency: None,
		retrieval_latency: None,
		theoretical: None,
		successful_reads: None,
		failed_reads: None,
		total_reads: None,
		reads_per_sec: None,
		read_bytes_per_sec: None,
		data_verified: None,
	};

	results.push(scenario);
	on_result(results);
	Ok(())
}
