use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::chain_info::{ChainLimits, EnvironmentInfo};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockStats {
	pub number: u64,
	pub tx_count: u64,
	/// Sum of uncompressed payload sizes (not actual block/PoV size).
	pub payload_bytes: u64,
	/// Whether this block was recorded during the pre-fill phase (before pool
	/// saturation). `false` = measured (steady-state), `true` = pre-fill.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub prefill: bool,
	/// On-chain wall clock timestamp (ms since Unix epoch) from `pallet_timestamp::Now`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub timestamp_ms: Option<u64>,
	/// Block hash (hex-encoded). Used to match best blocks to finalized blocks.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub hash: Option<String>,
	/// Whether this block has been confirmed finalized.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub finalized: bool,
	/// Interval in milliseconds since the previous block's on-chain timestamp.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub interval_ms: Option<u64>,
}

/// Detailed submission statistics for later analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubmissionStats {
	/// Accounts that had stale nonces (already used in a previous run).
	pub stale_nonces: u64,
	/// Times a tx was re-queued due to pool full / temporarily banned.
	pub pool_full_retries: u64,
	/// Non-retriable errors (future nonce, other).
	pub errors: u64,
	/// Items remaining in queue when test stopped (not submitted).
	pub remaining_in_queue: u64,
	/// Number of accounts whose nonce was successfully pre-initialized.
	pub nonces_initialized: u64,
	/// Number of accounts whose nonce pre-init failed.
	pub nonces_failed: u64,
}

/// Theoretical block capacity limits derived from runtime constants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoreticalLimits {
	/// Max tx/block from block weight budget.
	pub weight_cap: u64,
	/// Max tx/block from block length budget.
	pub length_cap: u64,
	/// Hard extrinsic count limit (MaxBlockTransactions = 512).
	pub count_cap: u64,
	/// Effective limit: min(weight, length, count).
	pub effective_cap: u64,
	/// Which limit is the bottleneck.
	pub bottleneck: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
	pub p50: Duration,
	pub p95: Duration,
	pub p99: Duration,
	pub min: Duration,
	pub max: Duration,
	pub mean: Duration,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScenarioResult {
	pub name: String,
	pub duration: Duration,
	#[serde(default, skip_serializing_if = "is_zero_u64")]
	pub total_submitted: u64,
	#[serde(default, skip_serializing_if = "is_zero_u64")]
	pub total_confirmed: u64,
	#[serde(default, skip_serializing_if = "is_zero_u64")]
	pub total_errors: u64,
	pub payload_size: usize,
	pub throughput_tps: f64,
	pub throughput_bytes_per_sec: f64,
	#[serde(default, skip_serializing_if = "is_zero_f64")]
	pub avg_tx_per_block: f64,
	#[serde(default, skip_serializing_if = "is_zero_u64")]
	pub peak_tx_per_block: u64,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub inclusion_latency: Option<LatencyStats>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub finalization_latency: Option<LatencyStats>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub retrieval_latency: Option<LatencyStats>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub theoretical: Option<TheoreticalLimits>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub chain_limits: Option<ChainLimits>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub environment: Option<EnvironmentInfo>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub blocks: Vec<BlockStats>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub submission_stats: Option<SubmissionStats>,

	// --- Block timing fields ---
	/// Average block interval in milliseconds (from on-chain timestamps of steady blocks).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub avg_block_interval_ms: Option<f64>,
	/// Number of fork replacements detected during the test.
	#[serde(default, skip_serializing_if = "is_zero_u64")]
	pub fork_detections: u64,
	/// Whether throughput was computed from on-chain timestamps (true) or Instant (false).
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub onchain_timing: bool,

	// --- Read-domain fields ---
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub total_reads: Option<u64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub successful_reads: Option<u64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub failed_reads: Option<u64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub reads_per_sec: Option<f64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub read_bytes_per_sec: Option<f64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub data_verified: Option<bool>,
}

fn is_zero_u64(v: &u64) -> bool {
	*v == 0
}

fn is_zero_f64(v: &f64) -> bool {
	*v == 0.0
}

