// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Chunked store example - Store large files with automatic chunking
//!
//! This example demonstrates:
//! - Storing large files (> 8 MiB) with automatic chunking
//! - Progress tracking during upload
//! - DAG-PB manifest creation for IPFS compatibility
//! - Retrieving chunk and manifest CIDs
//!
//! Usage:
//!   cargo run --example chunked_store --features std [file_path]
//!   cargo run --example chunked_store --features std large_video.mp4

use bulletin_sdk_rust::{
	async_client::{AsyncBulletinClient, AsyncClientConfig},
	prelude::*,
	submit::TransactionSubmitter,
};
use std::{env, fs};

// Include the SubxtSubmitter from simple_store example
// In a real application, this would be in a shared module
include!("simple_store.rs");

#[tokio::main]
async fn main() -> Result<()> {
	println!("🚀 Bulletin SDK - Chunked Store Example\n");

	// 1. Get file path from command line
	let args: Vec<String> = env::args().collect();
	if args.len() < 2 {
		eprintln!("Usage: {} <file_path>", args[0]);
		eprintln!("Example: {} large_video.mp4", args[0]);
		return Ok(());
	}

	let file_path = &args[1];
	println!("📁 Reading file: {}", file_path);

	// 2. Read file data
	let data = fs::read(file_path)
		.map_err(|e| Error::StorageFailed(format!("Failed to read file: {:?}", e)))?;

	println!("📊 File size: {} bytes ({:.2} MB)\n", data.len(), data.len() as f64 / 1_048_576.0);

	// 3. Setup connection
	println!("📡 Connecting to Bulletin Chain at ws://localhost:9944...");
	let endpoint = "ws://localhost:9944";

	let signer = PairSigner::new(Pair::from_string("//Alice", None).expect("Valid dev seed"));
	let submitter = SubxtSubmitter::new(endpoint, signer).await?;
	println!("✅ Connected to Bulletin Chain\n");

	// 4. Create client with custom config
	let config = AsyncClientConfig {
		default_chunk_size: 1024 * 1024, // 1 MiB chunks
		max_parallel: 8,
		create_manifest: true,
	};
	let client = AsyncBulletinClient::with_config(submitter, config);

	// 5. Estimate authorization needed
	let (est_txs, est_bytes) = client.estimate_authorization(data.len() as u64);
	println!("📋 Authorization estimate:");
	println!("   Transactions needed: {}", est_txs);
	println!("   Total bytes: {} ({:.2} MB)\n", est_bytes, est_bytes as f64 / 1_048_576.0);

	// 6. Store with progress tracking
	println!("⏳ Uploading with chunking and manifest creation...\n");

	let mut chunks_completed = 0;
	let mut total_chunks = 0;

	let result = client
		.store_chunked(
			&data,
			None, // use default config
			StoreOptions::default(),
			Some(|event| match event {
				ProgressEvent::ChunkStarted { index, total } =>
					if total_chunks == 0 {
						total_chunks = total;
						println!("🔨 Starting upload of {} chunks...", total);
					},
				ProgressEvent::ChunkCompleted { index, total, cid } => {
					chunks_completed += 1;
					let progress = (chunks_completed as f32 / total as f32) * 100.0;
					println!(
						"   ✅ Chunk {}/{} completed ({:.1}%) - {} bytes",
						chunks_completed,
						total,
						progress,
						cid.len()
					);
				},
				ProgressEvent::ChunkFailed { index, total, error } => {
					println!("   ❌ Chunk {}/{} failed: {}", index + 1, total, error);
				},
				ProgressEvent::ManifestStarted => {
					println!("\n📦 Creating DAG-PB manifest...");
				},
				ProgressEvent::ManifestCreated { cid } => {
					println!("   ✅ Manifest created: {} bytes\n", cid.len());
				},
				ProgressEvent::Completed { manifest_cid } => {
					println!("🎉 Upload complete!");
					if manifest_cid.is_some() {
						println!("   Manifest included\n");
					}
				},
			}),
		)
		.await?;

	// 7. Display results
	println!("📊 Final Results:");
	println!("   Total chunks: {}", result.num_chunks);
	println!("   Total size: {} bytes", result.total_size);
	println!("   Chunk CIDs: {} CIDs stored", result.chunk_cids.len());

	if let Some(manifest_cid) = &result.manifest_cid {
		println!("\n📦 DAG-PB Manifest:");
		println!("   CID bytes: {}", manifest_cid.len());
		#[cfg(feature = "std")]
		if let Ok(cid) = crate::cid::cid_from_bytes(manifest_cid) {
			println!("   CID (base32): {}", crate::cid::cid_to_string(&cid));
			println!("\n💡 You can retrieve this file via IPFS using:");
			println!("   ipfs cat {}", crate::cid::cid_to_string(&cid));
		}
	}

	println!("\n🎉 Chunked upload completed successfully!");
	println!("\n💡 Next steps:");
	println!("   - Use the manifest CID to retrieve the full file");
	println!("   - Access via IPFS gateway");
	println!("   - Individual chunks are also stored and accessible");

	Ok(())
}
