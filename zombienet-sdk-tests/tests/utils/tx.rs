// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Transaction helpers: nonce management, storage operations, retention period.

use super::{config::*, crypto::*, sdk::*};
use anyhow::{anyhow, Result};
use bulletin_sdk_rust::{AuthorizeAccountEntry, TransactionClient, WaitFor};
use std::time::Duration;
use subxt::{
	config::substrate::{SubstrateConfig, SubstrateExtrinsicParamsBuilder},
	dynamic::{tx, Value},
	OnlineClient,
};
use subxt_signer::sr25519::dev;

fn transaction_timeout(wait_for: WaitFor) -> Duration {
	Duration::from_secs(match wait_for {
		WaitFor::InBlock => TRANSACTION_TIMEOUT_SECS,
		WaitFor::Finalized => FINALIZED_TRANSACTION_TIMEOUT_SECS,
	})
}

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
	nonce: u64,
) -> Result<(Vec<StoredItem>, u64)> {
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

	tracing::info!("Authorizing {} items", items_data.len());
	let sdk = sdk_client(node).await?;
	authorize_account_via_sudo(&sdk, &signer.public_key().0, total_transactions, total_bytes)
		.await?;

	// Store each item
	let mut stored_items = Vec::new();
	for (i, data) in items_data.into_iter().enumerate() {
		let (hash_hex, cid) = content_hash_and_cid(&data);
		tracing::info!("Storing item {} ({} bytes): hash={}, CID={}", i, data.len(), hash_hex, cid);

		let receipt = tokio::time::timeout(
			transaction_timeout(WaitFor::InBlock),
			sdk.store(data.clone(), &signer, WaitFor::InBlock),
		)
		.await
		.map_err(|_| anyhow!("store transaction for item {} timed out", i))??;

		let block_hash = receipt_block_hash(&receipt.block_hash)?;
		let block_number = sdk.api().blocks().at(block_hash).await?.number() as u64;
		tracing::info!("Item {} stored at block {}", i, block_number);

		stored_items.push(StoredItem { data, block_number });
	}

	let next_nonce = nonce + 1 + stored_items.len() as u64;
	Ok((stored_items, next_nonce))
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
	nonce: u64,
) -> Result<(u64, u64)> {
	let sdk = sdk_client(node).await?;
	top_up_alice_authorization(&sdk, 10, (data.len() as u64) * 2).await?;
	let block_number = submit_store_signed(&sdk, data).await?;
	Ok((block_number, nonce + 2))
}

/// Returns the block number. Waits for finalization (for LDB / sync tests where the captured
/// block number must be on the canonical chain *before* the test proceeds).
pub async fn authorize_and_store_data_finalized(
	node: &zombienet_sdk::NetworkNode,
	data: &[u8],
) -> Result<u64> {
	let signer = dev::alice();
	let sdk = sdk_client(node).await?;
	authorize_account_via_sudo_finalized(&sdk, &signer.public_key().0, 10, (data.len() as u64) * 2)
		.await?;

	tracing::info!("Submitting store transaction (finalized)...");
	let receipt = tokio::time::timeout(
		transaction_timeout(WaitFor::Finalized),
		sdk.store(data.to_vec(), &signer, WaitFor::Finalized),
	)
	.await
	.map_err(|_| anyhow!("store transaction timed out"))??;

	let block_hash = receipt_block_hash(&receipt.block_hash)?;
	let block_number = sdk.api().blocks().at(block_hash).await?.number() as u64;

	tracing::info!("Store transaction finalized at block {}", block_number);
	Ok(block_number)
}

/// Sudo'd `TransactionStorage::authorize_account` for `who`. Additive on the unexpired path.
pub async fn authorize_account_via_sudo(
	sdk: &TransactionClient,
	who: &[u8; 32],
	transactions: u32,
	bytes: u64,
) -> Result<()> {
	authorize_account_via_sudo_inner(sdk, who, transactions, bytes, WaitFor::InBlock).await
}

/// Like [`authorize_account_via_sudo`] but waits for finality — required when `who`'s next
/// step is its first signed extrinsic, since the pool's `validate_signed` reads finalized
/// state.
pub async fn authorize_account_via_sudo_finalized(
	sdk: &TransactionClient,
	who: &[u8; 32],
	transactions: u32,
	bytes: u64,
) -> Result<()> {
	authorize_account_via_sudo_inner(sdk, who, transactions, bytes, WaitFor::Finalized).await
}

async fn authorize_account_via_sudo_inner(
	sdk: &TransactionClient,
	who: &[u8; 32],
	transactions: u32,
	bytes: u64,
	wait_for: WaitFor,
) -> Result<()> {
	tracing::info!(
		"Authorizing 0x{}.. (+{} tx, +{} bytes, {:?})",
		hex::encode(&who[..4]),
		transactions,
		bytes,
		wait_for,
	);

	let entry = AuthorizeAccountEntry { who: (*who).into(), transactions, bytes };
	tokio::time::timeout(
		transaction_timeout(wait_for),
		sdk.authorize_accounts(vec![entry], true, &dev::alice(), wait_for),
	)
	.await
	.map_err(|_| anyhow!("authorize_account via sudo timed out"))??;

	tracing::info!("Authorization included");
	Ok(())
}

pub async fn top_up_alice_authorization(
	sdk: &TransactionClient,
	transactions: u32,
	bytes: u64,
) -> Result<()> {
	let alice_pk = dev::alice().public_key().0;
	authorize_account_via_sudo(sdk, &alice_pk, transactions, bytes).await
}

