// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Subxt transaction helpers: nonce management, storage operations, retention period.

use super::{config::*, crypto::*};
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

/// Submit `sudo(System::set_storage([(key, value)]))` signed by Alice. Used to write to
/// runtime storage that has no public extrinsic — RetentionPeriod, Authorizations, etc.
/// `wait_for_finality` selects in-best-block vs finalized progress wait.
async fn sudo_set_storage_item(
	client: &OnlineClient<SubstrateConfig>,
	key: &[u8],
	value: &[u8],
	nonce: u64,
	wait_for_finality: bool,
) -> Result<()> {
	let signer = dev::alice();
	let items = Value::unnamed_composite([Value::unnamed_composite([
		Value::from_bytes(key),
		Value::from_bytes(value),
	])]);
	let set_storage_call = tx("System", "set_storage", vec![items]);
	let sudo_call = tx("Sudo", "sudo", vec![set_storage_call.into_value()]);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
	let timeout = if wait_for_finality {
		FINALIZED_TRANSACTION_TIMEOUT_SECS
	} else {
		TRANSACTION_TIMEOUT_SECS
	};

	tokio::time::timeout(Duration::from_secs(timeout), async {
		let progress = client.tx().sign_and_submit_then_watch(&sudo_call, &signer, params).await?;
		if wait_for_finality {
			wait_for_finalized(progress).await?;
		} else {
			wait_for_in_best_block(progress).await?;
		}
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("sudo set_storage timed out"))??;
	Ok(())
}

pub async fn set_retention_period(
	client: &OnlineClient<SubstrateConfig>,
	retention_period: u32,
	nonce: u64,
) -> Result<()> {
	let key = retention_period_storage_key();
	let value = retention_period.to_le_bytes();
	log::info!(
		"Setting RetentionPeriod to {} blocks via sudo (key: 0x{}, value: 0x{})",
		retention_period,
		hex::encode(&key),
		hex::encode(value),
	);
	sudo_set_storage_item(client, &key, &value, nonce, false).await?;
	log::info!("RetentionPeriod set successfully");
	Ok(())
}

pub async fn set_retention_period_finalized(
	client: &OnlineClient<SubstrateConfig>,
	retention_period: u32,
	nonce: u64,
) -> Result<()> {
	let key = retention_period_storage_key();
	let value = retention_period.to_le_bytes();
	log::info!(
		"Setting RetentionPeriod to {} blocks via sudo (finalized, nonce={})",
		retention_period,
		nonce,
	);
	sudo_set_storage_item(client, &key, &value, nonce, true).await?;
	log::info!("RetentionPeriod set successfully (finalized)");
	Ok(())
}

/// A stored data item with its content and metadata for verification.
#[derive(Clone)]
pub struct StoredItem {
	pub data: Vec<u8>,
	pub block_number: u64,
}

/// Authorize and store multiple distinct data items. Returns the stored items and next nonce.
///
/// Each item gets a unique pattern suffix to ensure distinct content hashes.
/// Authorization is done once upfront for all items.
pub async fn authorize_and_store_items(
	node: &zombienet_sdk::NetworkNode,
	base_pattern: &[u8],
	item_sizes: &[usize],
	mut nonce: u64,
) -> Result<(Vec<StoredItem>, u64)> {
	let client: OnlineClient<SubstrateConfig> = node.wait_client().await?;
	let signer = dev::alice();

	// Generate distinct data for each item by appending an index suffix to the pattern
	let items_data: Vec<Vec<u8>> = item_sizes
		.iter()
		.enumerate()
		.map(|(i, &size)| {
			let mut pattern = base_pattern.to_vec();
			pattern.extend_from_slice(format!("ITEM_{}_", i).as_bytes());
			generate_test_data(size, &pattern)
		})
		.collect();

	// Authorize enough bytes for all items
	let total_bytes: u64 = items_data.iter().map(|d| d.len() as u64).sum::<u64>() * 2;
	let total_transactions = items_data.len() as u32 + 5; // extra margin

	let authorize_call = subxt::tx::dynamic(
		"Sudo",
		"sudo",
		vec![value! {
			TransactionStorage(authorize_account {
				who: Value::from_bytes(signer.public_key().0),
				transactions: total_transactions,
				bytes: total_bytes
			})
		}],
	);

	log::info!(
		"Authorizing {} items ({} bytes total, nonce={})",
		items_data.len(),
		total_bytes,
		nonce
	);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
	nonce += 1;

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress =
			client.tx().sign_and_submit_then_watch(&authorize_call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("authorization transaction timed out"))??;

	log::info!("Authorization for all items included in block");

	// Store each item
	let mut stored_items = Vec::new();
	for (i, data) in items_data.into_iter().enumerate() {
		let (hash_hex, cid) = content_hash_and_cid(&data);
		log::info!("Storing item {} ({} bytes): hash={}, CID={}", i, data.len(), hash_hex, cid);

		let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(&data)]);
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
		nonce += 1;

		let (block_hash, _events) =
			tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
				let progress =
					client.tx().sign_and_submit_then_watch(&store_call, &signer, params).await?;
				wait_for_in_best_block(progress).await
			})
			.await
			.map_err(|_| anyhow!("store transaction for item {} timed out", i))??;

		let block = client.blocks().at(block_hash).await?;
		let block_number = block.number() as u64;
		log::info!("Item {} stored at block {}", i, block_number);

		stored_items.push(StoredItem { data, block_number });
	}

	Ok((stored_items, nonce))
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
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
	nonce += 1;

	let (block_hash, _events) =
		tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
			let progress =
				client.tx().sign_and_submit_then_watch(&store_call, &signer, params).await?;
			wait_for_in_best_block(progress).await
		})
		.await
		.map_err(|_| anyhow!("store transaction timed out"))??;

	// Read the authoritative block from `TransactionByContentHash` at the inclusion block —
	// subxt's `tx_in_block.block_hash()` can name a block whose `block.number()` is one ahead
	// of the canonical key in `Transactions[N]` (the latter is what `on_initialize` uses to
	// schedule auto-renewal). See `canonical_store_block` for the full rationale.
	let content_hash = blake2_256(data);
	let block_number = canonical_store_block(&client, block_hash, &content_hash).await?;

	log::info!("Store transaction included at canonical block {}", block_number);
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

	log::info!("Submitting authorization transaction (finalized, nonce={})...", nonce);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
	nonce += 1;

	tokio::time::timeout(Duration::from_secs(FINALIZED_TRANSACTION_TIMEOUT_SECS), async {
		let progress =
			client.tx().sign_and_submit_then_watch(&authorize_call, &signer, params).await?;
		wait_for_finalized(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("authorization transaction timed out"))??;

	log::info!("Authorization transaction finalized");

	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);

	log::info!("Submitting store transaction (finalized, nonce={})...", nonce);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();
	nonce += 1;

	let (block_hash, _events) =
		tokio::time::timeout(Duration::from_secs(FINALIZED_TRANSACTION_TIMEOUT_SECS), async {
			let progress =
				client.tx().sign_and_submit_then_watch(&store_call, &signer, params).await?;
			wait_for_finalized(progress).await
		})
		.await
		.map_err(|_| anyhow!("store transaction timed out"))??;

	let block = client.blocks().at(block_hash).await?;
	let block_number = block.number() as u64;

	log::info!("Store transaction finalized at block {}", block_number);
	Ok((block_number, nonce))
}

