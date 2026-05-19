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

/// Submit `sudo(System::set_storage([(key, value)]))` signed by Alice — writes runtime
/// storage that has no public extrinsic.
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
	tracing::info!(
		"Setting RetentionPeriod to {} blocks via sudo (key: 0x{}, value: 0x{})",
		retention_period,
		hex::encode(&key),
		hex::encode(value),
	);
	sudo_set_storage_item(client, &key, &value, nonce, false).await?;
	tracing::info!("RetentionPeriod set successfully");
	Ok(())
}

pub async fn set_retention_period_finalized(
	client: &OnlineClient<SubstrateConfig>,
	retention_period: u32,
	nonce: u64,
) -> Result<()> {
	let key = retention_period_storage_key();
	let value = retention_period.to_le_bytes();
	tracing::info!(
		"Setting RetentionPeriod to {} blocks via sudo (finalized, nonce={})",
		retention_period,
		nonce,
	);
	sudo_set_storage_item(client, &key, &value, nonce, true).await?;
	tracing::info!("RetentionPeriod set successfully (finalized)");
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

	tracing::info!(
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

	tracing::info!("Authorization for all items included in block");

	// Store each item
	let mut stored_items = Vec::new();
	for (i, data) in items_data.into_iter().enumerate() {
		let (hash_hex, cid) = content_hash_and_cid(&data);
		tracing::info!("Storing item {} ({} bytes): hash={}, CID={}", i, data.len(), hash_hex, cid);

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
		tracing::info!("Item {} stored at block {}", i, block_number);

		stored_items.push(StoredItem { data, block_number });
	}

	Ok((stored_items, nonce))
}

/// Returns (block_number, next_nonce). Waits for best block.
/// Best-only inclusion for both the authorize and store extrinsics — caller gets fast
/// turnaround (~1 block) so any follow-up like `enable_auto_renew` can land before
/// `store_block + RP`. Returns the canonical inclusion block number read at the
/// inclusion-block hash. Callers that later assert against finalized state accept that a
/// rare reorg between best-inclusion and finality can leave the captured block number
/// pointing at an orphaned chain.
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

	tracing::info!("Submitting authorization transaction (nonce={})...", nonce);
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

	tracing::info!("Authorization transaction included in block");

	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);

	tracing::info!("Submitting store transaction (nonce={})...", nonce);
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

	// subxt's `tx_in_block.block_hash()` can name a block whose number is one ahead of the
	// canonical `Transactions[N]` key — see `canonical_store_block`.
	let content_hash = blake2_256(data);
	let block_number = canonical_store_block(&client, block_hash, &content_hash).await?;

	tracing::info!("Store transaction included at canonical block {}", block_number);
	Ok((block_number, nonce))
}

/// Returns (block_number, next_nonce). Waits for finalization (for LDB / sync tests where
/// the captured block number must be on the canonical chain *before* the test proceeds).
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

	tracing::info!("Submitting authorization transaction (finalized, nonce={})...", nonce);
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

	tracing::info!("Authorization transaction finalized");

	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);

	tracing::info!("Submitting store transaction (finalized, nonce={})...", nonce);
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

	tracing::info!("Store transaction finalized at block {}", block_number);
	Ok((block_number, nonce))
}

/// Sudo'd `TransactionStorage::authorize_account` for `who`. Additive on the unexpired path.
pub async fn authorize_account_via_sudo(
	client: &OnlineClient<SubstrateConfig>,
	who: &[u8; 32],
	transactions: u32,
	bytes: u64,
	nonce: u64,
) -> Result<()> {
	authorize_account_via_sudo_inner(client, who, transactions, bytes, nonce, false).await
}

/// Like [`authorize_account_via_sudo`] but waits for finality — required when `who`'s next
/// step is its first signed extrinsic, since the pool's `validate_signed` reads finalized
/// state.
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

	tracing::info!(
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

	tracing::info!("Authorization included");
	Ok(())
}

pub async fn top_up_alice_authorization(
	client: &OnlineClient<SubstrateConfig>,
	transactions: u32,
	bytes: u64,
	nonce: u64,
) -> Result<()> {
	let alice_pk = dev::alice().public_key().0;
	authorize_account_via_sudo(client, &alice_pk, transactions, bytes, nonce).await
}

/// Signed `store(data)` from Alice; caller ensures Alice is authorized. Returns the
/// canonical inclusion block number (see [`canonical_store_block`]). Waits for best-only
/// inclusion; callers accept the rare-reorg risk on later finalized-state reads.
pub async fn submit_store_signed(
	client: &OnlineClient<SubstrateConfig>,
	data: &[u8],
	nonce: u64,
) -> Result<u64> {
	let signer = dev::alice();
	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();

	tracing::info!("Submitting store (nonce={}, {} bytes)...", nonce, data.len());

	let (block_hash, _events) =
		tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
			let progress =
				client.tx().sign_and_submit_then_watch(&store_call, &signer, params).await?;
			wait_for_in_best_block(progress).await
		})
		.await
		.map_err(|_| anyhow!("store transaction timed out"))??;

	let content_hash = blake2_256(data);
	let block_number = canonical_store_block(client, block_hash, &content_hash).await?;
	tracing::info!("Store transaction included at canonical block {}", block_number);
	Ok(block_number)
}

