// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Integration tests for Bulletin SDK
//!
//! These tests require a running Bulletin Chain node at ws://localhost:9944
//!
//! Run with: cargo test --test integration_tests --features std -- --test-threads=1
//!
//! Note: Use --test-threads=1 to avoid parallel test conflicts on the same chain

#![cfg(feature = "std")]

use bulletin_sdk_rust::{
	async_client::{AsyncBulletinClient, AsyncClientConfig},
	prelude::*,
	submit::{TransactionReceipt, TransactionSubmitter},
};
use sp_core::{sr25519::Pair, Pair as PairT};
use sp_runtime::AccountId32;
use std::str::FromStr;
use subxt::{tx::PairSigner, OnlineClient, PolkadotConfig};

// Mock chain metadata - in real tests, generate with subxt macro
mod mock_runtime {
	#[subxt::subxt(runtime_metadata_path = "../../artifacts/metadata.scale")]
	pub mod bulletin {}
}

use mock_runtime::bulletin;

/// Test submitter implementation using subxt
struct TestSubmitter {
	api: OnlineClient<PolkadotConfig>,
	signer: PairSigner<PolkadotConfig, Pair>,
}

impl TestSubmitter {
	async fn new(endpoint: &str, seed: &str) -> Result<Self> {
		let api = OnlineClient::<PolkadotConfig>::from_url(endpoint)
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Connection failed: {:?}", e)))?;

		let pair = Pair::from_string(seed, None).expect("Valid dev seed");
		let signer = PairSigner::new(pair);

		Ok(Self { api, signer })
	}
}

#[async_trait::async_trait]
impl TransactionSubmitter for TestSubmitter {
	async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt> {
		let tx = bulletin::tx().transaction_storage().store(data);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Submission failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None, // Would need to query for block number
		})
	}

	async fn submit_authorize_account(
		&self,
		who: AccountId32,
		transactions: u32,
		bytes: u64,
	) -> Result<TransactionReceipt> {
		let tx = bulletin::tx().transaction_storage().authorize_account(who, transactions, bytes);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Submission failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_authorize_preimage(
		&self,
		content_hash: ContentHash,
		max_size: u64,
	) -> Result<TransactionReceipt> {
		let tx = bulletin::tx().transaction_storage().authorize_preimage(content_hash, max_size);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Submission failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_renew(&self, block: u32, index: u32) -> Result<TransactionReceipt> {
		let tx = bulletin::tx().transaction_storage().renew(block, index);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Submission failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Finalization failed: {:?}", e)))?;

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
		let tx = bulletin::tx().transaction_storage().refresh_account_authorization(who);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Submission failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_refresh_preimage_authorization(
		&self,
		content_hash: ContentHash,
	) -> Result<TransactionReceipt> {
		let tx = bulletin::tx()
			.transaction_storage()
			.refresh_preimage_authorization(content_hash);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Submission failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Finalization failed: {:?}", e)))?;

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
		let tx = bulletin::tx().transaction_storage().remove_expired_account_authorization(who);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Submission failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}

	async fn submit_remove_expired_preimage_authorization(
		&self,
		content_hash: ContentHash,
	) -> Result<TransactionReceipt> {
		let tx = bulletin::tx()
			.transaction_storage()
			.remove_expired_preimage_authorization(content_hash);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, &self.signer)
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Submission failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::SubmissionFailed(format!("Finalization failed: {:?}", e)))?;

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", result.block_hash()),
			extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
			block_number: None,
		})
	}
}

/// Helper to create test client
async fn create_test_client(seed: &str) -> Result<AsyncBulletinClient<TestSubmitter>> {
	let endpoint = "ws://localhost:9944";
	let submitter = TestSubmitter::new(endpoint, seed).await?;
	Ok(AsyncBulletinClient::new(submitter))
}