/// Submit a sudo'd `TransactionStorage::authorize_account` for `who` with the given capacity,
/// adding to any existing authorization. Each successful store/renew consumes one transaction
/// slot and `data.len()` bytes from this allowance. Signed by Alice (the sudo key).
///
/// `wait_for_finality` selects in-best-block vs finalized progress wait. Use the finalized
/// variant whenever the next step is a signed extrinsic from `who` and the pool's
/// `validate_signed` would otherwise see a stale (pre-authorize) finalized state and reject
/// the tx with `InvalidTransaction`. The non-finalized variant is fine when `who` already
/// had authorization (top-up scenarios), when the next dependent action is many blocks away,
/// or when the caller batches a finalization wait afterward.
pub async fn authorize_account_via_sudo(
	client: &OnlineClient<SubstrateConfig>,
	who: &[u8; 32],
	transactions: u32,
	bytes: u64,
	nonce: u64,
) -> Result<()> {
	authorize_account_via_sudo_inner(client, who, transactions, bytes, nonce, false).await
}

/// Same as [`authorize_account_via_sudo`] but blocks until the authorize tx is finalized.
/// Use when the immediate next step is a signed extrinsic from `who` (e.g. a fresh account
/// whose first tx would otherwise race the pool's finalized-state validation).
pub async fn authorize_account_via_sudo_finalized(
	client: &OnlineClient<SubstrateConfig>,
	who: &[u8; 32],
	transactions: u32,
	bytes: u64,
	nonce: u64,
) -> Result<()> {
	authorize_account_via_sudo_inner(client, who, transactions, bytes, nonce, true).await
}

