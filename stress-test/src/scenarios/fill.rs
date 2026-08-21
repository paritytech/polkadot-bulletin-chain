// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Saturating feeder: a fixed pool of reused accounts with sequential nonces
//! keeps the tx pool full so every sealed block goes out full. Runs until
//! killed. Built for dataset generation (manual-seal dev node), where the
//! one-shot-account pipeline leaves blocks mostly empty between batches.

use anyhow::Result;
use std::{
	sync::{
		atomic::{AtomicU64, Ordering},
		Arc,
	},
	time::{Duration, Instant},
};
use subxt::OnlineClient;
use subxt_signer::sr25519::Keypair;

use anyhow::anyhow;
use subxt::dynamic::{tx, Value};

use crate::{
	accounts::{keypair_at_derivation_prefix, NonceTracker},
	authorize::authorize_accounts,
	client::{fetch_txpool_pending_total, BulletinConfig, BulletinExtrinsicParamsBuilder},
	store::{generate_payload, store_submit_pre_signed},
};

/// Sign a store with an **immortal** era. The chain seals blocks far faster
/// than wall clock during a build, so the default 32-block mortal era pinned
/// to a long-lived client expires in seconds (`AncientBirthBlock`, silently
/// dropped by the pool).
fn sign_store_immortal(
	client: &OnlineClient<BulletinConfig>,
	signer: &Keypair,
	data: &[u8],
	nonce: u64,
) -> Result<Vec<u8>> {
	let store_call = tx("TransactionStorage", "store", vec![Value::from_bytes(data)]);
	let params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).immortal().build();
	let mut partial = client
		.tx()
		.create_partial_offline(&store_call, params)
		.map_err(|e| anyhow!("create_partial_offline: {e}"))?;
	Ok(partial.sign(signer).into_encoded())
}

/// Pause submission while the pool holds more than this many txs (a few
/// blocks worth at 512 KiB payloads keeps the proposer fed without hitting
/// pool byte limits). Oversubmitting is worse than idling: the fork-aware
/// pool silently drops the excess, which strands the nonces behind each
/// dropped tx, and signing burns CPU the proposer needs.
const POOL_HIGH_WATER: usize = 600;
/// How often each worker re-checks the pool depth, in submissions.
const POOL_CHECK_EVERY: u64 = 8;

/// Run the saturating feeder until the process is killed.
pub async fn run_fill(
	client: &OnlineClient<BulletinConfig>,
	authorizer: &Keypair,
	authorizer_nonces: &NonceTracker,
	ws_url: &str,
	payload_bytes: usize,
	num_accounts: u32,
	workers: u32,
) -> Result<()> {
	let prefix = "FILL";
	let keypairs: Vec<Keypair> =
		(0..num_accounts).map(|i| keypair_at_derivation_prefix(prefix, i)).collect();
	let account_ids: Vec<subxt::utils::AccountId32> =
		keypairs.iter().map(|k| k.public_key().to_account_id()).collect();

	// Idempotent on restart: re-authorizing adds allowance, which is harmless
	// (stores are soft-capped), and nonces are re-read from chain.
	tracing::info!("fill: authorizing {num_accounts} reusable accounts");
	authorize_accounts(
		client,
		authorizer,
		authorizer_nonces,
		&account_ids,
		1_000_000,
		100 * 1024 * 1024 * 1024,
	)
	.await?;

	let store_nonces = NonceTracker::new();
	for id in &account_ids {
		store_nonces.init_from_chain(client, id).await?;
	}

	let submitted = Arc::new(AtomicU64::new(0));
	let start = Instant::now();

	let mut tasks = Vec::new();
	for w in 0..workers {
		let my_keypairs: Vec<Keypair> =
			keypairs.iter().skip(w as usize).step_by(workers as usize).cloned().collect();
		let my_ids: Vec<subxt::utils::AccountId32> =
			my_keypairs.iter().map(|k| k.public_key().to_account_id()).collect();
		let client = client.clone();
		let nonces = store_nonces.clone();
		let ws_url = ws_url.to_string();
		let submitted = submitted.clone();

		tasks.push(tokio::spawn(async move {
			let rpc = jsonrpsee::ws_client::WsClientBuilder::default()
				.max_request_size(32 * 1024 * 1024)
				.build(&ws_url)
				.await?;
			let mut i = 0usize;
			let mut since_check = 0u64;
			loop {
				let kp = my_keypairs[i % my_keypairs.len()].clone();
				let id = my_ids[i % my_ids.len()].clone();
				i += 1;

				let nonce = nonces.next_nonce(&id);
				let c = client.clone();
				let encoded = tokio::task::spawn_blocking(move || {
					let payload = generate_payload(payload_bytes);
					sign_store_immortal(&c, &kp, &payload, nonce)
				})
				.await??;

				if let Err(e) = store_submit_pre_signed(&rpc, &encoded).await {
					// Pool full or transient RPC error: rollback and back off.
					nonces.rollback(&id);
					tracing::debug!("fill: submit failed ({e}), backing off");
					tokio::time::sleep(Duration::from_millis(200)).await;
					continue;
				}
				submitted.fetch_add(1, Ordering::Relaxed);

				since_check += 1;
				if since_check >= POOL_CHECK_EVERY {
					since_check = 0;
					// Err means the depth is unknown: back off rather than flood.
					while fetch_txpool_pending_total(&ws_url).await.unwrap_or(usize::MAX) >
						POOL_HIGH_WATER
					{
						tokio::time::sleep(Duration::from_millis(200)).await;
					}
				}
			}
			#[allow(unreachable_code)]
			Ok::<_, anyhow::Error>(())
		}));
	}

	// Progress line once a minute; the runner script tracks height and DB size.
	// Nonce re-sync from chain heals ladders broken by silent pool drops
	// (a dropped tx strands every higher nonce of that account as future).
	loop {
		tokio::time::sleep(Duration::from_secs(60)).await;
		for id in &account_ids {
			if let Err(e) = store_nonces.refresh(client, id).await {
				tracing::warn!("fill: nonce refresh failed for {id}: {e}");
			}
		}
		let n = submitted.load(Ordering::Relaxed);
		let mib = n as f64 * payload_bytes as f64 / 1024.0 / 1024.0;
		let secs = start.elapsed().as_secs_f64();
		tracing::info!(
			"fill: {n} stores submitted, ~{mib:.0} MiB payload, avg {:.1} MiB/s",
			mib / secs
		);
		if tasks.iter().all(|t| t.is_finished()) {
			anyhow::bail!("fill: all workers exited");
		}
	}
}
