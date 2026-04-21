//! Test plans — hardcoded and YAML-defined.
//!
//! A plan is a sequence of entries. Each entry is either a single step or a
//! parallel group of steps (different scenario types running concurrently).
//!
//! # YAML format
//!
//! ```yaml
//! steps:
//!   # Sequential step
//!   - scenario: throughput
//!     variants: "1KB"
//!     target_blocks: 100
//!
//!   # Parallel group — different scenarios run concurrently
//!   - parallel:
//!     - scenario: throughput
//!       variants: "MIXED"
//!       target_blocks: 100
//!     - scenario: bitswap
//!       iterations: 512
//!
//!   # Another sequential step
//!   - scenario: throughput
//!     variants: "2MB"
//!     target_blocks: 50
//! ```
//!
//! All fields except `scenario` are optional and fall back to CLI defaults.

use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

/// A test plan: a sequence of entries to execute.
#[derive(Debug, Deserialize)]
pub struct TestPlan {
	pub steps: Vec<PlanEntry>,
}

/// An entry in a test plan: either a single step or a parallel group.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum PlanEntry {
	/// Multiple steps running concurrently (must be different scenario types).
	Parallel { parallel: Vec<PlanStep> },
	/// A single step running sequentially.
	Single(PlanStep),
}

impl PlanEntry {
	/// Human-readable description for logging.
	pub fn description(&self) -> String {
		match self {
			PlanEntry::Single(step) => {
				format!("{} ({})", step.scenario, step.variants.as_deref().unwrap_or("all"))
			},
			PlanEntry::Parallel { parallel } => {
				let parts: Vec<String> = parallel
					.iter()
					.map(|s| {
						format!("{} ({})", s.scenario, s.variants.as_deref().unwrap_or("all"))
					})
					.collect();
				format!("parallel [{}]", parts.join(" + "))
			},
		}
	}
}

/// A single step in a test plan.
#[derive(Debug, Deserialize)]
pub struct PlanStep {
	/// Scenario to run: "throughput" or "bitswap".
	pub scenario: Scenario,

	/// Throughput: comma-separated variant labels (e.g. "1KB", "MIXED", "1KB,2MB").
	pub variants: Option<String>,

	/// Number of measured blocks per variant (overrides --target-blocks).
	pub target_blocks: Option<u32>,

	/// Number of RPC submission workers (overrides --submitters).
	pub submitters: Option<usize>,

	/// Blocks worth of txs per pipeline iteration (overrides --iteration-blocks).
	pub iteration_blocks: Option<u32>,

	/// RNG seed for MIXED mode (overrides --mix-seed).
	pub mix_seed: Option<u64>,

	/// Bitswap: number of unique items to store and read.
	pub iterations: Option<u32>,

	/// Bitswap: payload size in bytes for each stored item.
	pub payload_size: Option<usize>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Scenario {
	Throughput,
	Bitswap,
}

impl std::fmt::Display for Scenario {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Scenario::Throughput => write!(f, "throughput"),
			Scenario::Bitswap => write!(f, "bitswap"),
		}
	}
}

/// Load a test plan from a YAML file.
pub fn load_from_file(path: &Path) -> Result<TestPlan> {
	let content = std::fs::read_to_string(path)
		.map_err(|e| anyhow::anyhow!("Failed to read plan file {}: {e}", path.display()))?;
	let plan: TestPlan = serde_yaml::from_str(&content)
		.map_err(|e| anyhow::anyhow!("Failed to parse plan file {}: {e}", path.display()))?;
	if plan.steps.is_empty() {
		anyhow::bail!("Plan file {} has no steps", path.display());
	}
	Ok(plan)
}

// --- Built-in plans ---

/// Quick performance test: 1KB, MIXED, and 2MB payloads, ~100 minutes each.
/// 100 min / 6s block time ≈ 1000 blocks per step. Total ~5 hours.
pub fn quick_performance_test() -> TestPlan {
	TestPlan {
		steps: vec![
			PlanEntry::Single(PlanStep {
				scenario: Scenario::Throughput,
				variants: Some("1KB".into()),
				target_blocks: Some(1000),
				..default_step()
			}),
			PlanEntry::Single(PlanStep {
				scenario: Scenario::Throughput,
				variants: Some("MIXED".into()),
				target_blocks: Some(1000),
				..default_step()
			}),
			PlanEntry::Single(PlanStep {
				scenario: Scenario::Throughput,
				variants: Some("2MB".into()),
				target_blocks: Some(1000),
				..default_step()
			}),
		],
	}
}

fn default_step() -> PlanStep {
	PlanStep {
		scenario: Scenario::Throughput,
		variants: None,
		target_blocks: None,
		submitters: None,
		iteration_blocks: None,
		mix_seed: None,
		iterations: None,
		payload_size: None,
	}
}