#[tokio::test]
#[ignore] // Run with --ignored flag when local node is available
async fn test_simple_store() -> Result<()> {
	let client = create_test_client("//Alice").await?;

	let data = b"Hello, Bulletin Chain! Integration test.".to_vec();
	let result = client.store(data.clone(), StoreOptions::default()).await?;

	assert!(result.cid.len() > 0);
	assert_eq!(result.size, data.len() as u64);

	println!("✅ Simple store test passed");
	println!("   CID: {:?}", hex::encode(&result.cid));
	println!("   Size: {} bytes", result.size);

	Ok(())
}

#[tokio::test]
#[ignore]
async fn test_chunked_store() -> Result<()> {
	let client = create_test_client("//Alice").await?;

	// Create 5 MiB test data
	let data = vec![0x42u8; 5 * 1024 * 1024];

	let mut chunks_completed = 0;
	let result = client
		.store_chunked(
			&data,
			Some(ChunkerConfig {
				chunk_size: 1_048_576, // 1 MiB
				max_parallel: 4,
				create_manifest: true,
			}),
			StoreOptions::default(),
			Some(Box::new(move |event| match event {
				ProgressEvent::ChunkCompleted { index, total, .. } => {
					chunks_completed += 1;
					println!("   Chunk {}/{} completed", index + 1, total);
				},
				ProgressEvent::ManifestCreated { .. } => println!("   Manifest created"),
				_ => {},
			})),
		)
		.await?;

	assert_eq!(result.num_chunks, 5); // 5 MiB / 1 MiB = 5 chunks
	assert!(result.manifest_cid.is_some());

	println!("✅ Chunked store test passed");
	println!("   Chunks: {}", result.num_chunks);
	println!("   Manifest CID: {:?}", result.manifest_cid.map(|c| hex::encode(&c)));

	Ok(())
}

#[tokio::test]
#[ignore]
async fn test_authorization_workflow() -> Result<()> {
	let alice_client = create_test_client("//Alice").await?;

	// Get Bob's account
	let bob_account_str = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty";
	let bob_account = AccountId32::from_str(bob_account_str)
		.map_err(|e| Error::InvalidConfig(format!("Invalid account: {:?}", e)))?;

	// Estimate authorization
	let (transactions, bytes) = alice_client.estimate_authorization(1_000_000);
	println!("   Authorization estimate: {} tx, {} bytes", transactions, bytes);

	// Authorize Bob's account (Alice has sudo)
	let receipt = alice_client.authorize_account(bob_account.clone(), transactions, bytes).await?;

	assert!(!receipt.block_hash.is_empty());
	println!("✅ Account authorization test passed");
	println!("   Block hash: {}", receipt.block_hash);

	// Test refresh
	let refresh_receipt = alice_client.refresh_account_authorization(bob_account.clone()).await?;

	assert!(!refresh_receipt.block_hash.is_empty());
	println!("✅ Authorization refresh test passed");

	Ok(())
}

#[tokio::test]
#[ignore]
async fn test_preimage_authorization() -> Result<()> {
	let alice_client = create_test_client("//Alice").await?;

	let data = b"Test preimage content";
	let content_hash = sp_io::hashing::blake2_256(data);

	// Authorize preimage
	let receipt = alice_client.authorize_preimage(content_hash, data.len() as u64).await?;

	assert!(!receipt.block_hash.is_empty());
	println!("✅ Preimage authorization test passed");

	// Test refresh
	let refresh_receipt = alice_client.refresh_preimage_authorization(content_hash).await?;

	assert!(!refresh_receipt.block_hash.is_empty());
	println!("✅ Preimage refresh test passed");

	Ok(())
}

