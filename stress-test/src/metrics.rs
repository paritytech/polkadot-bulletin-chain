// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Prometheus metrics for long-running stress runs (`--prometheus-port`).
//!
//! [`PrometheusMetrics`] is a concrete recorder reached through the process-wide [`metrics`]
//! accessor — one recorder per process, never optional, so there is nothing to thread through the
//! scenario drivers. Hot paths increment live counters at event time (submission, block
//! confirmation, read completion) so a scraping Prometheus sees the run as a live time series that
//! can be correlated with node-side metrics (`substrate_sub_txpool_*`, `substrate_proposer_*`,
//! ...). End-of-variant summaries from [`ScenarioResult`] are mirrored into
//! `bulletin_stress_result_*` gauges for "last run" panels.
//!
//! All metrics carry a `variant` label (payload-size labels like `1KB` for the block-capacity
//! sweep, scenario slugs like `hop-full-cycle` otherwise) so one metric shape serves every test
//! variant. Tests construct a throwaway recorder with [`PrometheusMetrics::for_tests`].
//!
//! # Write-path model
//!
//! The write path is counted along two independent axes.
//!
//! **Unique transactions.** `tx_offered_*` (the tool starts trying to place this extrinsic),
//! `tx_accepted_*` (a pool took it), `tx_abandoned_*` (the worker stopped trying) and
//! `tx_confirmed_*` (it appeared in a finalized block) are incremented **exactly once per
//! extrinsic**, so each is a count of transactions, never of events.
//!
//! **Attempts.** `submit_attempts_total` is incremented once per `author_submitExtrinsic` RPC
//! call, with `outcome=accepted` or the failure class, so one transaction that needed five tries
//! contributes five samples.
//!
//! The two axes give these invariants:
//!
//! ```text
//! attempts >= offered                          // equality ⇒ no retries at all
//! retries per transaction = attempts / offered - 1
//! offered = accepted + abandoned + in-flight    // in-flight = still inside StoreWorker::submit
//! ```
//!
//! Abandoned is not a synonym for lost data: `reason="already_imported"` means the node already
//! had the transaction, and it is grouped under abandoned only because this worker stops tracking
//! it from that point on.
//!
//! Every write-path `_bytes_total` counts **uncompressed payload** bytes, so offered / accepted /
//! abandoned / confirmed bytes are directly comparable and converge under zero loss. Wire size is
//! kept separately as `tx_accepted_encoded_bytes_total` (SCALE-encoded extrinsic, i.e. payload
//! plus signature and call overhead).
//!
//! [`serve`] binds a hyper exposition server for the global recorder's registry.

use anyhow::{Context, Result};
use std::{
	net::SocketAddr,
	sync::OnceLock,
	time::{Duration, SystemTime, UNIX_EPOCH},
};
use substrate_prometheus_endpoint::{
	prometheus::core::{Atomic, Collector},
	register, CounterVec, Gauge, GaugeVec, HistogramOpts, HistogramVec, Opts, PrometheusError,
	Registry, F64, U64,
};

use crate::report::{LatencyStats, ScenarioResult};

/// Buckets for the per-block transaction-count histogram. The runtime's hard extrinsic count
/// limit (`MaxBlockTransactions`) is 512, so the top bucket is the full block.
const BLOCK_TXS_BUCKETS: &[f64] =
	&[1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 192.0, 256.0, 384.0, 512.0];

/// Buckets for the per-block payload-size histogram, in bytes. Coarse below 4 MB, then tight
/// around the 8–9 MB band where production blocks (versi `bc-3000`: 8.4–8.8 MB) sit, so "full" and
/// "short" blocks land in different buckets.
const BLOCK_BYTES_BUCKETS: &[f64] = &[
	65_536.0,
	262_144.0,
	1_048_576.0,
	2_097_152.0,
	4_194_304.0,
	6_291_456.0,
	7_340_032.0,
	8_388_608.0,
	8_912_896.0,
	9_437_184.0,
	10_485_760.0,
	12_582_912.0,
];

