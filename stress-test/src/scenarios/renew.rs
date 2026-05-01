//! Renew stress test: upload 512 items, then renew the same items every block.

use anyhow::Result;
use std::time::{Duration, Instant};
use subxt::{dynamic::Value, OnlineClient};
use subxt_signer::sr25519::Keypair;

use crate::{
	accounts::NonceTracker,
	client::{BulletinConfig, BulletinExtrinsicParamsBuilder},
	report::ScenarioResult,
	store,
};

#[allow(clippy::too_many_arguments)]
pub async fn run_renew_stress(
	client: &OnlineClient<BulletinConfig>,
	authorizer_signer: &Keypair,
	nonce_tracker: &NonceTracker,
	ws_urls: &[&str],
	chain_limits: &crate::chain_info::ChainLimits,
	num_store_txs: usize,
	chunk_size: usize,
	target_renew_blocks: u32,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
) -> Result<()> {
	let max_block_txs = chain_limits.max_block_transactions as usize;
	let num_store_txs = num_store_txs.max(max_block_txs);

	tracing::info!(
		"=== renew stress: store {num_store_txs} items × {}KB, then renew {max_block_txs}/block for {target_renew_blocks} blocks ===",
		chunk_size / 1024,
	);

	// Phase 1: Upload items.
	let signer =
		subxt_signer::sr25519::Keypair::from_secret_key(rand::random()).expect("valid keypair");
	let account_id = signer.public_key().to_account_id();
	let total_bytes = num_store_txs * chunk_size;

	// Authorize for stores + all renewals.
	let total_txs_needed = num_store_txs as u32 + max_block_txs as u32 * (target_renew_blocks + 5);
	let total_bytes_needed = (total_bytes as u64) +
		(chunk_size as u64 * max_block_txs as u64 * (target_renew_blocks as u64 + 5));
	crate::authorize::authorize_accounts(
		client,
		authorizer_signer,
		nonce_tracker,
		&[account_id.clone()],
		total_txs_needed,
		total_bytes_needed,
	)
	.await?;

	tracing::info!("Phase 1: uploading {num_store_txs} items...");
	let payloads: Vec<Vec<u8>> = (0..num_store_txs)
		.map(|i| store::generate_indexed_payload(chunk_size, i as u32))
		.collect();
	let ws_owned: Vec<String> = ws_urls.iter().map(|s| s.to_string()).collect();
	let upload =
		store::sequential_nonce_upload(client, &signer, payloads, ws_owned.clone(), chain_limits)
			.await?;
	tracing::info!("Phase 1 done: {}/{} confirmed", upload.total_confirmed, num_store_txs);

	if upload.total_confirmed == 0 {
		anyhow::bail!("No stores confirmed");
	}

	// Phase 1b: Find stored items.
	tracing::info!("Scanning for stored items...");
	let stored = discover_stored_items(client, max_block_txs).await?;
	tracing::info!("Found {} stored items to renew", stored.len());
	if stored.len() < max_block_txs {
		tracing::warn!("Only {} items found, need {max_block_txs} to fill a block", stored.len());
	}

	// Phase 2: Renew same items every block.
	tracing::info!(
		"Phase 2: renewing {} items/block for {target_renew_blocks} blocks...",
		stored.len().min(max_block_txs),
	);

	let mut best_sub = client.blocks().subscribe_best().await?;
	let rpc_client = jsonrpsee::ws_client::WsClientBuilder::default().build(&ws_owned[0]).await?;

	let start = Instant::now();
	let deadline = start + Duration::from_secs(600);
	let mut confirmed_blocks = 0u32;
	let mut total_renewed: u64 = 0;
	let mut best_nonce = {
		use jsonrpsee::core::client::ClientT;
		rpc_client
			.request::<u64, _>(
				"system_accountNextIndex",
				jsonrpsee::rpc_params![account_id.to_string()],
			)
			.await
			.unwrap_or(0)
	};
	let items_per_wave = stored.len().min(max_block_txs);

	loop {
		if Instant::now() > deadline || confirmed_blocks >= target_renew_blocks {
			break;
		}

		let Some(Ok(block)) = best_sub.next().await else { break };
		let block_number = block.number() as u64;
		let block_hash = block.hash();

		// Read nonce.
		{
			use jsonrpsee::core::client::ClientT;
			if let Ok(n) = rpc_client
				.request::<u64, _>(
					"system_accountNextIndex",
					jsonrpsee::rpc_params![account_id.to_string()],
				)
				.await
			{
				best_nonce = n;
			}
		}

		// Sign renew txs for the same items every block.
		let mut signed_txs = Vec::with_capacity(items_per_wave);
		for i in 0..items_per_wave {
			let (ref_block, ref_index) = &stored[i];
			let nonce = best_nonce + i as u64;
			let call = subxt::dynamic::tx(
				"TransactionStorage",
				"renew",
				vec![Value::u128(*ref_block as u128), Value::u128(*ref_index as u128)],
			);
			let mut params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).mortal(16).build();
			use subxt::config::transaction_extensions::Params;
			params.inject_block(block_number, block_hash);
			if let Ok(mut partial) = client.tx().create_partial_offline(&call, params) {
				let signed = partial.sign(&signer);
				signed_txs.push(store::PreSignedTx {
					nonce,
					tx_hash: subxt::utils::H256::zero(),
					encoded: signed.into_encoded(),
					payload_size: 0,
				});
			}
		}

		let (ok, errs) = store::submit_sequential_wave(&ws_owned, &signed_txs).await;
		tracing::info!(
			"Block #{block_number}: submitted {} renew txs (nonce {}..{}), {ok} ok, {} errors",
			signed_txs.len(),
			best_nonce,
			best_nonce + signed_txs.len() as u64 - 1,
			errs.len(),
		);

		// Wait for next block and check nonce + BlockWeight.
		if let Some(Ok(next_block)) = best_sub.next().await {
			let nb = next_block.number() as u64;
			let nh = next_block.hash();
			{
				use jsonrpsee::core::client::ClientT;
				if let Ok(n) = rpc_client
					.request::<u64, _>(
						"system_accountNextIndex",
						jsonrpsee::rpc_params![account_id.to_string()],
					)
					.await
				{
					let included = n.saturating_sub(best_nonce);
					if included > 0 {
						confirmed_blocks += 1;
						total_renewed += included;

						// Read BlockWeight.
						let bw_addr = subxt::dynamic::storage("System", "BlockWeight", ());
						let bw_str = client
							.storage()
							.at(nh)
							.fetch(&bw_addr)
							.await
							.ok()
							.flatten()
							.and_then(|v| v.to_value().ok().map(|v| format!("{v}")));

						tracing::info!(
							"Block #{nb}: {included} renewals included, \
							 total {total_renewed}, blocks {confirmed_blocks}/{target_renew_blocks}"
						);
						if let Some(bw) = bw_str {
							tracing::info!("  BlockWeight: {bw}");
						}
					}
					best_nonce = n;
				}
			}
		}
	}

	let duration = start.elapsed();
	let tps = total_renewed as f64 / duration.as_secs_f64();

	tracing::info!(
		"Renew stress done: {total_renewed} renewals in {confirmed_blocks} blocks, \
		 {tps:.1} tx/s, {:.1}s",
		duration.as_secs_f64(),
	);

	results.push(ScenarioResult {
		name: format!("Renew stress: {} items/block × {} blocks", items_per_wave, confirmed_blocks),
		duration,
		total_submitted: total_renewed,
		total_confirmed: total_renewed,
		total_errors: 0,
		throughput_tps: tps,
		avg_tx_per_block: if confirmed_blocks > 0 {
			total_renewed as f64 / confirmed_blocks as f64
		} else {
			0.0
		},
		peak_tx_per_block: items_per_wave as u64,
		..Default::default()
	});
	on_result(results);
	Ok(())
}