#[tokio::test]
#[ignore]
async fn test_cid_calculation() -> Result<()> {
	let data = b"Test data for CID calculation";

	// Test different hash algorithms
	let cid_blake2 = crate::cid::calculate_cid(data, CidCodec::Raw, HashAlgorithm::Blake2b256)?;
	let cid_sha2 = crate::cid::calculate_cid(data, CidCodec::Raw, HashAlgorithm::Sha2_256)?;

	assert_ne!(cid_blake2, cid_sha2);
	assert!(cid_blake2.len() > 0);
	assert!(cid_sha2.len() > 0);

	println!("✅ CID calculation test passed");
	println!("   Blake2b-256 CID: {:?}", hex::encode(&cid_blake2));
	println!("   SHA2-256 CID: {:?}", hex::encode(&cid_sha2));

	Ok(())
}

#[tokio::test]
#[ignore]
async fn test_chunking() -> Result<()> {
	use bulletin_sdk_rust::chunker::{Chunker, FixedSizeChunker};

	let data = vec![0xAAu8; 10 * 1024 * 1024]; // 10 MiB
	let config = ChunkerConfig {
		chunk_size: 1_048_576, // 1 MiB
		max_parallel: 8,
		create_manifest: false,
	};

	let chunker = FixedSizeChunker::new(config);
	let chunks = chunker.chunk(&data)?;

	assert_eq!(chunks.len(), 10); // 10 MiB / 1 MiB = 10 chunks

	// Verify each chunk size
	for (i, chunk) in chunks.iter().enumerate() {
		assert_eq!(chunk.data.len(), 1_048_576);
		assert_eq!(chunk.index, i as u32);
		assert_eq!(chunk.total_chunks, 10);
	}

	println!("✅ Chunking test passed");
	println!("   Chunks created: {}", chunks.len());

	Ok(())
}

#[tokio::test]
#[ignore]
async fn test_dag_manifest() -> Result<()> {
	use bulletin_sdk_rust::{
		chunker::{Chunk, Chunker, FixedSizeChunker},
		dag::{DagBuilder, UnixFsDagBuilder},
	};

	let data = vec![0xBBu8; 5 * 1024 * 1024]; // 5 MiB
	let config = ChunkerConfig { chunk_size: 1_048_576, max_parallel: 8, create_manifest: true };

	let chunker = FixedSizeChunker::new(config);
	let chunks = chunker.chunk(&data)?;

	let builder = UnixFsDagBuilder;
	let manifest = builder.build(&chunks, HashAlgorithm::Blake2b256)?;

	assert!(manifest.root_cid.len() > 0);
	assert_eq!(manifest.chunks.len(), 5);
	assert_eq!(manifest.total_size, 5 * 1024 * 1024);

	println!("✅ DAG manifest test passed");
	println!("   Root CID: {:?}", hex::encode(&manifest.root_cid));
	println!("   Chunks: {}", manifest.chunks.len());

	Ok(())
}

#[test]
fn test_error_types() {
	// Test error creation
	let err1 = Error::ChunkingFailed("Test error".to_string());
	assert!(matches!(err1, Error::ChunkingFailed(_)));

	let err2 = Error::InvalidCid("Invalid CID".to_string());
	assert!(matches!(err2, Error::InvalidCid(_)));

	let err3 = Error::SubmissionFailed("Submission failed".to_string());
	assert!(matches!(err3, Error::SubmissionFailed(_)));

	println!("✅ Error types test passed");
}

#[test]
fn test_authorization_estimation() {
	use bulletin_sdk_rust::authorization::AuthorizationManager;

	let manager = AuthorizationManager::new();

	// Test 1 MB
	let (tx1, bytes1) = manager.estimate_authorization(1_048_576);
	assert_eq!(tx1, 1);
	assert_eq!(bytes1, 1_048_576);

	// Test 10 MB
	let (tx2, bytes2) = manager.estimate_authorization(10_485_760);
	assert_eq!(tx2, 10);
	assert_eq!(bytes2, 10_485_760);

	// Test fractional chunk
	let (tx3, bytes3) = manager.estimate_authorization(1_500_000);
	assert_eq!(tx3, 2); // Rounds up
	assert_eq!(bytes3, 1_500_000);

	println!("✅ Authorization estimation test passed");
}