/// Buckets for end-to-end latency histograms in seconds. Retrieval latencies are sub-second on a
/// warm bitswap peer; inclusion latencies span multiple block intervals under pool saturation.
const LATENCY_BUCKETS_SECS: &[f64] = &[
	0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 3.0, 4.5, 6.0, 9.0, 12.0, 18.0, 24.0, 36.0, 60.0, 120.0,
];

/// `outcome` label of `bulletin_stress_submit_attempts_total` for a successful submission RPC. The
/// failure values are [`TxPoolError::metric_class`](crate::store::TxPoolError::metric_class).
pub const OUTCOME_ACCEPTED: &str = "accepted";

static METRICS: OnceLock<PrometheusMetrics> = OnceLock::new();

/// The process-wide recorder. Registration happens on first use and cannot fail: every metric name,
/// label and bucket list is a literal registered on a fresh [`Registry`].
pub fn metrics() -> &'static PrometheusMetrics {
	METRICS.get_or_init(|| {
		PrometheusMetrics::new().expect("metric registration on a fresh registry cannot fail")
	})
}

/// How a run ended. Mapped to the `outcome` label of `bulletin_stress_runs_total`.
#[derive(Debug, Clone, Copy)]
pub enum RunOutcome {
	/// Ran to completion without an error.
	Ok,
	/// The command returned an error; in loop mode the next run still starts.
	Failed,
	/// Interrupted by Ctrl+C — partial results were flushed, so this is neither a clean run
	/// nor a failure and must not pollute either series.
	Cancelled,
}

impl RunOutcome {
	fn as_label(self) -> &'static str {
		match self {
			Self::Ok => "ok",
			Self::Failed => "failed",
			Self::Cancelled => "cancelled",
		}
	}
}

/// Which end-to-end duration a latency observation measures. Mapped to the `kind` label.
#[derive(Debug, Clone, Copy)]
pub enum LatencyKind {
	/// Submit → transaction seen in a (best) block.
	Inclusion,
	/// Submit → transaction seen in a finalized block.
	Finalization,
	/// Read request → payload received (bitswap / HOP claim).
	Retrieval,
}

impl LatencyKind {
	fn as_label(self) -> &'static str {
		match self {
			Self::Inclusion => "inclusion",
			Self::Finalization => "finalization",
			Self::Retrieval => "retrieval",
		}
	}
}

/// Concrete Prometheus recorder shared by all scenarios of one process.
pub struct PrometheusMetrics {
	registry: Registry,
	// Live write path, unique transactions (each incremented exactly once per extrinsic).
	tx_offered: CounterVec<U64>,
	tx_offered_bytes: CounterVec<U64>,
	tx_accepted: CounterVec<U64>,
	tx_accepted_bytes: CounterVec<U64>,
	tx_accepted_encoded_bytes: CounterVec<U64>,
	tx_abandoned: CounterVec<U64>,
	tx_abandoned_bytes: CounterVec<U64>,
	tx_confirmed: CounterVec<U64>,
	tx_confirmed_bytes: CounterVec<U64>,
	// Live write path, one increment per RPC call.
	submit_attempts: CounterVec<U64>,
	// Observed blocks.
	blocks_observed: CounterVec<U64>,
	block_txs: HistogramVec,
	block_bytes: HistogramVec,
	// Live latency distributions.
	latency: HistogramVec,
	// Live read path (bitswap).
	reads: CounterVec<U64>,
	read_bytes: CounterVec<U64>,
	// Run lifecycle.
	runs: CounterVec<U64>,
	run_in_progress: Gauge<U64>,
	run_start_timestamp: Gauge<F64>,
	variant_active: GaugeVec<U64>,
	// End-of-variant summaries (mirrors of `ScenarioResult`).
	result_throughput_tps: GaugeVec<F64>,
	result_throughput_bps: GaugeVec<F64>,
	result_avg_tx_per_block: GaugeVec<F64>,
	result_peak_tx_per_block: GaugeVec<U64>,
	result_latency: GaugeVec<F64>,
	result_reads_per_sec: GaugeVec<F64>,
	result_read_bytes_per_sec: GaugeVec<F64>,
}

fn reg<M: Clone + Collector + 'static>(
	registry: &Registry,
	name: &str,
	metric: Result<M, PrometheusError>,
) -> Result<M> {
	metric
		.and_then(|m| register(m, registry))
		.with_context(|| format!("failed to register {name}"))
}

