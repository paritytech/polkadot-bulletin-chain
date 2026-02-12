// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Subxt transaction helpers: nonce management, storage operations, retention period.

use super::config::*;
use super::crypto::*;
use anyhow::{anyhow, Result};
use std::time::Duration;
use subxt::config::substrate::{SubstrateConfig, SubstrateExtrinsicParamsBuilder};
use subxt::ext::scale_value::value;
use subxt::{dynamic::tx, dynamic::Value, OnlineClient};
use subxt_signer::sr25519::dev;

pub async fn wait_for_in_best_block(
	mut progress: subxt::tx::TxProgress<SubstrateConfig, OnlineClient<SubstrateConfig>>,
) -> Result<(subxt::utils::H256, subxt::blocks::ExtrinsicEvents<SubstrateConfig>)> {
	use subxt::tx::TxStatus;

	while let Some(status) = progress.next().await {
		match status? {
			TxStatus::InBestBlock(tx_in_block) => {
				let block_hash = tx_in_block.block_hash();
				let events = tx_in_block.wait_for_success().await?;
				return Ok((block_hash, events));
			},
			TxStatus::Error { message } |
			TxStatus::Invalid { message } |
			TxStatus::Dropped { message } => {
				anyhow::bail!("Transaction failed: {}", message);
			},
			_ => continue,
		}
	}
	anyhow::bail!("Transaction stream ended without InBestBlock status")
}

/// Use for LDB tests where database state must be consistent.
pub async fn wait_for_finalized(
	mut progress: subxt::tx::TxProgress<SubstrateConfig, OnlineClient<SubstrateConfig>>,
) -> Result<(subxt::utils::H256, subxt::blocks::ExtrinsicEvents<SubstrateConfig>)> {
	use subxt::tx::TxStatus;

	while let Some(status) = progress.next().await {
		match status? {
			TxStatus::InFinalizedBlock(tx_in_block) => {
				let block_hash = tx_in_block.block_hash();
				let events = tx_in_block.wait_for_success().await?;
				return Ok((block_hash, events));
			},
			TxStatus::Error { message } |
			TxStatus::Invalid { message } |
			TxStatus::Dropped { message } => {
				anyhow::bail!("Transaction failed: {}", message);
			},
			_ => continue,
		}
	}
	anyhow::bail!("Transaction stream ended without InFinalizedBlock status")
}

pub async fn set_retention_period(
	client: &OnlineClient<SubstrateConfig>,
	retention_period: u32,
	nonce: u64,
) -> Result<()> {
	let signer = dev::alice();
	let key = retention_period_storage_key();
	let value = retention_period.to_le_bytes().to_vec();

	log::info!(
		"Setting RetentionPeriod to {} blocks via sudo (key: 0x{}, value: 0x{})",
		retention_period,
		hex::encode(&key),
		hex::encode(&value)
	);

	let items = Value::unnamed_composite([Value::unnamed_composite([
		Value::from_bytes(&key),
		Value::from_bytes(&value),
	])]);

	let set_storage_call = tx("System", "set_storage", vec![items]);
	let sudo_call = tx("Sudo", "sudo", vec![set_storage_call.into_value()]);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress = client
			.tx()
			.sign_and_submit_then_watch(&sudo_call, &signer, params)
			.await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("set_retention_period transaction timed out"))??;

	log::info!("RetentionPeriod set successfully");
	Ok(())
}

pub async fn set_retention_period_finalized(
	client: &OnlineClient<SubstrateConfig>,
	retention_period: u32,
	nonce: u64,
) -> Result<()> {
	let signer = dev::alice();
	let key = retention_period_storage_key();
	let value = retention_period.to_le_bytes().to_vec();

	log::info!(
		"Setting RetentionPeriod to {} blocks via sudo (finalized, nonce={})",
		retention_period,
		nonce
	);

	let items = Value::unnamed_composite([Value::unnamed_composite([
		Value::from_bytes(&key),
		Value::from_bytes(&value),
	])]);

	let set_storage_call = tx("System", "set_storage", vec![items]);
	let sudo_call = tx("Sudo", "sudo", vec![set_storage_call.into_value()]);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();

	tokio::time::timeout(
		Duration::from_secs(FINALIZED_TRANSACTION_TIMEOUT_SECS),
		async {
			let progress = client
				.tx()
				.sign_and_submit_then_watch(&sudo_call, &signer, params)
				.await?;
			wait_for_finalized(progress).await?;
			Ok::<_, anyhow::Error>(())
		},
	)
	.await
	.map_err(|_| anyhow!("set_retention_period_finalized transaction timed out"))??;

	log::info!("RetentionPeriod set successfully (finalized)");
	Ok(())
}

