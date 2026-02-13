// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Subxt transaction helpers: nonce management, storage operations, runtime upgrades.

use super::{config::*, crypto::*, subxt_config::*};
use anyhow::{anyhow, Result};
use std::time::Duration;
use subxt::{
	config::substrate::{SubstrateConfig, SubstrateExtrinsicParamsBuilder},
	dynamic::{tx, Value},
	ext::scale_value::value,
	OnlineClient,
};
use subxt_signer::sr25519::dev;

pub async fn wait_for_in_best_block(
	mut progress: subxt::tx::TxProgress<BulletinConfig, OnlineClient<BulletinConfig>>,
) -> Result<(subxt::utils::H256, subxt::blocks::ExtrinsicEvents<BulletinConfig>)> {
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

pub async fn set_retention_period(
	client: &OnlineClient<BulletinConfig>,
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
	let params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).build();

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress = client.tx().sign_and_submit_then_watch(&sudo_call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("set_retention_period transaction timed out"))??;

	log::info!("RetentionPeriod set successfully");
	Ok(())
}

/// Returns (block_number, next_nonce). Waits for best block.
pub async fn authorize_and_store_data(
	node: &zombienet_sdk::NetworkNode,
	data: &[u8],
	mut nonce: u64,
) -> Result<(u64, u64)> {
	let client: OnlineClient<BulletinConfig> = node.wait_client().await?;
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
	let params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).build();
	nonce += 1;

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress =
			client.tx().sign_and_submit_then_watch(&authorize_call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("authorization transaction timed out"))??;

	log::info!("Authorization transaction included in block");

	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);

	log::info!("Submitting store transaction (nonce={})...", nonce);
	let params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).build();
	nonce += 1;

	let (block_hash, _events) =
		tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
			let progress =
				client.tx().sign_and_submit_then_watch(&store_call, &signer, params).await?;
			wait_for_in_best_block(progress).await
		})
		.await
		.map_err(|_| anyhow!("store transaction timed out"))??;

	let block = client.blocks().at(block_hash).await?;
	let block_number = block.number() as u64;

	log::info!("Store transaction included at block {}", block_number);
	Ok((block_number, nonce))
}

pub async fn get_alice_nonce(node: &zombienet_sdk::NetworkNode) -> Result<u64> {
	let client: OnlineClient<BulletinConfig> = node.wait_client().await?;
	let alice_account_id = dev::alice().public_key().to_account_id();
	let nonce = client.tx().account_nonce(&alice_account_id).await?;
	log::info!("Alice's current nonce: {}", nonce);
	Ok(nonce)
}