fn counter_vec<T: Atomic + 'static>(
	registry: &Registry,
	name: &str,
	help: &str,
	labels: &[&str],
) -> Result<CounterVec<T>> {
	reg(registry, name, CounterVec::new(Opts::new(name, help), labels))
}

fn gauge<T: Atomic + 'static>(registry: &Registry, name: &str, help: &str) -> Result<Gauge<T>> {
	reg(registry, name, Gauge::new(name, help))
}

fn gauge_vec<T: Atomic + 'static>(
	registry: &Registry,
	name: &str,
	help: &str,
	labels: &[&str],
) -> Result<GaugeVec<T>> {
	reg(registry, name, GaugeVec::new(Opts::new(name, help), labels))
}

fn histogram_vec(
	registry: &Registry,
	name: &str,
	help: &str,
	labels: &[&str],
	buckets: &[f64],
) -> Result<HistogramVec> {
	reg(
		registry,
		name,
		HistogramVec::new(HistogramOpts::new(name, help).buckets(buckets.to_vec()), labels),
	)
}

impl PrometheusMetrics {
	/// Register every metric family on a fresh [`Registry`]. Use [`metrics`] outside tests.
	fn new() -> Result<Self> {
		let r = Registry::new();

		Ok(Self {
			tx_offered: counter_vec(
				&r,
				"bulletin_stress_tx_offered_total",
				"Store extrinsics the tool started trying to place, counted once per extrinsic \
				 before its first submission attempt (offered load).",
				&["variant"],
			)?,
			tx_offered_bytes: counter_vec(
				&r,
				"bulletin_stress_tx_offered_bytes_total",
				"Uncompressed payload bytes offered, counted once per extrinsic.",
				&["variant"],
			)?,
			tx_accepted: counter_vec(
				&r,
				"bulletin_stress_tx_accepted_total",
				"Store extrinsics accepted into a node's transaction pool, counted once per \
				 extrinsic (accepted ≤ offered).",
				&["variant"],
			)?,
			tx_accepted_bytes: counter_vec(
				&r,
				"bulletin_stress_tx_accepted_bytes_total",
				"Uncompressed payload bytes accepted into a transaction pool.",
				&["variant"],
			)?,
			tx_accepted_encoded_bytes: counter_vec(
				&r,
				"bulletin_stress_tx_accepted_encoded_bytes_total",
				"SCALE-encoded extrinsic bytes accepted into a transaction pool (wire size; \
				 payload plus signature and call overhead).",
				&["variant"],
			)?,
			tx_abandoned: counter_vec(
				&r,
				"bulletin_stress_tx_abandoned_total",
				"Store extrinsics the worker stopped tracking, counted once per extrinsic, by \
				 reason (already_imported means the node already had it, not data loss).",
				&["variant", "reason"],
			)?,
			tx_abandoned_bytes: counter_vec(
				&r,
				"bulletin_stress_tx_abandoned_bytes_total",
				"Uncompressed payload bytes of abandoned extrinsics, by reason.",
				&["variant", "reason"],
			)?,
			submit_attempts: counter_vec(
				&r,
				"bulletin_stress_submit_attempts_total",
				"Submission RPC calls, one increment per call, by outcome (accepted, pool_full, \
				 banned, ...); attempts ≥ offered, the excess being retries.",
				&["variant", "outcome"],
			)?,
			tx_confirmed: counter_vec(
				&r,
				"bulletin_stress_tx_confirmed_total",
				"Store transactions confirmed in finalized blocks observed by the block monitor.",
				&["variant"],
			)?,
			tx_confirmed_bytes: counter_vec(
				&r,
				"bulletin_stress_tx_confirmed_bytes_total",
				"Uncompressed payload bytes confirmed in finalized blocks.",
				&["variant"],
			)?,
			blocks_observed: counter_vec(
				&r,
				"bulletin_stress_blocks_observed_total",
				"Finalized blocks observed by the block monitor during a variant (including empty \
				 ones).",
				&["variant"],
			)?,
			block_txs: histogram_vec(
				&r,
				"bulletin_stress_block_txs",
				"Store transactions per observed finalized block (block fullness distribution).",
				&["variant"],
				BLOCK_TXS_BUCKETS,
			)?,
			block_bytes: histogram_vec(
				&r,
				"bulletin_stress_block_bytes",
				"Uncompressed payload bytes per observed finalized block. Substrate/Cumulus expose \
				 no block-size-in-bytes metric (only substrate_proposer_number_of_transactions and \
				 substrate_block_height), so this is the only source of a per-block size \
				 distribution.",
				&["variant"],
				BLOCK_BYTES_BUCKETS,
			)?,
			latency: histogram_vec(
				&r,
				"bulletin_stress_latency_seconds",
				"End-to-end latency by kind (inclusion, finalization, retrieval).",
				&["variant", "kind"],
				LATENCY_BUCKETS_SECS,
			)?,
			reads: counter_vec(
				&r,
				"bulletin_stress_reads_total",
				"Bitswap read attempts, partitioned by outcome.",
				&["variant", "outcome"],
			)?,
			read_bytes: counter_vec(
				&r,
				"bulletin_stress_read_bytes_total",
				"Bytes downloaded via bitswap reads.",
				&["variant"],
			)?,
			runs: counter_vec(
				&r,
				"bulletin_stress_runs_total",
				"Completed stress runs, partitioned by outcome (one run = one command invocation \
				 or one --loop-interval-secs iteration).",
				&["outcome"],
			)?,
			run_in_progress: gauge(
				&r,
				"bulletin_stress_run_in_progress",
				"1 while a stress run is executing, 0 between --loop-interval-secs iterations.",
			)?,
			run_start_timestamp: gauge(
				&r,
				"bulletin_stress_run_start_timestamp_seconds",
				"Unix timestamp of the most recent run start.",
			)?,
			variant_active: gauge_vec(
				&r,
				"bulletin_stress_variant_active",
				"1 while the labeled variant is running (timeline segmentation for dashboards).",
				&["variant"],
			)?,
			result_throughput_tps: gauge_vec(
				&r,
				"bulletin_stress_result_throughput_tx_per_sec",
				"Measured throughput of the variant's last completed run (tx/s).",
				&["variant"],
			)?,
			result_throughput_bps: gauge_vec(
				&r,
				"bulletin_stress_result_throughput_bytes_per_sec",
				"Measured throughput of the variant's last completed run (payload bytes/s).",
				&["variant"],
			)?,
			result_avg_tx_per_block: gauge_vec(
				&r,
				"bulletin_stress_result_avg_tx_per_block",
				"Average store transactions per measured block in the variant's last completed \
				 run.",
				&["variant"],
			)?,
			result_peak_tx_per_block: gauge_vec(
				&r,
				"bulletin_stress_result_peak_tx_per_block",
				"Peak store transactions in a single block in the variant's last completed run.",
				&["variant"],
			)?,
			result_latency: gauge_vec(
				&r,
				"bulletin_stress_result_latency_seconds",
				"Latency summary of the variant's last completed run, by kind and quantile.",
				&["variant", "kind", "quantile"],
			)?,
			result_reads_per_sec: gauge_vec(
				&r,
				"bulletin_stress_result_reads_per_sec",
				"Read throughput of the variant's last completed run (successful reads/s).",
				&["variant"],
			)?,
			result_read_bytes_per_sec: gauge_vec(
				&r,
				"bulletin_stress_result_read_bytes_per_sec",
				"Read bandwidth of the variant's last completed run (bytes/s).",
				&["variant"],
			)?,
			registry: r,
		})
	}

