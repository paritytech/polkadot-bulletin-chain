// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Block-fetch and proof-event helpers shared across test files. Centralizes the
//! best-vs-finalized fetch semantics and the `ProofChecked` assertion so each
//! test file doesn't ship its own copy.

use super::events::count_event;
use anyhow::{anyhow, Result};
use subxt::{blocks::Block, config::substrate::SubstrateConfig, utils::H256, OnlineClient};

const FINALIZED_POLL_INTERVAL_SECS: u64 = 2;
const FINALIZED_POLL_TIMEOUT_SECS: u64 = 120;

/// Fetch the latest best block. `at_latest()` returns the latest finalized block via
/// chainHead_v2 — on cumulus, finality lags ~10s behind production, so it can be stuck at
/// block 0 well after the chain is producing.
pub async fn current_best_block(
	client: &OnlineClient<SubstrateConfig>,
) -> Result<Block<SubstrateConfig, OnlineClient<SubstrateConfig>>> {
	let mut sub = client.blocks().subscribe_best().await?;
	sub.next()
		.await
		.ok_or_else(|| anyhow!("subscribe_best stream empty"))?
		.map_err(Into::into)
}

/// Wait for the next best-block import and return its number. Bulk batches submitted right
/// after a fresh boundary get the full block interval to enter the pool; submitting
/// mid-interval races the author's pool snapshot and splits the batch across two blocks.
pub async fn wait_for_next_best_block(
	client: &OnlineClient<SubstrateConfig>,
	timeout_secs: u64,
) -> Result<u64> {
	let wait = async {
		let mut sub = client.blocks().subscribe_best().await?;
		let current = sub
			.next()
			.await
			.ok_or_else(|| anyhow!("subscribe_best stream empty"))??
			.number() as u64;
		loop {
			let block =
				sub.next().await.ok_or_else(|| anyhow!("subscribe_best stream ended"))??;
			if (block.number() as u64) > current {
				return anyhow::Ok(block.number() as u64);
			}
		}
	};
	tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), wait)
		.await
		.map_err(|_| anyhow!("no new best block within {}s", timeout_secs))?
}

/// Fetch the latest finalized block. Use this when event/storage reads must be stable —
/// best-view can briefly follow a non-canonical branch as chainHead_v2 resolves.
pub async fn current_finalized_block(
	client: &OnlineClient<SubstrateConfig>,
) -> Result<Block<SubstrateConfig, OnlineClient<SubstrateConfig>>> {
	Ok(client.blocks().at_latest().await?)
}

/// Locate `target` on the best chain by walking parents back from the head. Fails if the
/// chain hasn't yet reached `target` or if the walked ancestor doesn't match — callers
/// that need to outlast reorgs should use [`finalized_block_hash_at`] instead.
pub async fn block_hash_at(client: &OnlineClient<SubstrateConfig>, target: u64) -> Result<H256> {
	let mut current = current_best_block(client).await?;
	while (current.number() as u64) > target {
		let parent_hash = current.header().parent_hash;
		current = client.blocks().at(parent_hash).await?;
	}
	if (current.number() as u64) != target {
		anyhow::bail!("could not locate block {} (best chain at {})", target, current.number());
	}
	Ok(current.hash())
}

/// Locate the canonical block at `target` by walking back from the latest finalized block.
/// Polls until finality reaches `target` so callers can chain it directly after a best-block
/// wait without manually re-waiting on finality.
pub async fn finalized_block_hash_at(
	client: &OnlineClient<SubstrateConfig>,
	target: u64,
) -> Result<H256> {
	let start = std::time::Instant::now();
	let mut current = current_finalized_block(client).await?;
	while (current.number() as u64) < target {
		if start.elapsed().as_secs() > FINALIZED_POLL_TIMEOUT_SECS {
			anyhow::bail!(
				"finalized height {} did not reach target {} within {}s",
				current.number(),
				target,
				FINALIZED_POLL_TIMEOUT_SECS
			);
		}
		tokio::time::sleep(std::time::Duration::from_secs(FINALIZED_POLL_INTERVAL_SECS)).await;
		current = current_finalized_block(client).await?;
	}
	while (current.number() as u64) > target {
		let parent_hash = current.header().parent_hash;
		current = client.blocks().at(parent_hash).await?;
	}
	Ok(current.hash())
}

/// Strong assertion: exactly one `ProofChecked` event at `block`. Reads at the canonical
/// (finalized) block — caller must ensure finality has reached `block`, or rely on
/// [`finalized_block_hash_at`]'s internal poll.
pub async fn assert_proof_checked_at(
	client: &OnlineClient<SubstrateConfig>,
	block: u64,
	context: &str,
) -> Result<()> {
	let hash = finalized_block_hash_at(client, block).await?;
	let events = client.blocks().at(hash).await?.events().await?;
	let count = count_event(&events, "ProofChecked");
	assert_eq!(count, 1, "{}: expected 1 ProofChecked at block {}, saw {}", context, block, count);
	Ok(())
}