/// Returns (block_number, next_nonce). Waits for best block.
pub async fn authorize_and_store_data(
	node: &zombienet_sdk::NetworkNode,
	data: &[u8],
	mut nonce: u64,
) -> Result<(u64, u64)> {
	let client: OnlineClient<SubstrateConfig> = node.wait_client().await?;
	let signer = dev::alice();
	let bytes_to_authorize = (data.len() as u64) * 2;

	let authorize_call = subxt::tx::dynamic(
		"Sudo",
		"sudo",
		vec![value! {
			TransactionStorage(authorize_account {
				who: Value::from_bytes(signer.public_key().0),
				transactions: 10u32,
				bytes: bytes_to_authorize
			})
		}],
	);

	log::info!("Submitting authorization transaction (nonce={})...", nonce);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
	nonce += 1;

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress = client
			.tx()
			.sign_and_submit_then_watch(&authorize_call, &signer, params)
			.await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("authorization transaction timed out"))??;

	log::info!("Authorization transaction included in block");

	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);

	log::info!("Submitting store transaction (nonce={})...", nonce);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
	nonce += 1;

	let (block_hash, _events) =
		tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
			let progress = client
				.tx()
				.sign_and_submit_then_watch(&store_call, &signer, params)
				.await?;
			wait_for_in_best_block(progress).await
		})
		.await
		.map_err(|_| anyhow!("store transaction timed out"))??;

	let block = client.blocks().at(block_hash).await?;
	let block_number = block.number() as u64;

	log::info!("Store transaction included at block {}", block_number);
	Ok((block_number, nonce))
}

/// Returns (block_number, next_nonce). Waits for finalization (for LDB tests).
pub async fn authorize_and_store_data_finalized(
	node: &zombienet_sdk::NetworkNode,
	data: &[u8],
	mut nonce: u64,
) -> Result<(u64, u64)> {
	let client: OnlineClient<SubstrateConfig> = node.wait_client().await?;
	let signer = dev::alice();
	let bytes_to_authorize = (data.len() as u64) * 2;

	let authorize_call = subxt::tx::dynamic(
		"Sudo",
		"sudo",
		vec![value! {
			TransactionStorage(authorize_account {
				who: Value::from_bytes(signer.public_key().0),
				transactions: 10u32,
				bytes: bytes_to_authorize
			})
		}],
	);

	log::info!(
		"Submitting authorization transaction (finalized, nonce={})...",
		nonce
	);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
	nonce += 1;

	tokio::time::timeout(
		Duration::from_secs(FINALIZED_TRANSACTION_TIMEOUT_SECS),
		async {
			let progress = client
				.tx()
				.sign_and_submit_then_watch(&authorize_call, &signer, params)
				.await?;
			wait_for_finalized(progress).await?;
			Ok::<_, anyhow::Error>(())
		},
	)
	.await
	.map_err(|_| anyhow!("authorization transaction timed out"))??;

	log::info!("Authorization transaction finalized");

	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);

	log::info!(
		"Submitting store transaction (finalized, nonce={})...",
		nonce
	);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
	nonce += 1;

	let (block_hash, _events) = tokio::time::timeout(
		Duration::from_secs(FINALIZED_TRANSACTION_TIMEOUT_SECS),
		async {
			let progress = client
				.tx()
				.sign_and_submit_then_watch(&store_call, &signer, params)
				.await?;
			wait_for_finalized(progress).await
		},
	)
	.await
	.map_err(|_| anyhow!("store transaction timed out"))??;

	let block = client.blocks().at(block_hash).await?;
	let block_number = block.number() as u64;

	log::info!("Store transaction finalized at block {}", block_number);
	Ok((block_number, nonce))
}

pub async fn get_alice_nonce(node: &zombienet_sdk::NetworkNode) -> Result<u64> {
	let client: OnlineClient<SubstrateConfig> = node.wait_client().await?;
	let alice_account_id = dev::alice().public_key().to_account_id();
	let nonce = client.tx().account_nonce(&alice_account_id).await?;
	log::info!("Alice's current nonce: {}", nonce);
	Ok(nonce)
}
