// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Prometheus metrics for long-running stress runs (`--prometheus-port`).
//!
//! [`PrometheusMetrics`] is a concrete recorder (no trait, no `Option<_>` plumbing) constructed
//! once at startup and threaded into the scenario drivers. Hot paths increment live counters at
//! event time (submission, block confirmation, read completion) so a scraping Prometheus sees the
//! run as a live time series that can be correlated with node-side metrics
//! (`substrate_sub_txpool_*`, `substrate_proposer_*`, ...). End-of-variant summaries from
//! [`ScenarioResult`] are mirrored into `bulletin_stress_result_*` gauges for "last run" panels.
//!
//! All metrics carry a `variant` label (payload-size labels like `1KB` for the block-capacity
//! sweep, scenario slugs like `hop-full-cycle` otherwise) so one metric shape serves every test
//! variant. Tests construct a throwaway recorder with [`PrometheusMetrics::for_tests`].
//!
//! [`serve`] is a thin wrapper around `substrate_prometheus_endpoint::init_prometheus` that binds
//! a hyper exposition server to a socket address.

use anyhow::{Context, Result};
use std::{
	net::SocketAddr,
	time::{Duration, SystemTime, UNIX_EPOCH},
};
use substrate_prometheus_endpoint::{
	register, CounterVec, Gauge, GaugeVec, HistogramOpts, HistogramVec, Opts, Registry, F64, U64,
};

use crate::report::{LatencyStats, ScenarioResult};

/// Buckets for the per-block transaction-count histogram. The runtime's hard extrinsic count
/// limit (`MaxBlockTransactions`) is 512, so the top bucket is the full block.
pub const BLOCK_TXS_BUCKETS: &[f64] =
	&[1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 192.0, 256.0, 384.0, 512.0];

/// Buckets for end-to-end latency histograms in seconds. Inclusion/retrieval latencies span from
/// sub-second (bitswap reads) to multiple block intervals under pool saturation.
pub const LATENCY_BUCKETS_SECS: &[f64] =
	&[0.25, 0.5, 1.0, 2.0, 3.0, 4.5, 6.0, 9.0, 12.0, 18.0, 24.0, 36.0, 60.0, 120.0];

const OUTCOME_OK: &str = "ok";
const OUTCOME_FAILED: &str = "failed";
const READ_SUCCESS: &str = "success";
const READ_FAILURE: &str = "failure";

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
	// Live write path (incremented at event time).
	tx_submitted: CounterVec<U64>,
	tx_submitted_bytes: CounterVec<U64>,
	tx_errors: CounterVec<U64>,
	tx_confirmed: CounterVec<U64>,
	tx_confirmed_bytes: CounterVec<U64>,
	blocks_observed: CounterVec<U64>,
	block_txs: HistogramVec,
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

fn counter_vec(
	registry: &Registry,
	name: &str,
	help: &str,
	labels: &[&str],
) -> Result<CounterVec<U64>> {
	register(
		CounterVec::new(Opts::new(name, help), labels)
			.with_context(|| format!("failed to build {name}"))?,
		registry,
	)
	.with_context(|| format!("failed to register {name}"))
}

fn gauge_vec_f64(
	registry: &Registry,
	name: &str,
	help: &str,
	labels: &[&str],
) -> Result<GaugeVec<F64>> {
	register(
		GaugeVec::new(Opts::new(name, help), labels)
			.with_context(|| format!("failed to build {name}"))?,
		registry,
	)
	.with_context(|| format!("failed to register {name}"))
}

fn histogram_vec(
	registry: &Registry,
	name: &str,
	help: &str,
	labels: &[&str],
	buckets: &[f64],
) -> Result<HistogramVec> {
	register(
		HistogramVec::new(HistogramOpts::new(name, help).buckets(buckets.to_vec()), labels)
			.with_context(|| format!("failed to build {name}"))?,
		registry,
	)
	.with_context(|| format!("failed to register {name}"))
}