	/// Convenience constructor for tests.
	#[cfg(test)]
	pub fn for_tests() -> Self {
		Self::new().expect("metric registration on a fresh registry never fails")
	}

	// ---- run lifecycle ---------------------------------------------------

	/// Mark a run (one command invocation / loop iteration) as started.
	pub fn run_started(&self) {
		self.run_in_progress.set(1);
		let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
		self.run_start_timestamp.set(now);
	}

	/// Mark the current run as finished and count its outcome.
	pub fn run_finished(&self, outcome: RunOutcome) {
		self.run_in_progress.set(0);
		self.runs.with_label_values(&[outcome.as_label()]).inc();
	}

	/// Flag `variant` as (in)active — drives dashboard timeline panels.
	pub fn set_variant_active(&self, variant: &str, active: bool) {
		self.variant_active.with_label_values(&[variant]).set(u64::from(active));
	}

	// ---- live write path ---------------------------------------------------

	/// Record one store extrinsic of `payload_bytes` offered to the network. Call once per
	/// extrinsic, before its first submission attempt.
	pub fn inc_offered(&self, variant: &str, payload_bytes: u64) {
		self.tx_offered.with_label_values(&[variant]).inc();
		self.tx_offered_bytes.with_label_values(&[variant]).inc_by(payload_bytes);
	}