async fn authorize_account_via_sudo_inner(
	client: &OnlineClient<SubstrateConfig>,
	who: &[u8; 32],
	transactions: u32,
	bytes: u64,
	nonce: u64,
	wait_for_finality: bool,
) -> Result<()> {
	let alice = dev::alice();
	let authorize_call = subxt::tx::dynamic(
		"Sudo",
		"sudo",
		vec![value! {
			TransactionStorage(authorize_account {
				who: Value::from_bytes(*who),
				transactions: transactions,
				bytes: bytes
			})
		}],
	);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();

	log::info!(
		"Authorizing 0x{}.. (+{} tx, +{} bytes, alice nonce={}, wait_for_finality={})",
		hex::encode(&who[..4]),
		transactions,
		bytes,
		nonce,
		wait_for_finality,
	);

	let timeout = if wait_for_finality {
		FINALIZED_TRANSACTION_TIMEOUT_SECS
	} else {
		TRANSACTION_TIMEOUT_SECS
	};
	tokio::time::timeout(Duration::from_secs(timeout), async {
		let progress =
			client.tx().sign_and_submit_then_watch(&authorize_call, &alice, params).await?;
		if wait_for_finality {
			wait_for_finalized(progress).await?;
		} else {
			wait_for_in_best_block(progress).await?;
		}
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("authorize_account via sudo timed out"))??;

	log::info!("Authorization included");
	Ok(())
}

/// Convenience wrapper around [`authorize_account_via_sudo`] for Alice.
pub async fn top_up_alice_authorization(
	client: &OnlineClient<SubstrateConfig>,
	transactions: u32,
	bytes: u64,
	nonce: u64,
) -> Result<()> {
	let alice_pk = dev::alice().public_key().0;
	authorize_account_via_sudo(client, &alice_pk, transactions, bytes, nonce).await
}

/// Submit a single `TransactionStorage::store(data)` signed by Alice. Caller is responsible
/// for ensuring Alice has sufficient pre-existing authorization. Returns the inclusion block
/// number.
pub async fn submit_store_signed(
	client: &OnlineClient<SubstrateConfig>,
	data: &[u8],
	nonce: u64,
) -> Result<u64> {
	let signer = dev::alice();
	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();

	log::info!("Submitting store (nonce={}, {} bytes)...", nonce, data.len());

	let (block_hash, _events) =
		tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
			let progress =
				client.tx().sign_and_submit_then_watch(&store_call, &signer, params).await?;
			wait_for_in_best_block(progress).await
		})
		.await
		.map_err(|_| anyhow!("store transaction timed out"))??;

	// See `authorize_and_store_data` / `canonical_store_block` for why we read the canonical
	// block at the inclusion block hash instead of trusting subxt's reported block number.
	let content_hash = blake2_256(data);
	let block_number = canonical_store_block(client, block_hash, &content_hash).await?;
	log::info!("Store transaction included at canonical block {}", block_number);
	Ok(block_number)
}

