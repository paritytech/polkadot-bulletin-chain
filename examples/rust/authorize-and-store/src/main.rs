// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Authorize and store data on Bulletin Chain using the Bulletin SDK.
//!
//! This example demonstrates:
//! 1. Using the Bulletin SDK's TransactionClient for all chain interactions
//! 2. Authorizing an account to store data (requires sudo)
//! 3. Storing small data with CID verification
//! 4. Storing large data with DAG-PB manifest (chunked upload)
//!
//! ## Usage
//!
//!   cargo run --release -- --ws ws://localhost:10000 --seed "//Eve"

use anyhow::{anyhow, Result};
use bulletin_sdk_rust::prelude::*;
use clap::Parser;
use std::{str::FromStr, sync::Arc};
use subxt::utils::AccountId32;
use subxt_signer::sr25519::Keypair;
use tracing::info;
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(name = "authorize-and-store")]
#[command(about = "Authorize and store data on Bulletin Chain using the Bulletin SDK")]
struct Args {
	/// WebSocket URL of the Bulletin Chain node
	#[arg(long, default_value = "ws://localhost:10000")]
	ws: String,

	/// Seed phrase or dev seed (e.g., "//Eve" or mnemonic)
	#[arg(long, default_value = "//Eve")]
	seed: String,
}

