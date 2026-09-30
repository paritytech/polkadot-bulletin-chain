// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Readers and assertions for `sc-hop`'s `substrate_hop_*` Prometheus metrics.
//!
//! Metrics are per-node: the maintenance task that promotes a blob runs on the node
//! that received the submit, so every read must target that node. A sibling collator
//! reports all zeros.
//!
//! `NetworkNode::reports` resolves a missing series to `0.0` rather than erroring, so
//! an unregistered registry is indistinguishable from an idle one on any single read.
//! [`assert_hop_metrics_registered`] is the guard against that.

use super::config::*;
use anyhow::{Context, Result};
use zombienet_sdk::NetworkNode;

/// Read one `substrate_hop_*` series as `u64`. Absent series read as `0`.
pub async fn hop_metric(node: &NetworkNode, name: &str) -> Result<u64> {
	let value = node
		.reports(name.to_string())
		.await
		.map_err(|e| anyhow::anyhow!("read {name}: {e}"))?;
	Ok(value as u64)
}

/// Wait until a `substrate_hop_*` series satisfies `predicate`.
pub async fn wait_hop_metric(
	node: &NetworkNode,
	name: &str,
	predicate: impl Fn(u64) -> bool + Copy,
	timeout_secs: u64,
	what: &str,
) -> Result<()> {
	node.wait_metric_with_timeout(name.to_string(), move |v| predicate(v as u64), timeout_secs)
		.await
		.with_context(|| format!("{what}: {name} did not satisfy the predicate"))
}

/// Confirm the HOP metrics registry is actually wired before asserting on any value.
///
/// `HopParams::build_pool` degrades a registration failure to `HopMetrics::disabled()`
/// with only a warning, and absent series read as `0`, so without this every
/// "counter is zero" assertion would pass vacuously. The maintenance counter ticks
/// once per `--hop-check-interval`, making it the cheapest positive signal.
pub async fn assert_hop_metrics_registered(node: &NetworkNode, timeout_secs: u64) -> Result<()> {
	wait_hop_metric(
		node,
		HOP_MAINTENANCE_TICKS_METRIC,
		|ticks| ticks > 0,
		timeout_secs,
		"HOP metrics registry not wired (all substrate_hop_* series absent or disabled)",
	)
	.await
}

/// Counter values for delta assertions across a phase.
///
/// Not an atomic snapshot: `reports` re-scrapes per series, so the fields come from
/// consecutive scrapes. Fine for the monotonic comparisons used here, wrong for any
/// cross-field invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HopCounters {
	pub inserted_bytes: u64,
	pub promotions_confirmed: u64,
	pub removed_acked: u64,
	pub removed_expired_promoted: u64,
	pub removed_expired_unpromoted: u64,
}

impl HopCounters {
	pub async fn read(node: &NetworkNode) -> Result<Self> {
		Ok(Self {
			inserted_bytes: hop_metric(node, HOP_POOL_INSERTED_BYTES_METRIC).await?,
			promotions_confirmed: hop_metric(node, HOP_PROMOTIONS_CONFIRMED_METRIC).await?,
			removed_acked: hop_metric(node, HOP_REMOVED_ACKED_METRIC).await?,
			removed_expired_promoted: hop_metric(node, HOP_REMOVED_EXPIRED_PROMOTED_METRIC).await?,
			removed_expired_unpromoted: hop_metric(node, HOP_REMOVED_EXPIRED_UNPROMOTED_METRIC)
				.await?,
		})
	}
}