impl PrometheusMetrics {
	/// Construct a new recorder, registering every metric family on a fresh [`Registry`].
	pub fn new() -> Result<Self> {
		let registry = Registry::new();

		let tx_submitted = counter_vec(
			&registry,
			"bulletin_stress_tx_submitted_total",
			"Store extrinsics accepted by the node RPC, incremented at submission time.",
			&["variant"],
		)?;
		let tx_submitted_bytes = counter_vec(
			&registry,
			"bulletin_stress_tx_submitted_bytes_total",
			"Encoded extrinsic bytes accepted by the node RPC (offered load).",
			&["variant"],
		)?;
		let tx_errors = counter_vec(
			&registry,
			"bulletin_stress_tx_errors_total",
			"Submission error/retry events by class; retriable classes (pool_full, banned, ...) \
			 are counted once per occurrence.",
			&["variant", "class"],
		)?;
		let tx_confirmed = counter_vec(
			&registry,
			"bulletin_stress_tx_confirmed_total",
			"Store transactions confirmed in finalized blocks observed by the block monitor.",
			&["variant"],
		)?;
		let tx_confirmed_bytes = counter_vec(
			&registry,
			"bulletin_stress_tx_confirmed_bytes_total",
			"Uncompressed payload bytes confirmed in finalized blocks.",
			&["variant"],
		)?;
		let blocks_observed = counter_vec(
			&registry,
			"bulletin_stress_blocks_observed_total",
			"Finalized blocks observed by the block monitor during a variant (including empty \
			 ones).",
			&["variant"],
		)?;
		let block_txs = histogram_vec(
			&registry,
			"bulletin_stress_block_txs",
			"Store transactions per observed finalized block (block fullness distribution).",
			&["variant"],
			BLOCK_TXS_BUCKETS,
		)?;

		let latency = histogram_vec(
			&registry,
			"bulletin_stress_latency_seconds",
			"End-to-end latency by kind (inclusion, finalization, retrieval).",
			&["variant", "kind"],
			LATENCY_BUCKETS_SECS,
		)?;

		let reads = counter_vec(
			&registry,
			"bulletin_stress_reads_total",
			"Bitswap read attempts, partitioned by outcome.",
			&["variant", "outcome"],
		)?;
		let read_bytes = counter_vec(
			&registry,
			"bulletin_stress_read_bytes_total",
			"Bytes downloaded via bitswap reads.",
			&["variant"],
		)?;

		let runs = counter_vec(
			&registry,
			"bulletin_stress_runs_total",
			"Completed stress runs, partitioned by outcome (one run = one command invocation or \
			 one --loop-interval-secs iteration).",
			&["outcome"],
		)?;
		let run_in_progress = register(
			Gauge::<U64>::new(
				"bulletin_stress_run_in_progress",
				"1 while a stress run is executing, 0 between --loop-interval-secs iterations.",
			)
			.context("failed to build run_in_progress")?,
			&registry,
		)
		.context("failed to register run_in_progress")?;
		let run_start_timestamp = register(
			Gauge::<F64>::new(
				"bulletin_stress_run_start_timestamp_seconds",
				"Unix timestamp of the most recent run start.",
			)
			.context("failed to build run_start_timestamp")?,
			&registry,
		)
		.context("failed to register run_start_timestamp")?;
		let variant_active = register(
			GaugeVec::<U64>::new(
				Opts::new(
					"bulletin_stress_variant_active",
					"1 while the labeled variant is running (timeline segmentation for \
					 dashboards).",
				),
				&["variant"],
			)
			.context("failed to build variant_active")?,
			&registry,
		)
		.context("failed to register variant_active")?;

		let result_throughput_tps = gauge_vec_f64(
			&registry,
			"bulletin_stress_result_throughput_tx_per_sec",
			"Measured throughput of the variant's last completed run (tx/s).",
			&["variant"],
		)?;
		let result_throughput_bps = gauge_vec_f64(
			&registry,
			"bulletin_stress_result_throughput_bytes_per_sec",
			"Measured throughput of the variant's last completed run (payload bytes/s).",
			&["variant"],
		)?;
		let result_avg_tx_per_block = gauge_vec_f64(
			&registry,
			"bulletin_stress_result_avg_tx_per_block",
			"Average store transactions per measured block in the variant's last completed run.",
			&["variant"],
		)?;
		let result_peak_tx_per_block = register(
			GaugeVec::<U64>::new(
				Opts::new(
					"bulletin_stress_result_peak_tx_per_block",
					"Peak store transactions in a single block in the variant's last completed \
					 run.",
				),
				&["variant"],
			)
			.context("failed to build result_peak_tx_per_block")?,
			&registry,
		)
		.context("failed to register result_peak_tx_per_block")?;
		let result_latency = gauge_vec_f64(
			&registry,
			"bulletin_stress_result_latency_seconds",
			"Latency summary of the variant's last completed run, by kind and quantile.",
			&["variant", "kind", "quantile"],
		)?;
		let result_reads_per_sec = gauge_vec_f64(
			&registry,
			"bulletin_stress_result_reads_per_sec",
			"Read throughput of the variant's last completed run (successful reads/s).",
			&["variant"],
		)?;
		let result_read_bytes_per_sec = gauge_vec_f64(
			&registry,
			"bulletin_stress_result_read_bytes_per_sec",
			"Read bandwidth of the variant's last completed run (bytes/s).",
			&["variant"],
		)?;

		Ok(Self {
			registry,
			tx_submitted,
			tx_submitted_bytes,
			tx_errors,
			tx_confirmed,
			tx_confirmed_bytes,
			blocks_observed,
			block_txs,
			latency,
			reads,
			read_bytes,
			runs,
			run_in_progress,
			run_start_timestamp,
			variant_active,
			result_throughput_tps,
			result_throughput_bps,
			result_avg_tx_per_block,
			result_peak_tx_per_block,
			result_latency,
			result_reads_per_sec,
			result_read_bytes_per_sec,
		})
	}