	/// Record one store extrinsic accepted into a transaction pool. Call once per extrinsic, on the
	/// attempt that succeeded. `encoded_bytes` is the wire size of the same extrinsic.
	pub fn inc_accepted(&self, variant: &str, payload_bytes: u64, encoded_bytes: u64) {
		self.tx_accepted.with_label_values(&[variant]).inc();
		self.tx_accepted_bytes.with_label_values(&[variant]).inc_by(payload_bytes);
		self.tx_accepted_encoded_bytes
			.with_label_values(&[variant])
			.inc_by(encoded_bytes);
	}

	/// Record one store extrinsic the worker gave up on. Call once per extrinsic, on the path that
	/// stops retrying it.
	pub fn inc_abandoned(&self, variant: &str, reason: &'static str, payload_bytes: u64) {
		self.tx_abandoned.with_label_values(&[variant, reason]).inc();
		self.tx_abandoned_bytes
			.with_label_values(&[variant, reason])
			.inc_by(payload_bytes);
	}

	/// Record one submission RPC call and how it ended ([`OUTCOME_ACCEPTED`] or an error class).
	pub fn inc_submit_attempt(&self, variant: &str, outcome: &'static str) {
		self.submit_attempts.with_label_values(&[variant, outcome]).inc();
	}

	/// Record one finalized block observed by the block monitor: its store transactions and their
	/// uncompressed payload bytes, as both running totals and per-block distributions.
	pub fn observe_confirmed_block(&self, variant: &str, tx_count: u64, payload_bytes: u64) {
		self.tx_confirmed.with_label_values(&[variant]).inc_by(tx_count);
		self.tx_confirmed_bytes.with_label_values(&[variant]).inc_by(payload_bytes);
		self.blocks_observed.with_label_values(&[variant]).inc();
		self.block_txs.with_label_values(&[variant]).observe(tx_count as f64);
		self.block_bytes.with_label_values(&[variant]).observe(payload_bytes as f64);
	}

	// ---- latency ----------------------------------------------------------

	/// Record a single end-to-end latency observation.
	pub fn observe_latency(&self, variant: &str, kind: LatencyKind, duration: Duration) {
		self.observe_latency_repeated(variant, kind, duration, 1);
	}

	/// Record the same latency `count` times (batched reads that share one measured duration).
	pub fn observe_latency_repeated(
		&self,
		variant: &str,
		kind: LatencyKind,
		duration: Duration,
		count: usize,
	) {
		let hist = self.latency.with_label_values(&[variant, kind.as_label()]);
		let secs = duration.as_secs_f64();
		for _ in 0..count {
			hist.observe(secs);
		}
	}

	/// Record a batch of latency observations (scenarios that collect `Vec<Duration>`).
	pub fn observe_latencies(&self, variant: &str, kind: LatencyKind, durations: &[Duration]) {
		let hist = self.latency.with_label_values(&[variant, kind.as_label()]);
		for d in durations {
			hist.observe(d.as_secs_f64());
		}
	}

	// ---- live read path -----------------------------------------------------

	/// Record `count` bitswap read completions and `bytes` downloaded.
	pub fn inc_reads(&self, variant: &str, ok: bool, count: u64, bytes: u64) {
		self.reads
			.with_label_values(&[variant, if ok { "success" } else { "failure" }])
			.inc_by(count);
		if bytes > 0 {
			self.read_bytes.with_label_values(&[variant]).inc_by(bytes);
		}
	}

	// ---- end-of-variant summaries -------------------------------------------

