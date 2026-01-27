// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Simple store example - Store small data on Bulletin Chain
//!
//! This example demonstrates:
//! - Connecting to a local Bulletin Chain node
//! - Creating a TransactionSubmitter implementation with subxt
//! - Storing data and getting back the CID
//! - Viewing the transaction receipt
//!
//! Usage:
//!   cargo run --example simple_store --features std

use bulletin_sdk_rust::{
	async_client::{AsyncBulletinClient, AsyncClientConfig},
	prelude::*,
	submit::{TransactionReceipt, TransactionSubmitter},
};
use sp_core::sr25519::Pair;
use sp_runtime::AccountId32;
use subxt::{tx::PairSigner, OnlineClient, PolkadotConfig};

// This would normally be generated from chain metadata using subxt
// For this example, we'll use a simplified version
#[subxt::subxt(runtime_metadata_path = "metadata.scale")]
pub mod bulletin_runtime {}

/// Example implementation of TransactionSubmitter using subxt
struct SubxtSubmitter {
	api: OnlineClient<PolkadotConfig>,
	signer: PairSigner<PolkadotConfig, Pair>,
}

impl SubxtSubmitter {
	async fn new(endpoint: &str, signer: PairSigner<PolkadotConfig, Pair>) -> Result<Self> {
		let api = OnlineClient::<PolkadotConfig>::from_url(endpoint)
			.await
			.map_err(|e| Error::NetworkError(format!("Failed to connect: {:?}", e)))?;

		Ok(Self { api, signer })
	}
}

#[async_trait::async_trait]
impl TransactionSubmitter for SubxtSubmitter {
	async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt> {
		let tx = bulletin_runtime::tx().transaction_storage().store(data);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_authorize_account(
		&self,
		who: AccountId32,
		transactions: u32,
		bytes: u64,
	) -> Result<TransactionReceipt> {
		let tx = bulletin_runtime::tx()
			.transaction_storage()
			.authorize_account(who, transactions, bytes);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_authorize_preimage(
		&self,
		content_hash: [u8; 32],
		max_size: u64,
	) -> Result<TransactionReceipt> {
		let tx = bulletin_runtime::tx()
			.transaction_storage()
			.authorize_preimage(content_hash, max_size);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_renew(&self, block: u32, index: u32) -> Result<TransactionReceipt> {
		let tx = bulletin_runtime::tx().transaction_storage().renew(block, index);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_refresh_account_authorization(
		&self,
		who: AccountId32,
	) -> Result<TransactionReceipt> {
		let tx = bulletin_runtime::tx()
			.transaction_storage()
			.refresh_account_authorization(who);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_refresh_preimage_authorization(
		&self,
		content_hash: [u8; 32],
	) -> Result<TransactionReceipt> {
		let tx = bulletin_runtime::tx()
			.transaction_storage()
			.refresh_preimage_authorization(content_hash);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_remove_expired_account_authorization(
		&self,
		who: AccountId32,
	) -> Result<TransactionReceipt> {
		let tx = bulletin_runtime::tx()
			.transaction_storage()
			.remove_expired_account_authorization(who);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_remove_expired_preimage_authorization(
		&self,
		content_hash: [u8; 32],
	) -> Result<TransactionReceipt> {
		let tx = bulletin_runtime::tx()
			.transaction_storage()
			.remove_expired_preimage_authorization(content_hash);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}
}

#[tokio::main]
async fn main() -> Result<()> {
	println!("🚀 Bulletin SDK - Simple Store Example\n");

	// 1. Setup: Connect to local node
	println!("📡 Connecting to Bulletin Chain at ws://localhost:9944...");
	let endpoint = "ws://localhost:9944";

	// 2. Create signer (using Alice's dev account)
	let signer = PairSigner::new(Pair::from_string("//Alice", None).expect("Valid dev seed"));
	println!("🔑 Using account: {:?}\n", signer.account_id());

	// 3. Create submitter
	let submitter = SubxtSubmitter::new(endpoint, signer).await?;
	println!("✅ Connected to Bulletin Chain\n");

	// 4. Create Bulletin client
	let client = AsyncBulletinClient::new(submitter);

	// 5. Prepare data to store
	let data = b"Hello, Bulletin Chain! This is a simple store example.";
	println!("📝 Data to store: {} bytes", data.len());
	println!("   Content: {:?}\n", String::from_utf8_lossy(data));

	// 6. Store data (complete workflow!)
	println!("⏳ Storing data on chain...");
	let result = client.store(data.to_vec(), StoreOptions::default()).await?;

	// 7. Display results
	println!("✅ Data stored successfully!\n");
	println!("📊 Results:");
	println!("   CID (bytes): {} bytes", result.cid.len());
	#[cfg(feature = "std")]
	if let Ok(cid) = crate::cid::cid_from_bytes(&result.cid) {
		println!("   CID (base32): {}", crate::cid::cid_to_string(&cid));
	}
	println!("   Data size: {} bytes", result.size);
	if let Some(block) = result.block_number {
		println!("   Block number: {}", block);
	}

	println!("\n🎉 Example completed successfully!");
	println!("\n💡 Next steps:");
	println!("   - Try the chunked_store example for large files");
	println!("   - Use the CID to retrieve data via IPFS gateway");
	println!("   - Check the authorization example for managing permissions");

	Ok(())
}