/// Resolve the canonical store block by walking the **finalized** chain backward from the
/// latest finalized block until a `TransactionStorage::Stored` event whose payload contains
/// `content_hash` is found. Robust against best-vs-finalized reorgs: subxt's
/// `wait_for_in_best_block` can name an inclusion block that gets orphaned, so the eager
/// `canonical_store_block` reading at that hash returns a number not on the new canonical
/// chain. This walk re-anchors against finality.
///
/// Caller must already have waited for finality to cover the expected store block (e.g. via
/// `wait_for_finalized_height`). `search_from_inclusive` bounds how far back the walk
/// descends — pass the best-reported store block minus a small reorg-depth buffer.
pub async fn resolve_canonical_store_block(
	client: &OnlineClient<SubstrateConfig>,
	content_hash: &[u8; 32],
	search_from_inclusive: u64,
) -> Result<u64> {
	// 32-byte sliding-window match against the event's scale-encoded field bytes —
	// `Stored { index: u32, content_hash: [u8; 32], cid: Option<Cid> }`. Random collisions
	// at 32 bytes are astronomically unlikely.
	let mut current = client.blocks().at_latest().await?;
	loop {
		let block_n = current.number() as u64;
		if block_n < search_from_inclusive {
			break;
		}
		let events = current.events().await?;
		for ev in events.iter().filter_map(|e| e.ok()) {
			if ev.pallet_name() == "TransactionStorage" &&
				ev.variant_name() == "Stored" &&
				ev.field_bytes().windows(32).any(|w| w == content_hash)
			{
				return Ok(block_n);
			}
		}
		if block_n == 0 {
			break;
		}
		let parent_hash = current.header().parent_hash;
		current = client.blocks().at(parent_hash).await?;
	}
	anyhow::bail!(
		"Stored event for content_hash 0x{} not found on finalized chain at/above block {}",
		hex::encode(content_hash),
		search_from_inclusive,
	)
}

/// Canonical store/renew block number, read from `TransactionByContentHash` at the
/// inclusion-block hash. subxt's `tx_in_block.block_hash()` can name a block whose
/// `block.number()` is one ahead of the canonical `Transactions[N]` key the pallet uses to
/// schedule auto-renewal; reading at the inclusion-block state returns the authoritative
/// number.
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

/// Two `renew` calls signed by Alice and Bob respectively. `validate_signed` tags renewals
/// with `(who, content_hash)`, so two renews of the same data from the same signer would
/// conflict in the pool.
pub async fn submit_renew_pair(
	client: &OnlineClient<SubstrateConfig>,
	block: u32,
	index: u32,
	content_hash: &[u8; 32],
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

	tracing::info!(
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

	let block_alice = canonical_store_block(client, hash_alice, content_hash).await?;
	let block_bob = canonical_store_block(client, hash_bob, content_hash).await?;
	tracing::info!(
		"renew(block={}, idx={}) canonical inclusions: alice={}, bob={}",
		block,
		index,
		block_alice,
		block_bob,
	);
	Ok((block_alice, block_bob))
}

/// Signed `enable_auto_renew(content_hash)` from Alice.
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

	tracing::info!("Submitting enable_auto_renew (nonce={})...", nonce);

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress = client.tx().sign_and_submit_then_watch(&call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("enable_auto_renew transaction timed out"))??;

	tracing::info!("enable_auto_renew included in block");
	Ok(())
}

/// Signed `disable_auto_renew(content_hash)` from Alice. Required at the end of
/// shared-harness tests to avoid renewals consuming Alice's authorization indefinitely.
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

	tracing::info!("Submitting disable_auto_renew (nonce={})...", nonce);

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress = client.tx().sign_and_submit_then_watch(&call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("disable_auto_renew transaction timed out"))??;

	tracing::info!("disable_auto_renew included in block");
	Ok(())
}

/// Submit `sudo(TxPause::pause((pallet, call)))` signed by Alice. The `full_name` is encoded
/// as a `(BoundedVec<u8>, BoundedVec<u8>)` tuple matching `pallet_tx_pause::RuntimeCallNameOf`.
pub async fn sudo_tx_pause(
	client: &OnlineClient<SubstrateConfig>,
	pallet: &[u8],
	call: &[u8],
	nonce: u64,
) -> Result<()> {
	sudo_tx_pause_inner(client, pallet, call, nonce, "pause").await
}