	/// Mirror a completed [`ScenarioResult`] into the `bulletin_stress_result_*` gauges.
	pub fn record_result(&self, result: &ScenarioResult) {
		let variant = result.variant.as_str();
		self.result_throughput_tps
			.with_label_values(&[variant])
			.set(result.throughput_tps);
		self.result_throughput_bps
			.with_label_values(&[variant])
			.set(result.throughput_bytes_per_sec);
		self.result_avg_tx_per_block
			.with_label_values(&[variant])
			.set(result.avg_tx_per_block);
		self.result_peak_tx_per_block
			.with_label_values(&[variant])
			.set(result.peak_tx_per_block);
		for (kind, stats) in [
			(LatencyKind::Inclusion, &result.inclusion_latency),
			(LatencyKind::Finalization, &result.finalization_latency),
			(LatencyKind::Retrieval, &result.retrieval_latency),
		] {
			if let Some(stats) = stats {
				self.set_result_latency(variant, kind, stats);
			}
		}
		if let Some(v) = result.reads_per_sec {
			self.result_reads_per_sec.with_label_values(&[variant]).set(v);
		}
		if let Some(v) = result.read_bytes_per_sec {
			self.result_read_bytes_per_sec.with_label_values(&[variant]).set(v);
		}
	}

	fn set_result_latency(&self, variant: &str, kind: LatencyKind, stats: &LatencyStats) {
		let quantiles = [
			("p50", stats.p50),
			("p90", stats.p90),
			("p99", stats.p99),
			("min", stats.min),
			("max", stats.max),
			("mean", stats.mean),
		];
		for (quantile, value) in quantiles {
			self.result_latency
				.with_label_values(&[variant, kind.as_label(), quantile])
				.set(value.as_secs_f64());
		}
	}
}