#[tokio::main]
async fn main() -> Result<()> {
	// Initialize tracing subscriber
	let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
		.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
	let subscriber = FmtSubscriber::builder().with_env_filter(env_filter).finish();
	tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");

	let args = Args::parse();

	// Parse keypair from seed
	let keypair = keypair_from_seed(&args.seed)?;
	let account_id = AccountId32::from(keypair.public_key().0);
	info!("Using account: {}", account_id);

	// Step 1: Connect using SDK's TransactionClient
	info!("Connecting to {} using Bulletin SDK...", args.ws);
	let client = TransactionClient::new(&args.ws)
		.await
		.map_err(|e| anyhow!("Failed to connect: {:?}", e))?;
	info!("Connected successfully!");

	// Step 2: Authorize the account to store data
	info!("\nStep 1: Authorizing account using SDK...");

	let auth_receipt = client
		.authorize_account(account_id.clone(), 100, 100 * 1024 * 1024, &keypair, WaitFor::Finalized)
		.await
		.map_err(|e| anyhow!("Authorization failed: {:?}", e))?;

	info!("Account authorized successfully!");
	info!("  Block hash: {}", auth_receipt.block_hash);
	info!("  Transactions: {}", auth_receipt.transactions);
	info!("  Bytes: {}", auth_receipt.bytes);

	// Step 3: Prepare and store data using SDK
	info!("\nStep 2: Storing data using SDK...");
	let data_to_store = format!("Hello from Bulletin SDK at {}", chrono_lite());
	info!("Data: {}", data_to_store);

	// Calculate CID before submission using SDK utilities
	let sdk_client = BulletinClient::new();
	let options = StoreOptions {
		cid_codec: CidCodec::Raw,
		hash_algorithm: HashingAlgorithm::Blake2b256,
		wait_for: WaitFor::InBlock,
	};

	let operation = sdk_client
		.prepare_store(data_to_store.as_bytes().to_vec(), options)
		.map_err(|e| anyhow!("SDK error: {:?}", e))?;

	let cid_data = operation
		.calculate_cid()
		.map_err(|e| anyhow!("CID calculation error: {:?}", e))?;
	let cid_bytes =
		cid_to_bytes(&cid_data).map_err(|e| anyhow!("CID serialization error: {:?}", e))?;
	info!("Pre-calculated CID: {}", hex::encode(&cid_bytes));
	info!("Content hash: {}", hex::encode(&cid_data.content_hash));

	// Store using SDK's TransactionClient with progress callback
	let store_receipt = client
		.store_with_progress(
			data_to_store.as_bytes().to_vec(),
			&keypair,
			WaitFor::InBlock,
			Some(std::sync::Arc::new(|event| {
				info!("Progress: {:?}", event);
			})),
		)
		.await
		.map_err(|e| anyhow!("Store failed: {:?}", e))?;

	info!("\n✅ Data stored successfully using Bulletin SDK!");
	info!("  Block hash: {}", store_receipt.block_hash);
	info!("  Extrinsic hash: {}", store_receipt.extrinsic_hash);
	info!("  Data size: {} bytes", store_receipt.data_size);

	// Step 4: Demonstrate chunked storage with DAG-PB manifest
	info!("\n--- Step 3: Chunked Storage with DAG-PB Manifest ---");

	// Create larger data that will be chunked (3 MiB)
	let large_data_size = 3 * 1024 * 1024; // 3 MiB
	let large_data: Vec<u8> = (0..large_data_size).map(|i| (i % 256) as u8).collect();
	info!("Large data size: {} bytes ({} MiB)", large_data.len(), large_data.len() / 1024 / 1024);

	// Plan the upload offline: chunk boundaries, per-chunk CIDs, the DAG-PB
	// manifest, and the authorization cost.
	let chunker_config = ChunkerConfig {
		chunk_size: 1024 * 1024, // 1 MiB chunks
		max_parallel: 4,
		create_manifest: true, // Create DAG-PB manifest
	};
	let source: Arc<dyn SeekableSource> = Arc::new(blob_from_bytes(large_data));
	let estimate = client
		.estimate_upload(
			UploadInput::Source(source.clone()),
			UploadEstimateOptions { chunker: chunker_config, ..Default::default() },
		)
		.await
		.map_err(|e| anyhow!("Estimate failed: {:?}", e))?;
	info!(
		"Planned {} chunks + manifest: {} transactions, {} bytes of authorization",
		estimate.plan.chunk_cids.len(),
		estimate.base.transactions,
		estimate.base.bytes,
	);

	// Submit through the pipeline. A hand-rolled per-chunk store() loop is
	// exposed to nonce reuse under fork churn ("usurped"); the pipeline owns
	// nonce assignment, retries, and reorg recovery.
	let upload_config = UploadConfig {
		complete_on: WaitFor::InBlock,
		on_event: Some(Arc::new(|ev: UploadEvent| match ev {
			UploadEvent::ItemInBlock { index, total, block_hash, .. } => {
				info!("  item {}/{} in block: {}", index + 1, total, block_hash);
			},
			UploadEvent::ItemFailed { index, total, error, .. } => {
				info!("  item {}/{} FAILED: {}", index + 1, total, error);
			},
			_ => {},
		})),
		..Default::default()
	};
	let result = client
		.submit(&keypair, estimate, source, upload_config)
		.await
		.map_err(|e| anyhow!("Chunked upload failed: {:?}", e))?;

	// CIDs are returned in plan order; the manifest root is last.
	let root_cid = result.cids.last().expect("manifest root CID");
	info!("✅ Chunked upload complete: {} stores (chunks + manifest)", result.cids.len());
	info!("  Manifest root CID: {}", hex::encode(root_cid));
	info!("  Use this manifest CID to retrieve the complete file via IPFS/Bitswap");

	info!("\n✅ All examples completed successfully!");

	Ok(())
}

fn keypair_from_seed(seed: &str) -> Result<Keypair> {
	if seed.starts_with("//") {
		let uri = subxt_signer::SecretUri::from_str(seed)
			.map_err(|e| anyhow!("Failed to parse secret URI: {e}"))?;
		let keypair =
			Keypair::from_uri(&uri).map_err(|e| anyhow!("Failed to create keypair: {e}"))?;
		Ok(keypair)
	} else {
		let mnemonic = subxt_signer::bip39::Mnemonic::from_str(seed)
			.map_err(|e| anyhow!("Failed to parse mnemonic: {e}"))?;
		let keypair = Keypair::from_phrase(&mnemonic, None)
			.map_err(|e| anyhow!("Failed to create keypair from mnemonic: {e}"))?;
		Ok(keypair)
	}
}

fn chrono_lite() -> String {
	use std::time::{SystemTime, UNIX_EPOCH};
	let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
	format!("{}s", duration.as_secs())
}
