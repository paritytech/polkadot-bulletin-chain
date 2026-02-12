// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Metric/log waiting, sync verification, and session change detection.

use super::config::*;
use anyhow::{anyhow, Context, Result};
use std::time::Duration;
use subxt::{config::substrate::SubstrateConfig, OnlineClient};
use zombienet_orchestrator::network::node::LogLineCountOptions;

pub fn log_line_at_least_once(timeout_secs: u64) -> LogLineCountOptions {
	LogLineCountOptions::new(|count| count >= 1, Duration::from_secs(timeout_secs), false)
}

pub fn log_line_absent(timeout_secs: u64) -> LogLineCountOptions {
	LogLineCountOptions::no_occurences_within_timeout(Duration::from_secs(timeout_secs))
}

pub async fn expect_log_line(
	node: &zombienet_sdk::NetworkNode,
	pattern: &str,
	timeout_secs: u64,
	error_msg: &str,
) -> Result<()> {
	let result = node
		.wait_log_line_count_with_timeout(pattern, false, log_line_at_least_once(timeout_secs))
		.await
		.context(format!("Failed to check log: {}", pattern))?;
	if !result.success() {
		anyhow::bail!("{}", error_msg);
	}
	Ok(())
}

pub async fn expect_no_log_line(
	node: &zombienet_sdk::NetworkNode,
	pattern: &str,
	timeout_secs: u64,
	error_msg: &str,
) -> Result<()> {
	let result = node
		.wait_log_line_count_with_timeout(pattern, false, log_line_absent(timeout_secs))
		.await
		.context(format!("Failed to check absence of log: {}", pattern))?;
	if !result.success() {
		anyhow::bail!("{}", error_msg);
	}
	Ok(())
}

/// Verify state sync was attempted (--sync=fast). On fast-producing chains, state sync
/// may fall back to block sync because the target keeps moving - we only verify it started.
pub async fn verify_state_sync_completed(node: &zombienet_sdk::NetworkNode) -> Result<()> {
	log::info!("Verifying state sync was attempted");
	expect_log_line(
		node,
		"Starting state sync",
		LOG_TIMEOUT_SECS,
		"Node did not start state sync - fast sync may not be active",
	)
	.await?;
	expect_no_log_line(
		node,
		"verification failed",
		LOG_ERROR_TIMEOUT_SECS,
		"Node logged verification errors",
	)
	.await?;
	log::info!("✓ State sync was attempted");
	Ok(())
}

/// Verify warp sync completed via log check, then wait for idle state via metric.
pub async fn verify_warp_sync_completed(node: &zombienet_sdk::NetworkNode) -> Result<()> {
	log::info!("Verifying warp sync completed");
	expect_log_line(
		node,
		"Warp sync is complete",
		LOG_TIMEOUT_SECS,
		"Node did not complete warp sync",
	)
	.await?;
	wait_for_node_idle(node, SYNC_TIMEOUT_SECS)
		.await
		.context("Node did not reach idle state after warp sync")?;
	expect_no_log_line(
		node,
		"verification failed",
		LOG_ERROR_TIMEOUT_SECS,
		"Node logged verification errors",
	)
	.await?;
	log::info!("✓ Warp sync completed and node is idle");
	Ok(())
}

pub async fn wait_for_validator(node: &zombienet_sdk::NetworkNode) -> Result<()> {
	node.wait_metric_with_timeout(
		NODE_ROLE_METRIC,
		|role| role == VALIDATOR_ROLE_VALUE,
		METRIC_TIMEOUT_SECS,
	)
	.await
	.context("Node did not become validator")
}

pub async fn wait_for_fullnode(node: &zombienet_sdk::NetworkNode) -> Result<()> {
	node.wait_metric_with_timeout(
		NODE_ROLE_METRIC,
		|role| role == FULLNODE_ROLE_VALUE,
		METRIC_TIMEOUT_SECS,
	)
	.await
	.context("Node did not become full node")
}

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

pub async fn wait_for_finalized_height(
	node: &zombienet_sdk::NetworkNode,
	min_height: u64,
	timeout_secs: u64,
) -> Result<()> {
	node.wait_metric_with_timeout(
		FINALIZED_BLOCK_METRIC,
		|height| height >= min_height as f64,
		timeout_secs,
	)
	.await
	.context(format!("Node did not finalize block height {}", min_height))
}

pub async fn wait_for_node_idle(
	node: &zombienet_sdk::NetworkNode,
	timeout_secs: u64,
) -> Result<()> {
	node.wait_metric_with_timeout(
		IS_MAJOR_SYNCING_METRIC,
		|value| value == IDLE_VALUE,
		timeout_secs,
	)
	.await
	.context("Node did not reach idle state (still syncing)")
}

/// Wait for a parachain node's embedded relay chain to sync. Required for warp sync because
/// parachain warp sync queries the embedded relay chain for the finalized parachain head -
/// if queried too early, it returns genesis (#0) and warp sync gets stuck.
pub async fn wait_for_relay_chain_to_sync(
	node: &zombienet_sdk::NetworkNode,
	timeout_secs: u64,
) -> Result<()> {
	let result = node
		.wait_log_line_count_with_timeout(
			r"Update at relay chain block.*included: #[1-9]",
			false,
			log_line_at_least_once(timeout_secs),
		)
		.await
		.context("Failed to check relay chain sync status")?;

	if !result.success() {
		anyhow::bail!(
			"Embedded relay chain did not sync - no 'included' parachain blocks seen within {}s",
			timeout_secs
		);
	}

	log::info!("✓ Embedded relay chain is synced (seeing included parachain blocks)");
	Ok(())
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
/// aren't assigned to validate a parachain until session boundaries - collators can only
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