/// Serve `/metrics` for the global recorder on `addr`. The returned future runs until the
/// underlying hyper server exits (typically: never).
pub async fn serve(addr: SocketAddr) -> Result<()> {
	substrate_prometheus_endpoint::init_prometheus(addr, metrics().registry.clone())
		.await
		.map_err(|e| anyhow::anyhow!("prometheus exposition server failed: {e}"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use substrate_prometheus_endpoint::prometheus::{Encoder, TextEncoder};

	fn encode(m: &PrometheusMetrics) -> String {
		let mut buf = Vec::new();
		TextEncoder::new().encode(&m.registry.gather(), &mut buf).expect("encode");
		String::from_utf8(buf).expect("utf8")
	}

	#[test]
	fn registered_families_appear_in_encoded_output() {
		let m = PrometheusMetrics::for_tests();
		// Trigger one observation per family so each has a series.
		m.inc_offered("1KB", 1024);
		m.inc_submit_attempt("1KB", "pool_full");
		m.inc_submit_attempt("1KB", OUTCOME_ACCEPTED);
		m.inc_accepted("1KB", 1024, 1234);
		m.inc_abandoned("1KB", "dropped", 1024);
		m.observe_confirmed_block("1KB", 100, 100 * 1024);
		m.observe_latency("hop-full-cycle", LatencyKind::Inclusion, Duration::from_secs(6));
		m.inc_reads("bitswap-bulk-read", true, 3, 3 * 128 * 1024);
		m.run_started();
		m.run_finished(RunOutcome::Ok);
		m.set_variant_active("1KB", true);
		m.record_result(&ScenarioResult {
			name: "block-cap: Block Capacity (1KB)".into(),
			variant: "1KB".into(),
			throughput_tps: 84.5,
			throughput_bytes_per_sec: 86528.0,
			avg_tx_per_block: 500.0,
			peak_tx_per_block: 512,
			inclusion_latency: Some(LatencyStats {
				p50: Duration::from_secs(6),
				p90: Duration::from_secs(12),
				p99: Duration::from_secs(18),
				min: Duration::from_secs(3),
				max: Duration::from_secs(24),
				mean: Duration::from_secs(7),
			}),
			..Default::default()
		});

		let txt = encode(&m);
		for expected in [
			"bulletin_stress_tx_offered_total",
			"bulletin_stress_tx_offered_bytes_total",
			"bulletin_stress_tx_accepted_total",
			"bulletin_stress_tx_accepted_bytes_total",
			"bulletin_stress_tx_accepted_encoded_bytes_total",
			"bulletin_stress_tx_abandoned_total",
			"bulletin_stress_tx_abandoned_bytes_total",
			"bulletin_stress_submit_attempts_total",
			"bulletin_stress_tx_confirmed_total",
			"bulletin_stress_tx_confirmed_bytes_total",
			"bulletin_stress_blocks_observed_total",
			"bulletin_stress_block_txs",
			"bulletin_stress_block_bytes",
			"bulletin_stress_latency_seconds",
			"bulletin_stress_reads_total",
			"bulletin_stress_read_bytes_total",
			"bulletin_stress_runs_total",
			"bulletin_stress_run_in_progress",
			"bulletin_stress_run_start_timestamp_seconds",
			"bulletin_stress_variant_active",
			"bulletin_stress_result_throughput_tx_per_sec",
			"bulletin_stress_result_throughput_bytes_per_sec",
			"bulletin_stress_result_avg_tx_per_block",
			"bulletin_stress_result_peak_tx_per_block",
			"bulletin_stress_result_latency_seconds",
		] {
			assert!(txt.contains(expected), "expected family {expected} in:\n{txt}");
		}
	}

	/// Three transactions of a 1 KB variant: one accepted on its third attempt, one abandoned as
	/// dropped on its second, one still inside `submit`. Checks that the unique-transaction
	/// families count transactions (not events) while attempts counts RPC calls.
	#[test]
	fn unique_tx_counters_are_incremented_once_per_transaction() {
		let m = PrometheusMetrics::for_tests();
		for _ in 0..3 {
			m.inc_offered("1KB", 1024);
		}
		// tx1: pool_full, pool_full, accepted.
		m.inc_submit_attempt("1KB", "pool_full");
		m.inc_submit_attempt("1KB", "pool_full");
		m.inc_submit_attempt("1KB", OUTCOME_ACCEPTED);
		m.inc_accepted("1KB", 1024, 1220);
		// tx2: banned, dropped → abandoned.
		m.inc_submit_attempt("1KB", "banned");
		m.inc_submit_attempt("1KB", "dropped");
		m.inc_abandoned("1KB", "dropped", 1024);
		// tx3: one attempt so far, still retrying (in-flight).
		m.inc_submit_attempt("1KB", "pool_full");

		let txt = encode(&m);
		// offered = accepted + abandoned + in-flight  →  3 = 1 + 1 + 1.
		assert!(txt.contains("bulletin_stress_tx_offered_total{variant=\"1KB\"} 3"));
		assert!(txt.contains("bulletin_stress_tx_accepted_total{variant=\"1KB\"} 1"));
		assert!(txt
			.contains("bulletin_stress_tx_abandoned_total{reason=\"dropped\",variant=\"1KB\"} 1"));
		// attempts (6) >= offered (3): three retries across the three transactions.
		assert!(txt.contains(
			"bulletin_stress_submit_attempts_total{outcome=\"pool_full\",variant=\"1KB\"} 3"
		));
		assert!(txt.contains(
			"bulletin_stress_submit_attempts_total{outcome=\"banned\",variant=\"1KB\"} 1"
		));
		assert!(txt.contains(
			"bulletin_stress_submit_attempts_total{outcome=\"dropped\",variant=\"1KB\"} 1"
		));
		assert!(txt.contains(
			"bulletin_stress_submit_attempts_total{outcome=\"accepted\",variant=\"1KB\"} 1"
		));
		// Payload bytes follow the same partition; encoded bytes carry the wire overhead.
		assert!(txt.contains("bulletin_stress_tx_offered_bytes_total{variant=\"1KB\"} 3072"));
		assert!(txt.contains("bulletin_stress_tx_accepted_bytes_total{variant=\"1KB\"} 1024"));
		assert!(
			txt.contains("bulletin_stress_tx_accepted_encoded_bytes_total{variant=\"1KB\"} 1220")
		);
		assert!(txt.contains(
			"bulletin_stress_tx_abandoned_bytes_total{reason=\"dropped\",variant=\"1KB\"} 1024"
		));
	}

	#[test]
	fn abandon_reasons_are_separate_series() {
		let m = PrometheusMetrics::for_tests();
		m.inc_abandoned("1KB", "dropped", 1024);
		m.inc_abandoned("1KB", "already_imported", 1024);
		m.inc_abandoned("1KB", "already_imported", 1024);
		m.inc_abandoned("32KB", "connection_dead", 32 * 1024);

		let txt = encode(&m);
		assert!(txt
			.contains("bulletin_stress_tx_abandoned_total{reason=\"dropped\",variant=\"1KB\"} 1"));
		assert!(txt.contains(
			"bulletin_stress_tx_abandoned_total{reason=\"already_imported\",variant=\"1KB\"} 2"
		));
		assert!(txt.contains(
			"bulletin_stress_tx_abandoned_total{reason=\"connection_dead\",variant=\"32KB\"} 1"
		));
		assert!(txt.contains(
			"bulletin_stress_tx_abandoned_bytes_total{reason=\"connection_dead\",variant=\"32KB\"} 32768"
		));
	}

	/// Cumulative count of `{family}_bucket{le="{le}"}` in the encoded output.
	fn bucket_count(txt: &str, family: &str, le: &str) -> u64 {
		let (prefix, le) = (format!("{family}_bucket"), format!("le=\"{le}\""));
		txt.lines()
			.find(|l| l.starts_with(&prefix) && l.contains(&le))
			.and_then(|l| l.rsplit(' ').next())
			.and_then(|v| v.parse().ok())
			.unwrap_or_else(|| panic!("no {family} bucket {le} in:\n{txt}"))
	}

	#[test]
	fn confirmed_block_updates_all_block_families() {
		let m = PrometheusMetrics::for_tests();
		m.observe_confirmed_block("1KB", 500, 512_000);
		m.observe_confirmed_block("1KB", 0, 0);
		// A near-full block, in the band the byte buckets resolve.
		m.observe_confirmed_block("1KB", 480, 8_650_000);

		let txt = encode(&m);
		assert!(txt.contains("bulletin_stress_tx_confirmed_total{variant=\"1KB\"} 980"));
		assert!(txt.contains("bulletin_stress_tx_confirmed_bytes_total{variant=\"1KB\"} 9162000"));
		assert!(txt.contains("bulletin_stress_blocks_observed_total{variant=\"1KB\"} 3"));
		// All blocks fall in the +Inf tx bucket; the empty one also in the first (le="1").
		assert_eq!(bucket_count(&txt, "bulletin_stress_block_txs", "+Inf"), 3);
		assert_eq!(bucket_count(&txt, "bulletin_stress_block_txs", "1"), 1);
		// The 8.25 MiB block is above the 8 MiB bucket and at or below the 8.5 MiB one.
		assert_eq!(bucket_count(&txt, "bulletin_stress_block_bytes", "8388608"), 2);
		assert_eq!(bucket_count(&txt, "bulletin_stress_block_bytes", "8912896"), 3);
	}

	#[test]
	fn run_outcomes_are_separate_series() {
		let m = PrometheusMetrics::for_tests();
		m.run_finished(RunOutcome::Ok);
		m.run_finished(RunOutcome::Ok);
		m.run_finished(RunOutcome::Failed);
		m.run_finished(RunOutcome::Cancelled);

		let txt = encode(&m);
		assert!(txt.contains("bulletin_stress_runs_total{outcome=\"ok\"} 2"));
		assert!(txt.contains("bulletin_stress_runs_total{outcome=\"failed\"} 1"));
		assert!(txt.contains("bulletin_stress_runs_total{outcome=\"cancelled\"} 1"));
	}

	#[test]
	fn repeated_latency_observations_share_one_series() {
		let m = PrometheusMetrics::for_tests();
		m.observe_latency_repeated(
			"bitswap-bulk-read",
			LatencyKind::Retrieval,
			Duration::from_millis(40),
			3,
		);

		let txt = encode(&m);
		assert!(txt.contains(
			"bulletin_stress_latency_seconds_count{kind=\"retrieval\",variant=\"bitswap-bulk-read\"} 3"
		));
	}
}