/// Read `TransactionByContentHash[content_hash]` at the supplied block hash and return the
/// canonical block number where the store/renewal was indexed. Reading AT the inclusion
/// block (not `at_latest`) avoids the case where subxt's chainHead_v2 backend's latest view
/// is still pinned to a finalized block that doesn't yet reflect the just-included tx. The
/// pallet stores `(frame_system::block_number(), index)` at extrinsic-execution time, so
/// reading at the inclusion block's state always returns the authoritative block number —
/// even if subxt's `tx_in_block.block_hash()` later turns out to name a short-lived fork
/// candidate, the storage map at the SAME block-hash references is internally consistent.
pub async fn canonical_store_block(
	client: &OnlineClient<SubstrateConfig>,
	at_block_hash: subxt::utils::H256,
	content_hash: &[u8; 32],
) -> Result<u64> {
	let address = subxt::dynamic::storage(
		"TransactionStorage",
		"TransactionByContentHash",
		vec![Value::from_bytes(content_hash.as_slice())],
	);
	let value = client.storage().at(at_block_hash).fetch(&address).await?.ok_or_else(|| {
		anyhow!(
			"TransactionByContentHash[0x{}] is empty at block 0x{} — the store extrinsic \
				 should have populated this entry in the same block",
			hex::encode(content_hash),
			hex::encode(&at_block_hash.0[..8]),
		)
	})?;
	use subxt::ext::scale_value::{Primitive, ValueDef};
	let decoded = value.to_value()?;
	let block_number = match decoded.value {
		ValueDef::Composite(ref c) => c
			.values()
			.next()
			.and_then(|v| match &v.value {
				ValueDef::Primitive(Primitive::U128(n)) => Some(*n),
				_ => None,
			})
			.ok_or_else(|| {
				anyhow!("TransactionByContentHash value composite empty or non-numeric")
			})?,
		_ => anyhow::bail!("unexpected TransactionByContentHash value shape: {:?}", decoded),
	};
	Ok(block_number as u64)
}

/// Submit two `TransactionStorage::renew(block, index)` extrinsics back-to-back, signed by
/// **Alice and Bob respectively**. The pallet's `validate_signed` adds a `provides((who,
/// content_hash))` tag, so two renews of the same data from the **same** signer would
/// conflict in the pool — using two distinct signers avoids the conflict so both can sit in
/// the pool simultaneously and land in the same block. Caller must have authorized Bob for at
/// least one renew worth of capacity. Returns each renew's inclusion block number.
pub async fn submit_renew_pair(
	client: &OnlineClient<SubstrateConfig>,
	block: u32,
	index: u32,
	alice_nonce: u64,
	bob_nonce: u64,
) -> Result<(u64, u64)> {
	let alice = dev::alice();
	let bob = dev::bob();
	let renew_call = tx(
		"TransactionStorage",
		"renew",
		vec![Value::u128(block as u128), Value::u128(index as u128)],
	);
	let alice_params = SubstrateExtrinsicParamsBuilder::new().nonce(alice_nonce).build();
	let bob_params = SubstrateExtrinsicParamsBuilder::new().nonce(bob_nonce).build();

	log::info!(
		"Submitting two renew(block={}, index={}) calls (alice nonce={}, bob nonce={})",
		block,
		index,
		alice_nonce,
		bob_nonce
	);

	let alice_progress = client
		.tx()
		.sign_and_submit_then_watch(&renew_call, &alice, alice_params)
		.await?;
	let bob_progress =
		client.tx().sign_and_submit_then_watch(&renew_call, &bob, bob_params).await?;

	let (hash_alice, _) = tokio::time::timeout(
		Duration::from_secs(TRANSACTION_TIMEOUT_SECS),
		wait_for_in_best_block(alice_progress),
	)
	.await
	.map_err(|_| anyhow!("alice renew timed out"))??;
	let (hash_bob, _) = tokio::time::timeout(
		Duration::from_secs(TRANSACTION_TIMEOUT_SECS),
		wait_for_in_best_block(bob_progress),
	)
	.await
	.map_err(|_| anyhow!("bob renew timed out"))??;

	let block_alice = client.blocks().at(hash_alice).await?.number() as u64;
	let block_bob = client.blocks().at(hash_bob).await?.number() as u64;
	log::info!(
		"renew(block={}, idx={}) inclusions: alice={}, bob={}",
		block,
		index,
		block_alice,
		block_bob
	);
	Ok((block_alice, block_bob))
}

