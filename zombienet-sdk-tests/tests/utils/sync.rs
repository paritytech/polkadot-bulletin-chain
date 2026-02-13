// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Metric polling, session change detection, and chain stall/recovery helpers.

use super::config::*;
use anyhow::{anyhow, Context, Result};
use std::time::Duration;
use subxt::{config::substrate::SubstrateConfig, OnlineClient};

pub async fn wait_for_block_height(
	node: &zombienet_sdk::NetworkNode,
	min_height: u64,
	timeout_secs: u64,
) -> Result<()> {
	node.wait_metric_with_timeout(
		BEST_BLOCK_METRIC,
		|height| height >= min_height as f64,
		timeout_secs,
	)
	.await
	.context(format!("Node did not reach block height {}", min_height))
}

const WAIT_MAX_BLOCKS_FOR_SESSION: u32 = 50;

async fn is_session_change_block(
	block: &subxt::blocks::Block<SubstrateConfig, OnlineClient<SubstrateConfig>>,
) -> Result<bool> {
	let events = block.events().await.context("Failed to fetch block events")?;
	Ok(events.iter().any(|event| {
		event
			.as_ref()
			.is_ok_and(|e| e.pallet_name() == "Session" && e.variant_name() == "NewSession")
	}))
}

/// Wait for session change on relay chain. Required for parachain tests because validators
/// aren't assigned to validate a parachain until session boundaries — collators can only
/// produce backed blocks after validators are assigned.
pub async fn wait_for_first_session_change(
	relay_client: &OnlineClient<SubstrateConfig>,
	timeout_secs: u64,
) -> Result<()> {
	wait_for_nth_session_change(relay_client, 1, timeout_secs).await
}

pub async fn wait_for_nth_session_change(
	relay_client: &OnlineClient<SubstrateConfig>,
	mut sessions_to_wait: u32,
	timeout_secs: u64,
) -> Result<()> {
	log::info!("Waiting for {} session change(s) on relay chain...", sessions_to_wait);

	let wait_future = async {
		let mut blocks_sub = relay_client
			.blocks()
			.subscribe_finalized()
			.await
			.context("Failed to subscribe to finalized blocks")?;

		let mut waited_block_count = 0u32;

		while let Some(block_result) = blocks_sub.next().await {
			let block = block_result.context("Error receiving block")?;
			log::debug!("Relay chain finalized block #{}", block.number());

			if is_session_change_block(&block).await? {
				sessions_to_wait -= 1;
				log::info!(
					"Session change detected at relay block #{}. {} more to wait.",
					block.number(),
					sessions_to_wait
				);

				if sessions_to_wait == 0 {
					log::info!("All required session changes detected");
					return Ok(());
				}

				waited_block_count = 0;
			} else {
				waited_block_count += 1;
				if waited_block_count >= WAIT_MAX_BLOCKS_FOR_SESSION {
					anyhow::bail!(
						"Waited {} blocks without session change. Session should have arrived by now.",
						WAIT_MAX_BLOCKS_FOR_SESSION
					);
				}
			}
		}

		anyhow::bail!("Block subscription ended unexpectedly")
	};

	tokio::time::timeout(Duration::from_secs(timeout_secs), wait_future)
		.await
		.map_err(|_| anyhow!("Timeout waiting for session change after {}s", timeout_secs))?
}

pub async fn wait_for_session_change_on_node(
	relay_node: &zombienet_sdk::NetworkNode,
	timeout_secs: u64,
) -> Result<()> {
	let relay_client: OnlineClient<SubstrateConfig> =
		relay_node.wait_client().await.context("Failed to get relay chain client")?;
	wait_for_first_session_change(&relay_client, timeout_secs).await
}

/// Detect chain stall by polling best block metric.
/// Returns the block height at which the chain stalled.
pub async fn wait_for_chain_stall(
	node: &zombienet_sdk::NetworkNode,
	poll_interval_secs: u64,
	stall_threshold_secs: u64,
	timeout_secs: u64,
) -> Result<u64> {
	log::info!(
		"Watching for chain stall (threshold={}s, timeout={}s)...",
		stall_threshold_secs,
		timeout_secs
	);

	let start = tokio::time::Instant::now();
	let timeout = Duration::from_secs(timeout_secs);
	let interval = Duration::from_secs(poll_interval_secs);
	let threshold = Duration::from_secs(stall_threshold_secs);

	let mut last_height: Option<f64> = None;
	let mut stall_start: Option<tokio::time::Instant> = None;

	loop {
		if start.elapsed() > timeout {
			anyhow::bail!(
				"Timeout after {}s waiting for chain stall (last height: {:?})",
				timeout_secs,
				last_height
			);
		}

		// Try to read the current best block metric
		let current_height = node.reports(BEST_BLOCK_METRIC).await.ok().and_then(|v| {
			if v > 0.0 {
				Some(v)
			} else {
				None
			}
		});

		match (current_height, last_height) {
			(Some(current), Some(last)) if (current - last).abs() < f64::EPSILON => {
				// Block height unchanged
				let stall_time = stall_start.get_or_insert(tokio::time::Instant::now());
				if stall_time.elapsed() >= threshold {
					let height = current as u64;
					log::info!(
						"Chain stall detected at block {} (no progress for {}s)",
						height,
						stall_threshold_secs
					);
					return Ok(height);
				}
			},
			(Some(current), _) => {
				// Block height changed — reset stall detection
				log::debug!("Block height: {}", current);
				last_height = Some(current);
				stall_start = None;
			},
			(None, _) => {
				log::debug!("Could not read best block metric, retrying...");
			},
		}

		tokio::time::sleep(interval).await;
	}
}

/// Wait for chain to resume block production after a stall.
pub async fn wait_for_chain_recovery(
	node: &zombienet_sdk::NetworkNode,
	stalled_height: u64,
	timeout_secs: u64,
) -> Result<()> {
	log::info!(
		"Waiting for chain recovery (stalled at block {}, timeout={}s)...",
		stalled_height,
		timeout_secs
	);

	let target = stalled_height + 3;
	wait_for_block_height(node, target, timeout_secs).await?;

	log::info!("Chain recovered — blocks being produced past height {}", target);
	Ok(())
}
