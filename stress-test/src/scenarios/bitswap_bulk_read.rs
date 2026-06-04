//! Bulk Bitswap read scenario: discover CIDs from on-chain TransactionStorage,
//! then download them as fast as possible with configurable concurrency.

use anyhow::{anyhow, Result};
use std::{
	sync::{
		atomic::{AtomicBool, AtomicU64, Ordering},
		Arc,
	},
	time::{Duration, Instant},
};
use subxt::OnlineClient;

use crate::{
	bitswap::{self, BitswapClient},
	client::BulletinConfig,
	report::{compute_latency_stats, ScenarioResult},
};

/// Decode a block's `BoundedVec<TransactionInfo>` storage value into discovered
/// items. The layout mirrors the pallet's `TransactionInfo`
/// (`pallets/transaction-storage/src/types.rs`); fields are decoded in
/// declaration order so the cursor self-aligns regardless of field widths. On a
/// decode failure we log and stop rather than fabricate CIDs.
fn decode_transaction_infos(mut data: &[u8], block_number: u64) -> Vec<DiscoveredItem> {
	use subxt::ext::codec::{Compact, Decode};

	let input = &mut data;
	let count = match Compact::<u64>::decode(input) {
		Ok(c) => c.0 as usize,
		Err(e) => {
			tracing::warn!("block #{block_number}: bad TransactionInfo vec length: {e}");
			return Vec::new();
		},
	};

	let mut items = Vec::with_capacity(count);
	for i in 0..count {
		match decode_one_transaction_info(input, block_number) {
			Ok(Some(item)) => items.push(item),
			Ok(None) => {}, // decoded fine, but unsupported hashing — skip just this item
			Err(e) => {
				// Once one record fails to decode the rest are misaligned; stop.
				tracing::warn!("block #{block_number}: TransactionInfo #{i} decode failed: {e}");
				break;
			},
		}
	}
	items
}

/// Decode one `TransactionInfo`, consuming exactly its bytes from `input`.
/// Field order MUST match the pallet struct: `chunk_root[32] content_hash[32]
/// hashing(u8) cid_codec(u64) size(u32) extrinsic_index(u32) block_chunks(u32)
/// kind(u8)`.
fn decode_one_transaction_info(
	input: &mut &[u8],
	block_number: u64,
) -> Result<Option<DiscoveredItem>, subxt::ext::codec::Error> {
	use subxt::ext::codec::Decode;

	let chunk_root = <[u8; 32]>::decode(input)?;
	let content_hash = <[u8; 32]>::decode(input)?;
	let hashing_variant = u8::decode(input)?;
	let cid_codec = u64::decode(input)?;
	let size = u32::decode(input)?;
	let _extrinsic_index = u32::decode(input)?;
	let block_chunks = u32::decode(input)?;
	let _kind = u8::decode(input)?;

	let mh_code: u64 = match hashing_variant {
		0 => 0xb220, // Blake2b256
		1 => 0x12,   // Sha2_256
		2 => 0x1b,   // Keccak256
		other => {
			tracing::warn!("block #{block_number}: unknown hashing variant {other}, skipping item");
			return Ok(None);
		},
	};
	let mh = match cid::multihash::Multihash::<64>::wrap(mh_code, &content_hash) {
		Ok(mh) => mh,
		Err(e) => {
			tracing::warn!("block #{block_number}: invalid multihash: {e}");
			return Ok(None);
		},
	};
	let cid = cid::Cid::new_v1(cid_codec, mh);

	Ok(Some(DiscoveredItem {
		cid,
		size,
		chunk_root,
		content_hash,
		hashing_variant,
		cid_codec,
		block_chunks,
		block_number,
	}))
}

/// A discovered CID with its raw TransactionInfo fields for debugging.
#[derive(Clone)]
struct DiscoveredItem {
	cid: cid::Cid,
	size: u32,
	// Raw fields for diagnostics:
	chunk_root: [u8; 32],
	content_hash: [u8; 32],
	hashing_variant: u8,
	cid_codec: u64,
	block_chunks: u32,
	/// Block number where this item was stored.
	block_number: u64,
}

impl std::fmt::Display for DiscoveredItem {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"CID={}, size={}, block={}, hashing={}, codec=0x{:x}, \
			 content_hash=0x{}, chunk_root=0x{}, block_chunks={}",
			self.cid,
			self.size,
			self.block_number,
			match self.hashing_variant {
				0 => "Blake2b256",
				1 => "Sha2_256",
				2 => "Keccak256",
				_ => "Unknown",
			},
			self.cid_codec,
			hex::encode(self.content_hash),
			hex::encode(self.chunk_root),
			self.block_chunks,
		)
	}
}

