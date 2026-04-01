use anyhow::Result;
use std::{
	sync::Arc,
	time::{Duration, Instant},
};
use subxt::OnlineClient;
use subxt_signer::sr25519::Keypair;

use crate::{
	accounts::NonceTracker,
	bitswap::{self, BitswapClient},
	client::BulletinConfig,
	report::{compute_latency_stats, ScenarioResult},
	store,
};

/// B2 payload size: 128KB
const B2_PAYLOAD_SIZE: usize = 128 * 1024;

/// Concurrency levels to sweep.
const B2_CONCURRENCY_LEVELS: &[usize] = &[1, 2, 4, 8, 16, 32, 64];

/// Run B2 at a single concurrency level: N independent BitswapClients each
/// reading all items in parallel.
async fn run_b2_concurrent_read_level(
	multiaddr: &litep2p::types::multiaddr::Multiaddr,
	items: Arc<Vec<(cid::Cid, Vec<u8>)>>,
	payload_size: usize,
	concurrency: usize,
) -> Result<ScenarioResult> {
	let peer_id = BitswapClient::peer_id_from_multiaddr(multiaddr)?;

	// Create `concurrency` independent clients
	let mut clients = Vec::with_capacity(concurrency);
	for i in 0..concurrency {
		match bitswap::create_connected_client(multiaddr).await {
			Ok(c) => clients.push(c),
			Err(e) => log::warn!("B2: failed to create client {i}: {e}"),
		}
	}
	if clients.is_empty() {
		anyhow::bail!("B2: no clients connected (concurrency={concurrency})");
	}
	let actual_concurrency = clients.len();
	log::info!(
		"B2: {actual_concurrency}/{concurrency} clients connected, reading {} items each",
		items.len()
	);

	let wall_start = Instant::now();

	// Spawn one task per client — each reads ALL items sequentially
	let mut handles = Vec::with_capacity(actual_concurrency);
	for (idx, client) in clients.into_iter().enumerate() {
		let items = Arc::clone(&items);
		handles.push(tokio::spawn(async move {
			let mut timings: Vec<(Duration, bool, bool)> = Vec::with_capacity(items.len());
			for (cid, expected) in items.iter() {
				let start = Instant::now();
				match client.fetch_block(peer_id, *cid, Duration::from_secs(30)).await {
					Ok(data) => {
						let elapsed = start.elapsed();
						let verified = data == *expected;
						if !verified {
							log::warn!(
								"B2 client-{idx}: data mismatch (got {} bytes, expected {})",
								data.len(),
								expected.len()
							);
						}
						timings.push((elapsed, true, verified));
					},
					Err(e) => {
						let elapsed = start.elapsed();
						log::warn!("B2 client-{idx}: fetch failed: {e}");
						timings.push((elapsed, false, false));
					},
				}
			}
			timings
		}));
	}

	// Collect results from all tasks
	let mut all_durations = Vec::new();
	let mut successful = 0u64;
	let mut failed = 0u64;
	let mut all_verified = true;

	for handle in handles {
		let timings = handle.await.map_err(|e| anyhow::anyhow!("B2 task panicked: {e}"))?;
		for (dur, ok, verified) in timings {
			if ok {
				successful += 1;
				all_durations.push(dur);
			} else {
				failed += 1;
			}
			if !verified {
				all_verified = false;
			}
		}
	}

	let wall_time = wall_start.elapsed();
	let total_reads = successful + failed;
	let reads_per_sec = successful as f64 / wall_time.as_secs_f64();
	let read_bytes_per_sec = (successful * payload_size as u64) as f64 / wall_time.as_secs_f64();

	log::info!(
		"B2: concurrency={concurrency} — {successful}/{total_reads} reads OK, \
		 {reads_per_sec:.1} reads/s, wall={:.1}s",
		wall_time.as_secs_f64()
	);

	Ok(ScenarioResult {
		name: format!("B2: Concurrent Read (128KB, concurrency={concurrency})"),
		duration: wall_time,
		payload_size,
		retrieval_latency: compute_latency_stats(&mut all_durations),
		total_reads: Some(total_reads),
		successful_reads: Some(successful),
		failed_reads: Some(failed),
		reads_per_sec: Some(reads_per_sec),
		read_bytes_per_sec: Some(read_bytes_per_sec),
		data_verified: Some(all_verified),
		..Default::default()
	})
}

