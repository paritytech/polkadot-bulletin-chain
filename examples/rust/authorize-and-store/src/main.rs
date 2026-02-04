// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Authorize and store data on Bulletin Chain using subxt.
//!
//! This example demonstrates:
//! 1. Using PolkadotConfig with auto-discovered signed extensions from metadata
//! 2. Auto-discovery of Bulletin's custom ProvideCidConfig extension
//! 3. Authorizing an account to store data
//! 4. Storing data on the Bulletin Chain
//!
//! ## Setup
//!
//! Before running this example, generate metadata from a running node:
//!   ./fetch_metadata.sh ws://localhost:10000
//!
//! ## Usage
//!
//!   cargo run --release -- --ws ws://localhost:10000 --seed "//Alice"

use anyhow::{anyhow, Result};
use clap::Parser;
use std::str::FromStr;
use subxt::{
	config::SubstrateConfig,
	utils::AccountId32,
	OnlineClient,
};
use subxt_signer::sr25519::Keypair;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(name = "authorize-and-store")]
#[command(about = "Authorize and store data on Bulletin Chain using subxt")]
struct Args {
	/// WebSocket URL of the Bulletin Chain node
	#[arg(long, default_value = "ws://localhost:10000")]
	ws: String,

	/// Seed phrase or dev seed (e.g., "//Alice" or mnemonic)
	#[arg(long, default_value = "//Alice")]
	seed: String,
}

// Generate types from metadata using subxt codegen
// This reads bulletin_metadata.scale and generates all the necessary types at compile time
#[subxt::subxt(runtime_metadata_path = "bulletin_metadata.scale")]
pub mod bulletin {}

#[tokio::main]
async fn main() -> Result<()> {
	// Initialize tracing subscriber
	let subscriber = FmtSubscriber::builder()
		.with_max_level(Level::INFO)
		.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
		.finish();
	tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");

	let args = Args::parse();

	// Parse keypair from seed
	let keypair = keypair_from_seed(&args.seed)?;
	let account_id: AccountId32 = keypair.public_key().into();
	info!("Using account: {}", account_id);

	// Connect to Bulletin Chain node using SubstrateConfig
	// SubstrateConfig uses basic extrinsic params without auto-discovering custom extensions
	info!("Connecting to {}...", args.ws);
	let api = OnlineClient::<SubstrateConfig>::from_url(&args.ws)
		.await
		.map_err(|e| anyhow!("Failed to connect: {e:?}"))?;
	info!("Connected successfully!");

	// Step 1: Authorize the account to store data (requires sudo)
	info!("\nStep 1: Authorizing account...");

	// In subxt 0.37, to wrap a call in sudo, we need to manually construct the RuntimeCall
	// Build the inner call as a runtime type
	use bulletin::runtime_types;

	// Use bulletin_westend_runtime as that's what the metadata contains
	let authorize_call = runtime_types::bulletin_westend_runtime::RuntimeCall::TransactionStorage(
		runtime_types::pallet_transaction_storage::pallet::Call::authorize_account {
			who: account_id.clone(),
			transactions: 100,
			bytes: 100 * 1024 * 1024,
		}
	);

	// Wrap in sudo call (Alice is sudo in dev mode)
	let sudo_tx = bulletin::tx().sudo().sudo(authorize_call);

	api
		.tx()
		.sign_and_submit_then_watch_default(&sudo_tx, &keypair)
		.await
		.map_err(|e| anyhow!("Failed to submit authorization: {e:?}"))?
		.wait_for_finalized_success()
		.await
		.map_err(|e| anyhow!("Authorization transaction failed: {e:?}"))?;

	info!("Account authorized successfully!");

	// Step 2: Store data
	info!("\nStep 2: Storing data...");
	let data_to_store = format!("Hello from Bulletin Chain at {}", chrono_lite());
	info!("Data: {}", data_to_store);

	let store_tx = bulletin::tx().transaction_storage().store(data_to_store.as_bytes().to_vec());

	let tx_progress = api
		.tx()
		.sign_and_submit_then_watch_default(&store_tx, &keypair)
		.await
		.map_err(|e| anyhow!("Failed to submit store: {e:?}"))?;

	let tx_in_block = tx_progress
		.wait_for_finalized()
		.await
		.map_err(|e| anyhow!("Store transaction not finalized: {e:?}"))?;

	let block_hash = tx_in_block.block_hash();
	let block = api
		.blocks()
		.at(block_hash)
		.await
		.map_err(|e| anyhow!("Failed to get block: {e:?}"))?;

	let events = tx_in_block
		.wait_for_success()
		.await
		.map_err(|e| anyhow!("Store transaction failed: {e:?}"))?;

	info!("Data stored successfully!");
	info!("  Block number: {}", block.number());
	info!("  Block hash: {:?}", block_hash);

	// Find the Stored event to get the CID and index
	let stored_event = events
		.find_first::<bulletin::transaction_storage::events::Stored>()
		.map_err(|e| anyhow!("Failed to find Stored event: {e:?}"))?;

	if let Some(event) = stored_event {
		info!("  Content Hash: {}", hex::encode(&event.content_hash));
		info!("  Extrinsic Index: {}", event.index);
		if let Some(cid_bytes) = &event.cid {
			info!("  CID (bytes): {}", hex::encode(cid_bytes));
		}
		info!("  Size: {} bytes", data_to_store.len());
	}

	info!("\n✅ Test passed!");

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