impl ScenarioResult {
	pub fn print_text(&self) {
		println!();
		println!("{}", "=".repeat(72));
		println!(" {}", self.name);
		println!("{}", "=".repeat(72));
		println!(" Payload size        | {} bytes", format_bytes(self.payload_size));
		println!(" Duration            | {:.1}s", self.duration.as_secs_f64());

		// Write stats
		if self.total_submitted > 0 {
			println!(
				" Submitted / OK / Err| {} / {} / {}",
				self.total_submitted, self.total_confirmed, self.total_errors
			);
		}
		if self.throughput_tps > 0.0 {
			println!(
				" Throughput          | {:.1} tx/s  |  {}/s",
				self.throughput_tps,
				format_bytes(self.throughput_bytes_per_sec as usize)
			);
		}
		if self.avg_tx_per_block > 0.0 {
			println!(" Avg tx/block        | {:.1}", self.avg_tx_per_block);
			println!(" Peak tx/block       | {}", self.peak_tx_per_block);
		}
		if let Some(avg_interval) = self.avg_block_interval_ms {
			let timing_source = if self.onchain_timing { "on-chain" } else { "client" };
			print!(" Block timing        | avg interval {avg_interval:.0}ms ({timing_source})");
			if self.fork_detections > 0 {
				print!(" | {} forks detected", self.fork_detections);
			}
			println!();
		}

		// Read stats
		if let Some(total) = self.total_reads {
			println!("{}", "-".repeat(72));
			println!(" READ STATS");
			println!(
				" Reads OK / Fail     | {} / {}",
				self.successful_reads.unwrap_or(0),
				self.failed_reads.unwrap_or(0)
			);
			println!(" Total reads         | {total}");
			if let Some(rps) = self.reads_per_sec {
				println!(" Read throughput     | {rps:.1} reads/s");
			}
			if let Some(bps) = self.read_bytes_per_sec {
				println!(" Read bandwidth      | {}/s", format_bytes(bps as usize));
			}
			if let Some(verified) = self.data_verified {
				println!(" Data verified       | {verified}");
			}
		}

		if let Some(ref lat) = self.inclusion_latency {
			println!("{}", "-".repeat(72));
			println!(" LATENCY              p50       p95       p99       min       max");
			print_latency_row(" To inclusion  ", lat);
		}
		if let Some(ref lat) = self.finalization_latency {
			print_latency_row(" To finalization", lat);
		}
		if let Some(ref lat) = self.retrieval_latency {
			print_latency_row(" To retrieval  ", lat);
		}

		if let Some(ref th) = self.theoretical {
			println!("{}", "-".repeat(72));
			println!(" CAPACITY              {:>10}   {:>10}", "Theoretical", "Measured");
			println!(
				"  Weight cap          | {:>6} tx/blk   {:>6}",
				th.weight_cap,
				if self.avg_tx_per_block > 0.0 {
					format!("{:.1}", self.avg_tx_per_block)
				} else {
					"-".to_string()
				}
			);
			println!(
				"  Length cap          | {:>6} tx/blk   {:>6}",
				th.length_cap,
				if self.avg_tx_per_block > 0.0 {
					format!("{:.1}", self.avg_tx_per_block)
				} else {
					"-".to_string()
				}
			);
			println!(
				"  Count cap          | {:>6} tx/blk   {:>6}",
				th.count_cap,
				if self.avg_tx_per_block > 0.0 {
					format!("{:.1}", self.avg_tx_per_block)
				} else {
					"-".to_string()
				}
			);
			println!(
				"  Effective          | {:>6} tx/blk   {:>6}   (bottleneck: {})",
				th.effective_cap,
				if self.avg_tx_per_block > 0.0 {
					format!("{:.1}", self.avg_tx_per_block)
				} else {
					"-".to_string()
				},
				th.bottleneck
			);
			if self.avg_tx_per_block > 0.0 {
				let utilization = self.avg_tx_per_block / th.effective_cap as f64 * 100.0;
				println!("  Utilization        |              {utilization:>5.1}%");
				println!(
					"  Peak               |              {:>6} tx/blk",
					self.peak_tx_per_block
				);
			}
		}

		if !self.blocks.is_empty() {
			let has_timestamps = self.blocks.iter().any(|b| b.timestamp_ms.is_some());
			println!("{}", "-".repeat(if has_timestamps { 100 } else { 72 }));
			println!(" BLOCKS");
			if has_timestamps {
				println!(
					" {:<11} | {:>3} | {:>22} | {:>10} | {:>3} | Phase",
					"Block", "TXs", "Payload (uncompressed)", "Interval", "Fin"
				);
				for b in &self.blocks {
					let phase = if b.prefill { "pre-fill" } else { "measured" };
					let interval = b
						.interval_ms
						.map(|ms| format!("{ms:>7}ms"))
						.unwrap_or_else(|| "        -".to_string());
					let fin = if b.finalized { " Y" } else { " N" };
					println!(
						" #{:<10} | {:>3} | {:>22} | {} | {:>3} | {}",
						b.number,
						b.tx_count,
						format_bytes(b.payload_bytes as usize),
						interval,
						fin,
						phase
					);
				}
			} else {
				println!(
					" {:<11} | {:>3} | {:>22} | Phase",
					"Block", "TXs", "Payload (uncompressed)"
				);
				for b in &self.blocks {
					let phase = if b.prefill { "pre-fill" } else { "measured" };
					println!(
						" #{:<10} | {:>3} | {:>22} | {}",
						b.number,
						b.tx_count,
						format_bytes(b.payload_bytes as usize),
						phase
					);
				}
			}
		}

		println!("{}", "=".repeat(72));
		println!();
	}

