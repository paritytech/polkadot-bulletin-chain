use anyhow::Result;
use std::sync::Arc;
use subxt::OnlineClient;
use subxt_signer::sr25519::Keypair;

use crate::{
	accounts::NonceTracker,
	chain_info::ChainLimits,
	client::BulletinConfig,
	pipeline::{self, IterationPlan, StressWorkItem},
	report::{ScenarioResult, SubmissionStats},
	store,
};

fn scenario_result_from_bulk(
	result: &store::BulkStoreResult,
	account_count: usize,
	payload_size: usize,
	label: &str,
	chain_limits: &ChainLimits,
) -> ScenarioResult {
	let all_blocks = &result.blocks;
	let measured: Vec<_> = all_blocks.iter().filter(|b| !b.prefill).collect();

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

	let onchain_duration_ms = match (
		steady.first().and_then(|b| b.timestamp_ms),
		steady.last().and_then(|b| b.timestamp_ms),
	) {
		(Some(t1), Some(t2)) if t2 > t1 => Some(t2 - t1),
		_ => None,
	};
	let (tps, bps, onchain_timing) = if let Some(ms) = onchain_duration_ms {
		let secs = ms as f64 / 1000.0;
		(measured_confirmed as f64 / secs, steady_bytes as f64 / secs, true)
	} else {
		let secs = result.duration.as_secs_f64();
		(measured_confirmed as f64 / secs, steady_bytes as f64 / secs, false)
	};

	log::info!(
		"block-cap: {} total blocks ({} prefill, {} measured), {} steady-state \
		 ({} empty), avg={:.1}, peak={}, timing={}",
		all_blocks.len(),
		prefill_count,
		measured.len(),
		steady.len(),
		empty_count,
		avg,
		peak,
		if onchain_timing { "on-chain" } else { "client" }
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
		}),
		avg_block_interval_ms,
		fork_detections: result.fork_detections,
		onchain_timing,
		total_reads: None,
		successful_reads: None,
		failed_reads: None,
		reads_per_sec: None,
		read_bytes_per_sec: None,
		data_verified: None,
	}
}

/// Block capacity measurement using one-shot accounts.
///
/// Each account submits exactly 1 tx at nonce 0 with its own random payload.
/// Delegates to `store::bulk_store_oneshot` for concurrent submission with
/// backpressure. Only steady-state blocks (excluding first and last) are used
/// for avg/peak.
#[allow(clippy::too_many_arguments)]
pub async fn run_block_capacity(
	_client: &OnlineClient<BulletinConfig>,
	signers: &[Keypair],
	payload_size: usize,
	target_blocks: u32,
	ws_urls: &[&str],
	chain_limits: &ChainLimits,
	submitters: usize,
	block_input: store::BlockInput,
) -> Result<ScenarioResult> {
	log::info!(
		"block-cap: Block capacity test ({} one-shot accounts, {payload_size} bytes, \
		 {target_blocks} target blocks)",
		signers.len()
	);

	// Each account gets its own random payload.
	let work_items: Vec<_> = signers
		.iter()
		.cloned()
		.map(|kp| (kp, Arc::new(store::generate_payload(payload_size))))
		.collect();

	let total_target = target_blocks + 2; // include ramp-up and ramp-down blocks
	let result =
		store::bulk_store_oneshot(work_items, ws_urls, Some(total_target), submitters, block_input)
			.await?;

	Ok(scenario_result_from_bulk(&result, signers.len(), payload_size, "single", chain_limits))
}