/// Symmetric counterpart to [`sudo_tx_pause`].
pub async fn sudo_tx_unpause(
	client: &OnlineClient<SubstrateConfig>,
	pallet: &[u8],
	call: &[u8],
	nonce: u64,
) -> Result<()> {
	sudo_tx_pause_inner(client, pallet, call, nonce, "unpause").await
}

async fn sudo_tx_pause_inner(
	client: &OnlineClient<SubstrateConfig>,
	pallet: &[u8],
	call: &[u8],
	nonce: u64,
	method: &'static str,
) -> Result<()> {
	let signer = dev::alice();
	let full_name = Value::unnamed_composite([Value::from_bytes(pallet), Value::from_bytes(call)]);
	let inner = tx("TxPause", method, vec![full_name]);
	let sudo_call = tx("Sudo", "sudo", vec![inner.into_value()]);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();

	tracing::info!(
		"Submitting sudo(TxPause::{}((pallet={}, call={}))), nonce={}",
		method,
		core::str::from_utf8(pallet).unwrap_or("<non-utf8>"),
		core::str::from_utf8(call).unwrap_or("<non-utf8>"),
		nonce,
	);

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress = client.tx().sign_and_submit_then_watch(&sudo_call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("sudo TxPause::{} timed out", method))??;
	Ok(())
}

/// Single-signer `renew(block, index)` from Alice. Waits for inclusion and surfaces
/// dispatch failures via `wait_for_success`.
pub async fn submit_signed_renew(
	client: &OnlineClient<SubstrateConfig>,
	block: u32,
	index: u32,
	nonce: u64,
) -> Result<()> {
	let signer = dev::alice();
	let renew_call = tx(
		"TransactionStorage",
		"renew",
		vec![Value::u128(block as u128), Value::u128(index as u128)],
	);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();

	tracing::info!("Submitting renew(block={}, index={}) (nonce={})", block, index, nonce);

	tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress = client.tx().sign_and_submit_then_watch(&renew_call, &signer, params).await?;
		wait_for_in_best_block(progress).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.map_err(|_| anyhow!("renew transaction timed out"))??;
	Ok(())
}

/// Submit a `renew(block, index)` signed by Alice and assert it fails with `CallFiltered`
/// at dispatch (the tx is included in a block, but `BaseCallFilter` rejects the call).
pub async fn submit_renew_expecting_filtered(
	client: &OnlineClient<SubstrateConfig>,
	block: u32,
	index: u32,
	nonce: u64,
) -> Result<()> {
	let signer = dev::alice();
	let renew_call = tx(
		"TransactionStorage",
		"renew",
		vec![Value::u128(block as u128), Value::u128(index as u128)],
	);
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).build();

	tracing::info!(
		"Submitting renew(block={}, index={}) expecting CallFiltered (nonce={})",
		block,
		index,
		nonce
	);

	let result = tokio::time::timeout(Duration::from_secs(TRANSACTION_TIMEOUT_SECS), async {
		let progress = client.tx().sign_and_submit_then_watch(&renew_call, &signer, params).await?;
		wait_for_in_best_block(progress).await
	})
	.await
	.map_err(|_| anyhow!("renew (expecting CallFiltered) timed out"))?;

	match result {
		Ok(_) => Err(anyhow!("renew was dispatched while paused; expected CallFiltered")),
		Err(e) => {
			let msg = format!("{:?}", e);
			if msg.contains("CallFiltered") {
				tracing::info!("✓ renew rejected as expected (CallFiltered)");
				Ok(())
			} else {
				Err(anyhow!("renew failed, but not with CallFiltered: {}", msg))
			}
		},
	}
}

pub async fn get_alice_nonce(node: &zombienet_sdk::NetworkNode) -> Result<u64> {
	let client: OnlineClient<SubstrateConfig> = node.wait_client().await?;
	let alice_account_id = dev::alice().public_key().to_account_id();
	let nonce = client.tx().account_nonce(&alice_account_id).await?;
	tracing::info!("Alice's current nonce: {}", nonce);
	Ok(nonce)
}

/// Mirrors the pallet's `Authorization<BlockNumber>` SCALE layout (encoded as a tuple).
pub struct AuthorizationOverride {
	pub transactions: u32,
	pub transactions_allowance: u32,
	pub bytes: u64,
	pub bytes_permanent: u64,
	pub bytes_allowance: u64,
	pub expiration: u32,
}

/// Overwrite Alice's `Authorizations[Account(alice)]` entry via `sudo(System::set_storage)`.
/// `authorize_account` cannot shrink an existing entry or set a custom expiration block.
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

	tracing::info!(
		"Overriding Alice's Authorization (expiration={}, bytes_permanent={}, \
		 bytes_allowance={}) via sudo set_storage",
		auth.expiration,
		auth.bytes_permanent,
		auth.bytes_allowance,
	);
	sudo_set_storage_item(client, &key, &value, nonce, false).await?;
	tracing::info!("Alice's Authorization overridden");
	Ok(())
}