	pub fn print_json(&self) {
		println!("{}", serde_json::to_string_pretty(self).unwrap_or_default());
	}
}

fn print_latency_row(label: &str, lat: &LatencyStats) {
	println!(
		"{}  {:>8.2}s {:>8.2}s {:>8.2}s {:>8.2}s {:>8.2}s",
		label,
		lat.p50.as_secs_f64(),
		lat.p95.as_secs_f64(),
		lat.p99.as_secs_f64(),
		lat.min.as_secs_f64(),
		lat.max.as_secs_f64(),
	);
}

pub fn compute_latency_stats(durations: &mut [Duration]) -> Option<LatencyStats> {
	if durations.is_empty() {
		return None;
	}
	durations.sort();
	let len = durations.len();
	let sum: Duration = durations.iter().sum();
	Some(LatencyStats {
		p50: durations[len * 50 / 100],
		p95: durations[len * 95 / 100],
		p99: durations[(len * 99 / 100).min(len - 1)],
		min: durations[0],
		max: durations[len - 1],
		mean: sum / len as u32,
	})
}

fn format_bytes(bytes: usize) -> String {
	if bytes >= 1024 * 1024 {
		format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
	} else if bytes >= 1024 {
		format!("{:.2} KB", bytes as f64 / 1024.0)
	} else {
		format!("{bytes} B")
	}
}

/// Print sustained load analysis: compare first-quarter vs last-quarter throughput.
pub fn print_sustained_analysis(result: &ScenarioResult) {
	let blocks = &result.blocks;
	if blocks.len() < 4 {
		println!();
		println!("{}", "-".repeat(72));
		println!(" SUSTAINED ANALYSIS: Not enough blocks ({}) for analysis", blocks.len());
		println!("{}", "-".repeat(72));
		return;
	}

	let quarter = blocks.len() / 4;
	let first_quarter = &blocks[..quarter];
	let last_quarter = &blocks[blocks.len() - quarter..];

	let first_avg =
		first_quarter.iter().map(|b| b.tx_count).sum::<u64>() as f64 / first_quarter.len() as f64;
	let last_avg =
		last_quarter.iter().map(|b| b.tx_count).sum::<u64>() as f64 / last_quarter.len() as f64;

	let degradation =
		if first_avg > 0.0 { (last_avg - first_avg) / first_avg * 100.0 } else { 0.0 };

	println!();
	println!("{}", "-".repeat(72));
	println!(" SUSTAINED ANALYSIS ({} blocks total)", blocks.len());
	println!("{}", "-".repeat(72));
	println!(" First 25% avg tx/block  | {:.1} ({} blocks)", first_avg, first_quarter.len());
	println!(" Last 25% avg tx/block   | {:.1} ({} blocks)", last_avg, last_quarter.len());
	println!(
		" Degradation             | {}{:.1}%",
		if degradation >= 0.0 { "+" } else { "" },
		degradation
	);
	println!("{}", "-".repeat(72));
}

/// Print a final summary table comparing multiple scenario results.
pub fn print_summary_table(results: &[ScenarioResult]) {
	println!();
	println!("{}", "=".repeat(90));
	println!(" FINAL SUMMARY");
	println!("{}", "=".repeat(90));
	println!(
		" {:<35} | {:>8} | {:>10} | {:>10} | {:>8}",
		"Scenario", "Payload", "Throughput", "Confirmed", "Errors"
	);
	println!("{}", "-".repeat(90));
	for r in results {
		println!(
			" {:<35} | {:>8} | {:>8.1} tx/s | {:>10} | {:>8}",
			r.name,
			format_bytes(r.payload_size),
			r.throughput_tps,
			r.total_confirmed,
			r.total_errors,
		);
	}
	println!("{}", "=".repeat(90));
	println!();
}