/// Find (block_number, index) pairs from TransactionStorage::Transactions.
async fn discover_stored_items(
	client: &OnlineClient<BulletinConfig>,
	needed: usize,
) -> Result<Vec<(u64, u32)>> {
	let fin_ref = client.backend().latest_finalized_block_ref().await?;
	let storage = client.storage().at(fin_ref.hash());
	let addr = subxt::dynamic::storage("TransactionStorage", "Transactions", ());
	let mut entries = storage.iter(addr).await?;
	let mut items = Vec::new();

	while let Some(Ok(entry)) = entries.next().await {
		let key_bytes = entry.key_bytes;
		let block_number = if key_bytes.len() >= 36 {
			let offset = key_bytes.len() - 4;
			u32::from_le_bytes(key_bytes[offset..].try_into().unwrap_or([0; 4])) as u64
		} else {
			continue
		};

		let encoded = entry.value.encoded();
		let count = match encoded.first() {
			Some(&b) if b & 0x03 == 0 => (b >> 2) as u32,
			Some(&b) if b & 0x03 == 1 && encoded.len() >= 2 =>
				(u16::from_le_bytes([encoded[0], encoded[1]]) >> 2) as u32,
			_ => 0,
		};
		for idx in 0..count {
			items.push((block_number, idx));
		}
		if items.len() >= needed {
			break
		}
	}
	Ok(items)
}
