// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! # Bulletin SDK for Rust
//!
//! Off-chain client SDK for Polkadot Bulletin Chain that simplifies data storage
//! with automatic chunking, authorization management, and DAG-PB manifest generation.
//!
//! ## Features
//!
//! - **Automatic Chunking**: Split large files into optimal chunks (default 1 MiB)
//! - **DAG-PB Manifests**: IPFS-compatible manifest generation for chunked data
//! - **Authorization Management**: Helper functions for account and preimage authorization
//! - **Progress Tracking**: Callback-based progress events for uploads
//! - **no_std Compatible**: Core functionality works in no_std environments
//!
//! ## Usage
//!
//! ### Quick Start with AsyncBulletinClient (Recommended)
//!
//! The easiest way to store data is using [`AsyncBulletinClient`], which handles
//! connection, chunking, and transaction submission automatically:
//!
//! ```ignore
//! use bulletin_sdk_rust::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Connect to a Bulletin Chain node
//!     let client = AsyncBulletinClient::new("ws://localhost:9944").await?;
//!
//!     // Create a signer from a seed phrase or dev account
//!     let signer = Keypair::from_uri("//Alice")?;
//!
//!     // Store data - handles chunking and submission automatically
//!     let data = b"Hello, Bulletin!".to_vec();
//!     let result = client.store(data, &signer).await?;
//!
//!     println!("Stored with CID: {}", result.cid);
//!     Ok(())
//! }
//! ```
//!
//! ### Low-Level API: Prepare and Submit Separately
//!
//! For more control, you can prepare operations and submit them manually.
//! This is useful for batching, custom transaction parameters, or integration
//! with existing subxt setups.
//!
//! #### Step 1: Prepare the Operation
//!
//! ```ignore
//! use bulletin_sdk_rust::{BulletinClient, types::StoreOptions};
//!
//! let client = BulletinClient::new();
//! let data = b"Hello, Bulletin!".to_vec();
//! let options = StoreOptions::default();
//!
//! // This only prepares the data and calculates the CID - no network calls yet
//! let operation = client.prepare_store(data, options)?;
//! println!("Will store {} bytes", operation.size());
//! ```
//!
//! #### Step 2: Submit via Subxt
//!
//! ```ignore
//! use subxt::{OnlineClient, PolkadotConfig};
//! use bulletin_sdk_rust::subxt_config::BulletinConfig;
//!
//! // Connect to the chain
//! let api = OnlineClient::<BulletinConfig>::from_url("ws://localhost:9944").await?;
//!
//! // Build and submit the transaction
//! // (exact call depends on your runtime's metadata)
//! let tx = bulletin::tx().transaction_storage().store(
//!     operation.data,
//!     Some(operation.cid_config),
//! );
//! let result = tx.sign_and_submit_then_watch_default(&api, &signer).await?;
//! ```
//!
//! ### Chunked Store (Large Files)
//!
//! For files larger than 2 MiB, use chunked storage:
//!
//! ```ignore
//! use bulletin_sdk_rust::prelude::*;
//! use std::sync::Arc;
//!
//! let client = BulletinClient::new();
//! let large_data = vec![0u8; 100_000_000]; // 100 MB
//!
//! let config = ChunkerConfig {
//!     chunk_size: 1024 * 1024, // 1 MiB chunks
//!     max_parallel: 8,
//!     create_manifest: true,
//! };
//!
//! // Progress callback (must be Arc<dyn Fn> for thread safety)
//! let progress = Arc::new(|event: ProgressEvent| {
//!     println!("Progress: {:?}", event);
//! });
//!
//! let (batch, manifest) = client.prepare_store_chunked(
//!     &large_data,
//!     Some(config),
//!     StoreOptions::default(),
//!     Some(progress),
//! )?;
//!
//! println!("Prepared {} chunks", batch.len());
//! if let Some(ref m) = manifest {
//!     println!("Manifest size: {} bytes", m.len());
//! }
//!
//! // Submit each operation in batch.operations via subxt
//! // Then submit the manifest if present
//! ```
//!
//! Or use `AsyncBulletinClient` which handles chunking automatically:
//!
//! ```ignore
//! let result = client
//!     .store_builder(large_data)
//!     .with_chunk_size(1024 * 1024)
//!     .with_progress(|event| println!("{:?}", event))
//!     .send(&signer)
//!     .await?;
//! ```
//!
//! ## Feature Flags
//!
//! - `std` (default): Enable standard library support and subxt helpers
//! - `serde-support`: Enable serialization support for DAG structures
//!
//! ## no_std Support
//!
//! The SDK core is no_std compatible for use in constrained environments:
//!
//! ```toml
//! [dependencies]
//! bulletin-sdk-rust = { version = "0.1", default-features = false }
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

// Re-export codec for users
pub use codec;

// Core modules
pub mod authorization;
pub mod chunker;
pub mod cid;
pub mod client;
pub mod dag;
pub mod storage;
pub mod types;
pub mod utils;

// Async client with full transaction support (std-only)
#[cfg(feature = "std")]
pub mod async_client;

// Mock client for testing (std-only)
#[cfg(feature = "std")]
pub mod mock_client;

// Subxt configuration and custom signed extensions (std-only)
#[cfg(feature = "std")]
pub mod subxt_config;

// Re-export commonly used types
pub use client::{BulletinClient, ClientConfig};
pub use types::{
	AuthorizationScope, Chunk, ChunkedStoreResult, ChunkerConfig, CidCodec, Error, HashAlgorithm,
	ProgressCallback, ProgressEvent, Result, StoreOptions, StoreResult,
};

// Re-export CID types from pallet
pub use cid::{calculate_cid, Cid, CidConfig, CidData, ContentHash, HashingAlgorithm};

// Re-export key traits
pub use chunker::Chunker;
pub use dag::DagBuilder;

/// SDK version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Prelude module for convenient imports.
pub mod prelude {
	pub use crate::{
		authorization::{Authorization, AuthorizationManager},
		chunker::{Chunker, FixedSizeChunker},
		cid::{
			calculate_cid, calculate_cid_default, calculate_cid_with_config, cid_to_bytes, Cid,
			CidConfig, CidData, ContentHash,
		},
		client::{BulletinClient, ClientConfig},
		dag::{DagBuilder, DagManifest, UnixFsDagBuilder},
		storage::{BatchStorageOperation, StorageOperation},
		types::*,
		utils,
	};

	#[cfg(feature = "std")]
	pub use crate::async_client::{AsyncBulletinClient, AsyncClientConfig, StoreBuilder};

	#[cfg(feature = "std")]
	pub use crate::mock_client::{MockBulletinClient, MockClientConfig, MockOperation};

	#[cfg(feature = "std")]
	pub use crate::subxt_config::{BulletinConfig, BulletinExtrinsicParams, ProvideCidConfig};
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_version() {
		// VERSION is defined at compile time from CARGO_PKG_VERSION
		assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
	}

	#[test]
	fn test_prelude_imports() {
		use crate::prelude::*;

		// Test that all prelude imports are accessible
		let _client = BulletinClient::new();
		let _config = ChunkerConfig::default();
		let _options = StoreOptions::default();
	}
}