	/// Convenience constructor for tests.
	#[cfg(test)]
	pub fn for_tests() -> Self {
		Self::new().expect("metric registration on a fresh registry never fails")
	}

	/// Return the registry — used to spawn the exposition HTTP server ([`serve`]).
	pub fn registry(&self) -> &Registry {
		&self.registry
	}

	// ---- run lifecycle ---------------------------------------------------

	/// Mark a run (one command invocation / loop iteration) as started.
	pub fn run_started(&self) {
		self.run_in_progress.set(1);
		let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
		self.run_start_timestamp.set(now);
	}

	/// Mark the current run as finished and count its outcome.
	pub fn run_finished(&self, ok: bool) {
		self.run_in_progress.set(0);
		let outcome = if ok { OUTCOME_OK } else { OUTCOME_FAILED };
		self.runs.with_label_values(&[outcome]).inc();
	}

	/// Flag `variant` as (in)active — drives dashboard timeline panels.
	pub fn set_variant_active(&self, variant: &str, active: bool) {
		self.variant_active.with_label_values(&[variant]).set(u64::from(active));
	}

	// ---- live write path ---------------------------------------------------

	/// Record one accepted store submission of `bytes` encoded extrinsic bytes.
	pub fn inc_submitted(&self, variant: &str, bytes: u64) {
		self.tx_submitted.with_label_values(&[variant]).inc();
		self.tx_submitted_bytes.with_label_values(&[variant]).inc_by(bytes);
	}

	/// Record one submission error/retry event of the given class.
	pub fn inc_error(&self, variant: &str, class: &'static str) {
		self.tx_errors.with_label_values(&[variant, class]).inc();
	}

	/// Record one finalized block observed by the block monitor.
	pub fn observe_confirmed_block(&self, variant: &str, tx_count: u64, payload_bytes: u64) {
		self.tx_confirmed.with_label_values(&[variant]).inc_by(tx_count);
		self.tx_confirmed_bytes.with_label_values(&[variant]).inc_by(payload_bytes);
		self.blocks_observed.with_label_values(&[variant]).inc();
		self.block_txs.with_label_values(&[variant]).observe(tx_count as f64);
	}

	// ---- latency ----------------------------------------------------------