/// B2 sweep: store items once, then read at increasing concurrency levels.
#[allow(clippy::too_many_arguments)]
pub async fn run_b2_concurrent_read_sweep(
	client: &OnlineClient<BulletinConfig>,
	authorizer_signer: &Keypair,
	nonce_tracker: &NonceTracker,
	multiaddr: &litep2p::types::multiaddr::Multiaddr,
	item_count: u32,
	ws_url: &str,
) -> Result<Vec<ScenarioResult>> {
	log::info!(
		"B2: Concurrent read sweep ({item_count} items, {}KB payload)",
		B2_PAYLOAD_SIZE / 1024
	);

	// --- Generate unique payloads and compute CIDs ---
	let mut items: Vec<(cid::Cid, Vec<u8>)> = Vec::with_capacity(item_count as usize);
	let mut work_items: Vec<(Keypair, Arc<Vec<u8>>)> = Vec::with_capacity(item_count as usize);

	let seed = format!("B2read_{B2_PAYLOAD_SIZE}");
	let keypairs = crate::accounts::generate_keypairs(item_count, &seed);
	let account_ids: Vec<_> = keypairs.iter().map(|kp| kp.public_key().to_account_id()).collect();

	for (i, kp) in keypairs.into_iter().enumerate() {
		let data = store::generate_indexed_payload(B2_PAYLOAD_SIZE, i as u32);
		let cid = store::compute_cid_blake2b256(&data)?;
		items.push((cid, data.clone()));
		work_items.push((kp, Arc::new(data)));
	}

	// Authorize all one-shot accounts
	crate::authorize::authorize_accounts(
		client,
		authorizer_signer,
		nonce_tracker,
		&account_ids,
		1,
		(B2_PAYLOAD_SIZE + 1024) as u64,
	)
	.await?;

	// --- Store phase ---
	log::info!("B2: storing {item_count} items via bulk_store_oneshot...");
	let blocks_rx = store::subscribe_blocks(ws_url).await?;
	let store_result = store::bulk_store_oneshot(
		work_items,
		&[ws_url],
		None,
		4,
		store::BlockInput::BestOnly(blocks_rx),
	)
	.await?;
	log::info!(
		"B2: store complete — {}/{} confirmed in {:.1}s",
		store_result.total_confirmed,
		store_result.total_submitted,
		store_result.duration.as_secs_f64()
	);

	if store_result.total_confirmed == 0 {
		anyhow::bail!("B2: no items stored, cannot proceed with read phase");
	}

	// --- Sweep concurrency levels ---
	let mut results = Vec::new();
	let items = Arc::new(items);

	for &concurrency in B2_CONCURRENCY_LEVELS {
		log::info!("=== B2 sweep: concurrency={concurrency} ===");
		match run_b2_concurrent_read_level(
			multiaddr,
			Arc::clone(&items),
			B2_PAYLOAD_SIZE,
			concurrency,
		)
		.await
		{
			Ok(result) => results.push(result),
			Err(e) => {
				log::warn!("B2 sweep: concurrency={concurrency} failed: {e}");
				results.push(ScenarioResult {
					name: format!(
						"B2: Concurrent Read (128KB, concurrency={concurrency} - FAILED)"
					),
					payload_size: B2_PAYLOAD_SIZE,
					total_reads: Some(0),
					successful_reads: Some(0),
					failed_reads: Some((item_count as u64) * (concurrency as u64)),
					reads_per_sec: Some(0.0),
					read_bytes_per_sec: Some(0.0),
					data_verified: Some(false),
					..Default::default()
				});
			},
		}

		// Brief pause between levels for connection cleanup
		tokio::time::sleep(Duration::from_secs(2)).await;
	}

	Ok(results)
}