/// Block capacity measurement across multiple payload sizes.
///
/// For each payload size, splits one-shot accounts into iterations (~
/// [`crate::pipeline::BLOCK_CAPACITY_MEASURED_BLOCKS_PER_ITERATION`] measured blocks worth of txs
/// per iteration), then runs the producer/consumer pipeline:
/// [`StressWorkItem`]s on a bounded channel; the consumer waits on `txpool_status` before further
/// `recv`s when the pool is deep, so the generator blocks on `send`. Drains the pool between
/// variants.
#[allow(clippy::too_many_arguments)]
pub async fn run_block_capacity_sweep(
	client: &OnlineClient<BulletinConfig>,
	authorizer_signer: &Keypair,
	nonce_tracker: &NonceTracker,
	ws_urls: &[&str],
	chain_limits: &ChainLimits,
	submitters: usize,
	target_blocks: u32,
	variant_filter: Option<&str>,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
) -> Result<()> {
	let block_usable_bytes = chain_limits.normal_block_length as usize;
	let extrinsic_overhead = chain_limits.extrinsic_length_overhead as usize;
	let max_block_txs = chain_limits.max_block_transactions as usize;

	let all_payload_sizes: &[(usize, &str)] = &[
		(1024, "1KB"),
		(4096, "4KB"),
		(32 * 1024, "32KB"),
		(128 * 1024, "128KB"),
		(512 * 1024, "512KB"),
		(1024 * 1024, "1MB"),
		(2 * 1024 * 1024, "2MB"),
		(4 * 1024 * 1024, "4MB"),
		(5 * 1024 * 1024, "5MB"),
		(7 * 1024 * 1024, "7MB"),
		(7 * 1024 * 1024 + 512 * 1024, "7.5MB"),
		(2050 * 1024, "2050KB"),
		(8 * 1024 * 1024, "8MB"),
		(10 * 1024 * 1024, "10MB"),
	];

	// Filter variants if --variants was specified.
	let filter_set: Option<Vec<String>> = variant_filter.map(|f| {
		f.split(',')
			.map(|s| s.trim().to_uppercase())
			.filter(|s| !s.is_empty())
			.collect()
	});
	let payload_sizes: Vec<(usize, &str)> = all_payload_sizes
		.iter()
		.filter(|(_, label)| {
			filter_set
				.as_ref()
				.is_none_or(|set| set.iter().any(|f| f == &label.to_uppercase()))
		})
		.copied()
		.collect();

	if payload_sizes.is_empty() {
		let available: Vec<&str> = all_payload_sizes.iter().map(|(_, l)| *l).collect();
		anyhow::bail!(
			"No matching variants for filter {:?}. Available: {}",
			variant_filter.unwrap_or(""),
			available.join(", ")
		);
	}

	log::info!(
		"Running {} variant(s): {}",
		payload_sizes.len(),
		payload_sizes.iter().map(|(_, l)| *l).collect::<Vec<_>>().join(", ")
	);

	for &(payload_size, label) in &payload_sizes {
		let est_block_cap =
			(block_usable_bytes / (payload_size + extrinsic_overhead)).min(max_block_txs);
		// One-shot accounts: one account per transaction needed.
		// Need enough to fill all target blocks (plus ramp-up/down) plus a
		// backpressure buffer so the pool stays saturated while blocks drain.
		// Safety margin 1.5x: on multi-validator networks (Westend), BABE
		// rotation causes partial blocks (~70% avg utilization), so we need
		// more accounts than the theoretical estimate.
		let total_block_slots = (target_blocks + 2) as usize * est_block_cap;
		let backpressure_buffer = est_block_cap * 3;
		let accounts_needed = ((total_block_slots + backpressure_buffer) * 3 / 2).max(1) as u32;
		let est_pool_mb =
			(accounts_needed as usize * (payload_size + extrinsic_overhead)) / (1024 * 1024);
		let accounts_per_iter = pipeline::block_capacity_accounts_per_iteration(est_block_cap);
		let n_iterations = accounts_needed.div_ceil(accounts_per_iter);
		log::info!(
			"=== block-capacity variant: {label} payload, {accounts_needed} one-shot accounts \
			 in {n_iterations} iteration(s) (~{} measured blocks/iter × ~{est_block_cap} txs/block \
			 ≈ {accounts_per_iter} accounts/iter), est. pool demand ~{est_pool_mb} MB ===",
			pipeline::BLOCK_CAPACITY_MEASURED_BLOCKS_PER_ITERATION,
		);

		let seed = format!("T2sweep_{label}");
		let plans: Vec<IterationPlan> =
			pipeline::build_iteration_plans(accounts_needed, accounts_per_iter, &seed);
		let txpool_pause = pipeline::POOL_PENDING_PAUSE_THRESHOLD;
		log::info!(
			"{label}: iteration layout ready ({} iterations; continuous AuthorizeAndInitSlice + \
			 stores; txpool gate every {txpool_pause} dispatches (pause if pool > {txpool_pause}); \
			 authorize chunks of {})",
			plans.len(),
			crate::authorize::AUTHORIZE_BATCH_SIZE,
		);
		let variant_result: Result<ScenarioResult> = async {
			let dual = store::subscribe_blocks_dual(ws_urls[0]).await?;
			let (work_tx, work_rx) =
				tokio::sync::mpsc::channel::<StressWorkItem>(pipeline::WORK_CHANNEL_CAPACITY);

			let gen_plans = plans.clone();
			let gen_payload = payload_size;
			let gen_client = std::sync::Arc::new(client.clone());
			let generator = tokio::spawn(async move {
				pipeline::generate_block_capacity_work(work_tx, &gen_plans, gen_payload, gen_client)
					.await
			});

			let pipeline_out = pipeline::run_block_capacity_pipeline(
				work_rx,
				dual,
				ws_urls,
				submitters,
				payload_size,
				client,
				authorizer_signer,
				nonce_tracker,
			)
			.await;

			let bulk = match pipeline_out {
				Ok(b) => b,
				Err(e) => {
					generator.abort();
					let _ = generator.await;
					return Err(e);
				},
			};

			match generator.await {
				Ok(Ok(())) => {},
				Ok(Err(gen_e)) => return Err(gen_e),
				Err(join_e) =>
					return Err(anyhow::anyhow!("block-capacity generator task failed: {join_e}")),
			}

			let total_accounts: u32 = plans.iter().map(|p| p.account_count).sum();
			let result = scenario_result_from_bulk(
				&bulk,
				total_accounts as usize,
				payload_size,
				label,
				chain_limits,
			);

			if payload_size >= 4 * 1024 * 1024 && result.total_confirmed == 0 {
				log::warn!(
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
				log::error!("{label}: variant failed: {e}");
				results.push(ScenarioResult {
					name: format!("block-cap: Block Capacity ({label}) — ERROR"),
					duration: std::time::Duration::ZERO,
					total_submitted: 0,
					total_confirmed: 0,
					total_errors: 0,
					payload_size,
					throughput_tps: 0.0,
					throughput_bytes_per_sec: 0.0,
					avg_tx_per_block: 0.0,
					peak_tx_per_block: 0,
					inclusion_latency: None,
					finalization_latency: None,
					retrieval_latency: None,
					theoretical: Some(chain_limits.compute_theoretical_limits(payload_size)),
					chain_limits: None,
					environment: None,
					blocks: vec![],
					submission_stats: None,
					avg_block_interval_ms: None,
					fork_detections: 0,
					onchain_timing: false,
					total_reads: None,
					successful_reads: None,
					failed_reads: None,
					reads_per_sec: None,
					read_bytes_per_sec: None,
					data_verified: None,
				});
				on_result(results);
				// Try to continue with the next variant; if the client connection is
				// dead, the next authorize call will fail fast.
			},
		}

		// Drain the transaction pool before the next variant.
		log::info!("Draining pool after {label} variant...");
		if let Ok(mut blocks_sub) = client.blocks().subscribe_best().await {
			let mut consecutive_empty = 0u32;
			let drain_deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
			loop {
				if std::time::Instant::now() > drain_deadline {
					log::warn!("Pool drain timed out after 120s, continuing");
					break;
				}
				if let Some(Ok(block)) = blocks_sub.next().await {
					let stored_count = block
						.events()
						.await
						.map(|events| {
							events
								.iter()
								.filter(|e| {
									e.as_ref().is_ok_and(|ev| {
										ev.pallet_name() == "TransactionStorage" &&
											ev.variant_name() == "Stored"
									})
								})
								.count()
						})
						.unwrap_or(0);
					if stored_count == 0 {
						consecutive_empty += 1;
						if consecutive_empty >= 2 {
							log::info!(
								"Pool drained (2 consecutive empty blocks, last #{}) \
								 - safe for next variant",
								block.number()
							);
							break;
						}
					} else {
						consecutive_empty = 0;
						log::info!(
							"Block #{} still has {stored_count} Stored events, waiting...",
							block.number()
						);
					}
				}
			}
		} else {
			log::warn!("Could not subscribe to blocks for pool drain, skipping");
		}
	}

	Ok(())
}