/// Submit `TransactionStorage::enable_auto_renew(content_hash)` signed by Alice. The
/// account must already have authorization sufficient for one renewal cycle.
pub async fn enable_auto_renew(
	client: &OnlineClient<SubstrateConfig>,
	content_hash: &[u8; 32],
	nonce: u64,
) -> Result<()> {
	let signer = dev::alice();
	let call = tx(
		"TransactionStorage",
		"enable_auto_renew",
		vec![Value::from_bytes(content_hash.as_slice())],
	);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();

	log::info!("Submitting enable_auto_renew (nonce={})...", nonce);

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress = client.tx().sign_and_submit_then_watch(&call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("enable_auto_renew transaction timed out"))??;

	log::info!("enable_auto_renew included in block");
	Ok(())
}

/// Submit `TransactionStorage::disable_auto_renew(content_hash)` signed by Alice. Removes
/// the content hash from `AutoRenewals` so the auto-renew inherent stops processing it on
/// subsequent renewal blocks. Use at the end of shared-harness tests that called
/// `enable_auto_renew`: without this the chain keeps re-storing the item until Alice's
/// authorization runs out, polluting state for later tests in the group.
pub async fn disable_auto_renew(
	client: &OnlineClient<SubstrateConfig>,
	content_hash: &[u8; 32],
	nonce: u64,
) -> Result<()> {
	let signer = dev::alice();
	let call = tx(
		"TransactionStorage",
		"disable_auto_renew",
		vec![Value::from_bytes(content_hash.as_slice())],
	);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();

	log::info!("Submitting disable_auto_renew (nonce={})...", nonce);

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress = client.tx().sign_and_submit_then_watch(&call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("disable_auto_renew transaction timed out"))??;

	log::info!("disable_auto_renew included in block");
	Ok(())
}

pub async fn get_alice_nonce(node: &zombienet_sdk::NetworkNode) -> Result<u64> {
	let client: OnlineClient<SubstrateConfig> = node.wait_client().await?;
	let alice_account_id = dev::alice().public_key().to_account_id();
	let nonce = client.tx().account_nonce(&alice_account_id).await?;
	log::info!("Alice's current nonce: {}", nonce);
	Ok(nonce)
}

/// Fields to write into Alice's `Authorization` entry; mirrors the pallet's
/// `Authorization<BlockNumber>` SCALE layout (encoded as a tuple in that order).
pub struct AuthorizationOverride {
	pub transactions: u32,
	pub transactions_allowance: u32,
	pub bytes: u64,
	pub bytes_permanent: u64,
	pub bytes_allowance: u64,
	pub expiration: u32,
}

/// Overwrite Alice's `Authorizations[Account(alice)]` entry via `sudo(System::set_storage(..))`.
/// There is no extrinsic that lets a caller set a custom expiration block on an authorization,
/// so tests that need a short expiration (vs. the runtime's `AuthorizationPeriod = 14 * DAYS`)
/// must write the storage entry directly. The key is computed from metadata via
/// `address_bytes`, so the correct `Blake2_128Concat` hasher is applied automatically.
pub async fn override_alice_authorization(
	client: &OnlineClient<SubstrateConfig>,
	auth: AuthorizationOverride,
	nonce: u64,
) -> Result<()> {
	use subxt::ext::{
		codec::Encode,
		scale_value::{Composite, Value as ScaleValue},
	};

	let alice_pk = dev::alice().public_key().0;
	let scope_value =
		ScaleValue::variant("Account", Composite::Unnamed(vec![ScaleValue::from_bytes(alice_pk)]));
	let address =
		subxt::dynamic::storage("TransactionStorage", "Authorizations", vec![scope_value]);
	let key = client.storage().address_bytes(&address)?;
	let value = (
		auth.transactions,
		auth.transactions_allowance,
		auth.bytes,
		auth.bytes_permanent,
		auth.bytes_allowance,
		auth.expiration,
	)
		.encode();

	log::info!(
		"Overriding Alice's Authorization (expiration={}, bytes_permanent={}, \
		 bytes_allowance={}) via sudo set_storage",
		auth.expiration,
		auth.bytes_permanent,
		auth.bytes_allowance,
	);
	sudo_set_storage_item(client, &key, &value, nonce, false).await?;
	log::info!("Alice's Authorization overridden");
	Ok(())
}