/// Discover CIDs from on-chain `TransactionStorage::Transactions`,
/// filtering by size range. Stops once `target_bytes` of matching data found.
async fn discover_cids(
	client: &OnlineClient<BulletinConfig>,
	target_bytes: u64,
	min_size: u32,
	max_size: u32,
) -> Result<Vec<DiscoveredItem>> {
	let fin_ref = client.backend().latest_finalized_block_ref().await?;
	let header = client
		.backend()
		.block_header(fin_ref.hash())
		.await?
		.ok_or_else(|| anyhow!("cannot fetch finalized header"))?;
	let current_block: u64 = header.number.into();

	tracing::info!(
		"Discovering CIDs (size {}..{} bytes, target {} MB, block #{current_block})...",
		min_size,
		max_size,
		target_bytes / (1024 * 1024),
	);

	let storage = client.storage().at(fin_ref.hash());
	let mut items = Vec::new();
	let mut total_bytes: u64 = 0;
	let mut skipped = 0u64;

	let addr = subxt::dynamic::storage("TransactionStorage", "Transactions", ());
	let mut entries = storage.iter(addr).await?;
	let mut blocks_scanned = 0u64;

	while let Some(entry) = entries.next().await {
		let entry = match entry {
			Ok(e) => e,
			Err(e) => {
				tracing::debug!("Storage iteration error: {e}");
				continue;
			},
		};

		let key_bytes = entry.key_bytes;
		let block_number = if key_bytes.len() >= 36 {
			let offset = key_bytes.len() - 4;
			u32::from_le_bytes(key_bytes[offset..].try_into().unwrap_or([0; 4])) as u64
		} else {
			0
		};

		let encoded = entry.value.encoded();
		let parsed = decode_transaction_infos(encoded, block_number);

		for item in parsed {
			if item.size >= min_size && item.size <= max_size {
				total_bytes += item.size as u64;
				items.push(item);
			} else {
				skipped += 1;
			}
		}

		blocks_scanned += 1;
		if blocks_scanned.is_multiple_of(500) && !items.is_empty() {
			tracing::info!(
				"  ...scanned {blocks_scanned} blocks: {} matching CIDs ({} MB), {skipped} skipped",
				items.len(),
				total_bytes / (1024 * 1024),
			);
		}

		if total_bytes >= target_bytes {
			break;
		}
	}

	if items.is_empty() {
		anyhow::bail!(
			"No CIDs found matching size {}..{} bytes ({skipped} skipped)",
			min_size,
			max_size,
		);
	}

	tracing::info!(
		"Discovery: {} CIDs, {} MB ({blocks_scanned} blocks, {skipped} skipped by size)",
		items.len(),
		total_bytes / (1024 * 1024),
	);

	Ok(items)
}