	/// Record a single end-to-end latency observation.
	pub fn observe_latency(&self, variant: &str, kind: LatencyKind, duration: Duration) {
		self.latency
			.with_label_values(&[variant, kind.as_label()])
			.observe(duration.as_secs_f64());
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
		let outcome = if ok { READ_SUCCESS } else { READ_FAILURE };
		self.reads.with_label_values(&[variant, outcome]).inc_by(count);
		if bytes > 0 {
			self.read_bytes.with_label_values(&[variant]).inc_by(bytes);
		}
	}

	// ---- end-of-variant summaries -------------------------------------------

	/// Mirror a completed [`ScenarioResult`] into the `bulletin_stress_result_*` gauges.
	pub fn record_result(&self, result: &ScenarioResult) {
		let variant =
			if result.variant.is_empty() { result.name.as_str() } else { result.variant.as_str() };
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

/// Spawn an HTTP exposition server on `addr` serving `/metrics` from the recorder's registry.
/// The returned future runs until the underlying hyper server exits (typically: never).
pub async fn serve(addr: SocketAddr, registry: Registry) -> Result<()> {
	substrate_prometheus_endpoint::init_prometheus(addr, registry)
		.await
		.map_err(|e| anyhow::anyhow!("prometheus exposition server failed: {e}"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use substrate_prometheus_endpoint::prometheus::{Encoder, TextEncoder};

	fn encode(m: &PrometheusMetrics) -> String {
		let mut buf = Vec::new();
		TextEncoder::new().encode(&m.registry().gather(), &mut buf).expect("encode");
		String::from_utf8(buf).expect("utf8")
	}

	#[test]
	fn registered_families_appear_in_encoded_output() {
		let m = PrometheusMetrics::for_tests();
		// Trigger one observation per family so each has a series.
		m.inc_submitted("1KB", 1234);
		m.inc_error("1KB", "pool_full");
		m.observe_confirmed_block("1KB", 100, 100 * 1024);
		m.observe_latency("hop-full-cycle", LatencyKind::Inclusion, Duration::from_secs(6));
		m.inc_reads("bitswap-bulk-read", true, 3, 3 * 128 * 1024);
		m.run_started();
		m.run_finished(true);
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
			"bulletin_stress_tx_submitted_total",
			"bulletin_stress_tx_submitted_bytes_total",
			"bulletin_stress_tx_errors_total",
			"bulletin_stress_tx_confirmed_total",
			"bulletin_stress_tx_confirmed_bytes_total",
			"bulletin_stress_blocks_observed_total",
			"bulletin_stress_block_txs",
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

	#[test]
	fn confirmed_block_updates_all_block_families() {
		let m = PrometheusMetrics::for_tests();
		m.observe_confirmed_block("1KB", 500, 512_000);
		m.observe_confirmed_block("1KB", 0, 0);

		let txt = encode(&m);
		assert!(txt.contains("bulletin_stress_tx_confirmed_total{variant=\"1KB\"} 500"));
		assert!(txt.contains("bulletin_stress_blocks_observed_total{variant=\"1KB\"} 2"));
		// Both blocks fall in the +Inf bucket; the empty one also in the first (le="1").
		let inf_line = txt
			.lines()
			.find(|l| {
				l.starts_with("bulletin_stress_block_txs_bucket") && l.contains("le=\"+Inf\"")
			})
			.expect("+Inf bucket present");
		assert!(inf_line.ends_with(" 2"), "unexpected +Inf bucket line: {inf_line}");
	}

	#[test]
	fn run_outcomes_are_separate_series() {
		let m = PrometheusMetrics::for_tests();
		m.run_finished(true);
		m.run_finished(true);
		m.run_finished(false);

		let txt = encode(&m);
		assert!(txt.contains("bulletin_stress_runs_total{outcome=\"ok\"} 2"));
		assert!(txt.contains("bulletin_stress_runs_total{outcome=\"failed\"} 1"));
	}

	#[test]
	fn record_result_falls_back_to_name_when_variant_empty() {
		let m = PrometheusMetrics::for_tests();
		m.record_result(&ScenarioResult {
			name: "Renew stress".into(),
			throughput_tps: 10.0,
			..Default::default()
		});
		let txt = encode(&m);
		assert!(txt
			.contains("bulletin_stress_result_throughput_tx_per_sec{variant=\"Renew stress\"} 10"));
	}
}