/// Signed `store(data)` from Alice; caller ensures Alice is authorized. Returns the
/// canonical inclusion block number (see [`canonical_store_block`]). Waits for best-only
/// inclusion; callers accept the rare-reorg risk on later finalized-state reads.
pub async fn submit_store_signed(sdk: &TransactionClient, data: &[u8]) -> Result<u64> {
	tracing::info!("Submitting store ({} bytes)...", data.len());

	let receipt = tokio::time::timeout(
		transaction_timeout(WaitFor::InBlock),
		sdk.store(data.to_vec(), &dev::alice(), WaitFor::InBlock),
	)
	.await
	.map_err(|_| anyhow!("store transaction timed out"))??;

	let block_hash = receipt_block_hash(&receipt.block_hash)?;
	let block_number = canonical_store_block(sdk.api(), block_hash, &blake2_256(data)).await?;
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
pub async fn canonical_store_block<
	C: subxt::Config<Hasher: subxt::config::Hasher<Output = subxt::utils::H256>>,
>(
	client: &OnlineClient<C>,
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

/// Two `force_renew` calls signed by Alice and Bob respectively — synchronous immediate
/// renewals (each emits `Renewed`). `validate_signed` tags renewals with
/// `(who, content_hash)`, so two from the same signer would conflict in the pool.
pub async fn submit_renew_pair(
	sdk: &TransactionClient,
	block: u32,
	index: u32,
	content_hash: &[u8; 32],
) -> Result<(u64, u64)> {
	let (alice, bob) = (dev::alice(), dev::bob());
	tracing::info!(
		"Submitting two force_renew(block={}, index={}) calls (alice, bob)",
		block,
		index
	);

	let (receipt_alice, receipt_bob) =
		tokio::time::timeout(transaction_timeout(WaitFor::InBlock), async {
			futures::try_join!(
				sdk.force_renew((block, index), &alice, WaitFor::InBlock),
				sdk.force_renew((block, index), &bob, WaitFor::InBlock),
			)
		})
		.await
		.map_err(|_| anyhow!("force_renew pair timed out"))??;

	let hash_alice = receipt_block_hash(&receipt_alice.block_hash)?;
	let hash_bob = receipt_block_hash(&receipt_bob.block_hash)?;
	let block_alice = canonical_store_block(sdk.api(), hash_alice, content_hash).await?;
	let block_bob = canonical_store_block(sdk.api(), hash_bob, content_hash).await?;
	tracing::info!(
		"force_renew(block={}, idx={}) canonical inclusions: alice={}, bob={}",
		block,
		index,
		block_alice,
		block_bob,
	);
	Ok((block_alice, block_bob))
}

/// Signed one-shot `renew(ContentHash)` from Alice: schedules a single prepaid renewal
/// that fires at the `RetentionPeriod` boundary, then unregisters.
pub async fn submit_renew_one_shot(sdk: &TransactionClient, content_hash: &[u8; 32]) -> Result<()> {
	tracing::info!("Submitting one-shot renew...");

	tokio::time::timeout(
		transaction_timeout(WaitFor::InBlock),
		sdk.renew(*content_hash, &dev::alice(), WaitFor::InBlock),
	)
	.await
	.map_err(|_| anyhow!("renew transaction timed out"))??;

	tracing::info!("one-shot renew included in block");
	Ok(())
}

/// Signed `force_renew(ContentHash)` from Alice — synchronous renewal at dispatch. Returns
/// the inclusion block read from `TransactionByContentHash` at the best-block hash (see
/// [`canonical_store_block`]); callers re-anchor against finality for cycle math.
pub async fn submit_force_renew(sdk: &TransactionClient, content_hash: &[u8; 32]) -> Result<u64> {
	tracing::info!("Submitting force_renew...");

	let receipt = tokio::time::timeout(
		transaction_timeout(WaitFor::InBlock),
		sdk.force_renew(*content_hash, &dev::alice(), WaitFor::InBlock),
	)
	.await
	.map_err(|_| anyhow!("force_renew transaction timed out"))??;

	let block_hash = receipt_block_hash(&receipt.block_hash)?;
	let block_number = canonical_store_block(sdk.api(), block_hash, content_hash).await?;
	tracing::info!("force_renew included at block {}", block_number);
	Ok(block_number)
}

/// Signed `enable_auto_renew(content_hash)` from Alice.
pub async fn enable_auto_renew(sdk: &TransactionClient, content_hash: &[u8; 32]) -> Result<()> {
	tracing::info!("Submitting enable_auto_renew...");

	tokio::time::timeout(
		transaction_timeout(WaitFor::InBlock),
		sdk.enable_auto_renew(*content_hash, &dev::alice(), WaitFor::InBlock),
	)
	.await
	.map_err(|_| anyhow!("enable_auto_renew transaction timed out"))??;

	tracing::info!("enable_auto_renew included in block");
	Ok(())
}

/// Signed `disable_auto_renew(content_hash)` from Alice. Required at the end of
/// shared-harness tests to avoid renewals consuming Alice's authorization indefinitely.
pub async fn disable_auto_renew(sdk: &TransactionClient, content_hash: &[u8; 32]) -> Result<()> {
	tracing::info!("Submitting disable_auto_renew...");

	tokio::time::timeout(
		transaction_timeout(WaitFor::InBlock),
		sdk.disable_auto_renew(*content_hash, &dev::alice(), WaitFor::InBlock),
	)
	.await
	.map_err(|_| anyhow!("disable_auto_renew transaction timed out"))??;

	tracing::info!("disable_auto_renew included in block");
	Ok(())
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