/// Submit a parachain runtime upgrade via sudo on the parachain.
/// Uses ParachainSystem::authorize_upgrade + enact_authorized_upgrade.
pub async fn do_parachain_runtime_upgrade(
	node: &zombienet_sdk::NetworkNode,
	wasm_path: &str,
	nonce: &mut u64,
) -> Result<()> {
	let code = std::fs::read(wasm_path)
		.map_err(|e| anyhow!("Failed to read WASM from '{}': {}", wasm_path, e))?;
	log::info!("Read runtime WASM: {} bytes from {}", code.len(), wasm_path);

	let code_hash = blake2_256(&code);
	log::info!("Runtime WASM hash: 0x{}", hex::encode(code_hash));

	let client: OnlineClient<BulletinConfig> = node.wait_client().await?;
	let signer = dev::alice();

	// Step 1: authorize_upgrade_without_checks via sudo (on System pallet, skips version checks)
	let authorize_call = tx(
		"Sudo",
		"sudo_unchecked_weight",
		vec![
			value! {
				System(authorize_upgrade_without_checks {
					code_hash: Value::from_bytes(code_hash)
				})
			},
			// weight: (ref_time, proof_size)
			Value::unnamed_composite([Value::u128(0), Value::u128(0)]),
		],
	);

	log::info!("Submitting authorize_upgrade (nonce={})...", *nonce);
	let params = BulletinExtrinsicParamsBuilder::new().nonce(*nonce).build();
	*nonce += 1;

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress =
			client.tx().sign_and_submit_then_watch(&authorize_call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("authorize_upgrade transaction timed out"))??;

	log::info!("authorize_upgrade included in block");

	// Step 2: apply_authorized_upgrade (on System pallet)
	let enact_call = tx("System", "apply_authorized_upgrade", vec![Value::from_bytes(&code)]);

	log::info!("Submitting apply_authorized_upgrade (nonce={})...", *nonce);
	let params = BulletinExtrinsicParamsBuilder::new().nonce(*nonce).build();
	*nonce += 1;

	tokio::time::timeout(Duration::from_secs(FINALIZED_TRANSACTION_TIMEOUT_SECS), async {
		let progress = client.tx().sign_and_submit_then_watch(&enact_call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("enact_authorized_upgrade transaction timed out"))??;

	log::info!("enact_authorized_upgrade included — runtime upgrade scheduled");

	// Wait a few blocks for the relay chain to process the upgrade
	tokio::time::sleep(Duration::from_secs(18)).await;
	log::info!("Runtime upgrade should now be active");

	Ok(())
}

/// Force-set parachain validation code via relay chain sudo.
/// Used when the parachain is stalled and can't process its own transactions.
///
/// Uses `Paras::force_set_current_code` which immediately updates `CurrentCodeHash`
/// and `CodeByHash` on the relay chain. Relay chain validators will use the new code
/// for parachain block validation.
///
/// NOTE: This only updates the relay chain's view. The collator still uses its local
/// runtime (from `:code` storage or `codeSubstitutes`). For full recovery, combine
/// this with `codeSubstitutes` in the collator's chain spec + collator restart.
pub async fn force_parachain_code_upgrade_via_relay(
	relay_node: &zombienet_sdk::NetworkNode,
	para_id: u32,
	wasm_path: &str,
	nonce: &mut u64,
) -> Result<()> {
	let code = std::fs::read(wasm_path)
		.map_err(|e| anyhow!("Failed to read WASM from '{}': {}", wasm_path, e))?;
	log::info!(
		"Read fix runtime WASM: {} bytes from {} (for para_id={})",
		code.len(),
		wasm_path,
		para_id
	);

	// Relay chain uses standard SubstrateConfig (no ProvideCidConfig extension)
	let client: OnlineClient<SubstrateConfig> = relay_node.wait_client().await?;
	let signer = dev::alice();

	// force_set_current_code immediately updates the relay chain's parachain validation code.
	// Unlike force_schedule_code_upgrade, this does NOT go through the upgrade pipeline
	// and does NOT set UpgradeGoAheadSignal. It directly sets CurrentCodeHash + CodeByHash.
	let upgrade_call = tx(
		"Sudo",
		"sudo",
		vec![value! {
			Paras(force_set_current_code {
				para: para_id,
				new_code: Value::from_bytes(&code)
			})
		}],
	);

	log::info!(
		"Submitting relay chain Paras::force_set_current_code (nonce={}, para_id={})...",
		*nonce,
		para_id
	);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(*nonce).build();
	*nonce += 1;

	tokio::time::timeout(Duration::from_secs(FINALIZED_TRANSACTION_TIMEOUT_SECS), async {
		let progress =
			client.tx().sign_and_submit_then_watch(&upgrade_call, &signer, params).await?;
		wait_for_relay_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("relay chain force_set_current_code transaction timed out"))??;

	log::info!(
		"Relay chain force_set_current_code applied for para_id={} — validators now use fix runtime",
		para_id,
	);

	Ok(())
}

/// Wait for relay chain transaction to be included in best block.
/// Uses SubstrateConfig since relay chain doesn't have ProvideCidConfig.
async fn wait_for_relay_in_best_block(
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