/// Run bulk Bitswap read: discover CIDs from chain, then download with
/// specified concurrency.
#[allow(clippy::too_many_arguments)]
pub async fn run_bulk_read(
	client: &OnlineClient<BulletinConfig>,
	multiaddrs: &[litep2p::types::multiaddr::Multiaddr],
	target_bytes: u64,
	concurrency: usize,
	min_size: u32,
	max_size: u32,
	batch_size: usize,
	_ws_url: &str,
) -> Result<ScenarioResult> {
	let items = discover_cids(client, target_bytes, min_size, max_size).await?;
	let available_items = items.len();
	let available_bytes: u64 = items.iter().map(|i| i.size as u64).sum();

	tracing::info!(
		"Bulk read: {} items on chain ({} MB), target download: {} MB, \
		 concurrency={}, batch_size={}, peers={}",
		available_items,
		available_bytes / (1024 * 1024),
		target_bytes / (1024 * 1024),
		concurrency,
		batch_size,
		multiaddrs.len(),
	);

	// Create workers distributed across peers.
	// Each worker is a (client, peer_id) pair.
	let mut workers: Vec<(BitswapClient, litep2p::PeerId)> = Vec::with_capacity(concurrency);
	for i in 0..concurrency {
		let addr = &multiaddrs[i % multiaddrs.len()];
		let peer_id = BitswapClient::peer_id_from_multiaddr(addr)?;
		match bitswap::create_connected_client(addr).await {
			Ok(c) => {
				tracing::info!("Worker {i}: connected to peer {peer_id} ({})", addr);
				workers.push((c, peer_id));
			},
			Err(e) => tracing::warn!("Worker {i}: failed to connect to {addr}: {e}"),
		}
	}
	if workers.is_empty() {
		anyhow::bail!("No Bitswap clients connected");
	}
	let actual_concurrency = workers.len();
	tracing::info!("Bulk read: {actual_concurrency}/{concurrency} workers connected");

	let work = Arc::new(items);
	let next_idx = Arc::new(AtomicU64::new(0));
	let abort = Arc::new(AtomicBool::new(false));
	let bytes_downloaded = Arc::new(AtomicU64::new(0));
	let reads_ok = Arc::new(AtomicU64::new(0));
	let reads_failed = Arc::new(AtomicU64::new(0));
	let target = target_bytes;

	let wall_start = Instant::now();

	// Async progress logger — receives (item, data_len, elapsed) and
	// logs without blocking the download tasks.
	let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<(
		usize,          // first item index
		DiscoveredItem, // first item info
		usize,          // total bytes in batch
		usize,          // blocks in batch
		Duration,       // fetch elapsed
		u64,            // total downloaded so far
		u64,            // reads ok so far
	)>();
	let log_target = target;
	tokio::spawn(async move {
		while let Some((idx, item, batch_bytes, num_blocks, elapsed, downloaded, ok_count)) =
			log_rx.recv().await
		{
			let wall_secs = wall_start.elapsed().as_secs_f64().max(0.001);
			let speed_mb = downloaded as f64 / wall_secs / 1048576.0;
			let pct = (downloaded as f64 / log_target as f64 * 100.0).min(100.0);
			tracing::info!(
				"[{pct:5.1}%] read #{ok_count}: {idx} {} ({num_blocks} blocks, \
				 {:.1} MB, {:.0}ms) — {:.1} MB total, {speed_mb:.1} MB/s",
				item.cid,
				batch_bytes as f64 / 1048576.0,
				elapsed.as_secs_f64() * 1000.0,
				downloaded as f64 / 1048576.0,
			);
		}
	});

	// Spawn one task per worker — each pulls batches from the shared
	// queue until the download target is reached.
	let mut handles = Vec::with_capacity(actual_concurrency);
	for (client_idx, (client, peer_id)) in workers.into_iter().enumerate() {
		let work = Arc::clone(&work);
		let next_idx = Arc::clone(&next_idx);
		let abort = Arc::clone(&abort);
		let bytes_downloaded = Arc::clone(&bytes_downloaded);
		let reads_ok = Arc::clone(&reads_ok);
		let reads_failed = Arc::clone(&reads_failed);
		let log_tx = log_tx.clone();

		handles.push(tokio::spawn(async move {
			let mut timings: Vec<(Duration, bool)> = Vec::new();
			let mut consecutive_failures = 0u32;

			loop {
				if abort.load(Ordering::Relaxed) {
					break;
				}
				// Stop once global target is reached.
				if bytes_downloaded.load(Ordering::Relaxed) >= target {
					break;
				}

				// Check how much is left to download.
				let downloaded_so_far = bytes_downloaded.load(Ordering::Relaxed);
				if downloaded_so_far >= target {
					break;
				}

				// Grab a batch of items round-robin.
				let start_raw = next_idx.fetch_add(batch_size as u64, Ordering::Relaxed) as usize;
				let batch_items: Vec<_> = (0..batch_size)
					.map(|i| {
						let idx = (start_raw + i) % work.len();
						(idx, &work[idx])
					})
					.collect();
				let cids: Vec<cid::Cid> = batch_items.iter().map(|(_, item)| item.cid).collect();

				let start = Instant::now();
				match client.fetch_blocks(peer_id, &cids, Duration::from_secs(30)).await {
					Ok(blocks) => {
						let elapsed = start.elapsed();
						let batch_bytes: usize = blocks.iter().map(|b| b.len()).sum();
						let downloaded = bytes_downloaded
							.fetch_add(batch_bytes as u64, Ordering::Relaxed) +
							batch_bytes as u64;
						let ok_count = reads_ok.fetch_add(blocks.len() as u64, Ordering::Relaxed) +
							blocks.len() as u64;
						for _ in 0..blocks.len() {
							timings.push((elapsed / blocks.len() as u32, true));
						}
						consecutive_failures = 0;

						let (idx, item) = &batch_items[0];
						let _ = log_tx.send((
							*idx,
							(*item).clone(),
							batch_bytes,
							blocks.len(),
							elapsed,
							downloaded,
							ok_count,
						));
					},
					Err(e) => {
						let elapsed = start.elapsed();
						reads_failed.fetch_add(batch_items.len() as u64, Ordering::Relaxed);
						let (idx, item) = &batch_items[0];
						tracing::warn!(
							"Client {client_idx}: batch FAILED ({} CIDs, item {idx}, \
							 {:.0}ms): {e}\n  first item: {item}",
							batch_items.len(),
							elapsed.as_secs_f64() * 1000.0,
						);
						for _ in 0..batch_items.len() {
							timings.push((elapsed, false));
						}
						consecutive_failures += 1;
						if consecutive_failures >= 10 {
							tracing::warn!(
								"Client {client_idx}: {consecutive_failures} consecutive \
								 failures, this worker stopping"
							);
							break; // only this worker stops, not all
						}
					},
				}
			}
			timings
		}));
	}

	// Drop our sender so the logger task exits after all clients finish.
	drop(log_tx);

	// Collect results.
	let mut all_durations = Vec::new();
	let mut successful = 0u64;
	let mut failed = 0u64;

	for handle in handles {
		let timings = handle.await.map_err(|e| anyhow!("task panicked: {e}"))?;
		for (dur, ok) in timings {
			if ok {
				successful += 1;
				all_durations.push(dur);
			} else {
				failed += 1;
			}
		}
	}

	let wall_time = wall_start.elapsed();
	let downloaded = bytes_downloaded.load(Ordering::Relaxed);
	let total_reads = successful + failed;
	let reads_per_sec = successful as f64 / wall_time.as_secs_f64();
	let bytes_per_sec = downloaded as f64 / wall_time.as_secs_f64();

	tracing::info!(
		"Bulk read: {successful}/{total_reads} reads OK, \
		 {reads_per_sec:.1} reads/s, {:.1} MB/s, wall={:.1}s, \
		 downloaded {} MB",
		bytes_per_sec / 1048576.0,
		wall_time.as_secs_f64(),
		downloaded / (1024 * 1024),
	);

	Ok(ScenarioResult {
		name: format!(
			"Bulk Bitswap Read ({} unique CIDs, {} MB downloaded, concurrency={})",
			available_items,
			downloaded / (1024 * 1024),
			concurrency,
		),
		duration: wall_time,
		payload_size: downloaded as usize,
		retrieval_latency: compute_latency_stats(&mut all_durations),
		total_reads: Some(total_reads),
		successful_reads: Some(successful),
		failed_reads: Some(failed),
		reads_per_sec: Some(reads_per_sec),
		read_bytes_per_sec: Some(bytes_per_sec),
		data_verified: Some(true),
		..Default::default()
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use subxt::ext::codec::{Compact, Encode};

	/// Append one SCALE-encoded `TransactionInfo` matching the pallet layout.
	#[allow(clippy::too_many_arguments)]
	fn push_tx_info(
		buf: &mut Vec<u8>,
		chunk_root: [u8; 32],
		content_hash: [u8; 32],
		hashing: u8,
		cid_codec: u64,
		size: u32,
		extrinsic_index: u32,
		block_chunks: u32,
		kind: u8,
	) {
		buf.extend_from_slice(&chunk_root);
		buf.extend_from_slice(&content_hash);
		buf.push(hashing);
		cid_codec.encode_to(buf);
		size.encode_to(buf);
		extrinsic_index.encode_to(buf);
		block_chunks.encode_to(buf);
		buf.push(kind);
	}

	#[test]
	fn decodes_multiple_transaction_infos_without_drift() {
		// A BoundedVec<TransactionInfo> with 3 records. The old fixed-81-byte
		// parser misaligned every record after the first (it omitted
		// `extrinsic_index` and `kind`, making the real stride 86 bytes).
		let mut bytes = Vec::new();
		Compact(3u32).encode_to(&mut bytes);
		push_tx_info(&mut bytes, [0xAA; 32], [0x11; 32], 0, 0x55, 1024, 0, 1, 0);
		push_tx_info(&mut bytes, [0xBB; 32], [0x22; 32], 1, 0x70, 2_097_152, 7, 2, 1);
		push_tx_info(&mut bytes, [0xCC; 32], [0x33; 32], 2, 0x55, 4096, 3, 5, 0);

		let items = decode_transaction_infos(&bytes, 42);
		assert_eq!(items.len(), 3, "all three records must decode");

		// Record 0 — `block_chunks` is the real value (1), not extrinsic_index (0).
		assert_eq!(items[0].content_hash, [0x11; 32]);
		assert_eq!(items[0].cid_codec, 0x55);
		assert_eq!(items[0].block_chunks, 1);
		// Records 1 and 2 must be correct too — proves no per-record drift.
		assert_eq!(items[1].content_hash, [0x22; 32]);
		assert_eq!(items[1].cid_codec, 0x70);
		assert_eq!(items[1].hashing_variant, 1);
		assert_eq!(items[1].size, 2_097_152);
		assert_eq!(items[1].block_chunks, 2); // not 7 (extrinsic_index)
		assert_eq!(items[2].content_hash, [0x33; 32]);
		assert_eq!(items[2].cid_codec, 0x55);
		assert_eq!(items[2].block_chunks, 5);
	}

	#[test]
	fn skips_unknown_hashing_variant() {
		let mut bytes = Vec::new();
		Compact(1u32).encode_to(&mut bytes);
		push_tx_info(&mut bytes, [0; 32], [0; 32], 9, 0x55, 1, 0, 1, 0); // hashing=9 unknown
		let items = decode_transaction_infos(&bytes, 1);
		assert!(items.is_empty(), "unknown hashing variant must be skipped, not fabricated");
	}
}
