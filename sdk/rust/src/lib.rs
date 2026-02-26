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
//! ### Prepare and Submit via Subxt (Recommended)
//!
//! The SDK prepares storage operations; you submit them via subxt with your
//! runtime metadata. This gives you full control over transaction parameters.
//!
//! > **Note**: `AsyncBulletinClient` exists but is experimental and returns
//! > placeholder errors. Use `BulletinClient` for preparation and submit
//! > transactions directly via subxt.
//!
//! ### Step 1: Prepare the Operation
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
//! ### Step 2: Submit via Subxt
//!
//! ```ignore
//! use subxt::{OnlineClient, PolkadotConfig};
//!
//! // Connect to the chain
//! let api = OnlineClient::<PolkadotConfig>::from_url("ws://localhost:9944").await?;
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
//! ### Testing with MockBulletinClient
//!
//! For testing without a running node:
//!
//! ```ignore
//! use bulletin_sdk_rust::prelude::*;
//!
//! let client = MockBulletinClient::new();
//! let result = client.store(data).send().await?;
//! println!("Mock CID: {:?}", result.cid);
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
pub mod renewal;
pub mod storage;
pub mod types;
pub mod utils;

// Async client with full transaction support (std-only)
#[cfg(feature = "std")]
pub mod async_client;

// Mock client for testing (std-only)
#[cfg(feature = "std")]
pub mod mock_client;


// Re-export commonly used types
pub use client::{BulletinClient, ClientConfig};
pub use renewal::{RenewalOperation, RenewalTracker, TrackedEntry};
pub use types::{
	AuthorizationScope, Chunk, ChunkProgressEvent, ChunkedStoreResult, ChunkerConfig, CidCodec,
	Error, HashAlgorithm, ProgressCallback, ProgressEvent, RenewalResult, Result, StorageRef,
	StoreOptions, StoreResult, TransactionStatusEvent,
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
		renewal::{RenewalOperation, RenewalTracker, TrackedEntry},
		storage::{BatchStorageOperation, StorageOperation},
		types::*,
		utils,
	};

	#[cfg(feature = "std")]
	pub use crate::async_client::{AsyncBulletinClient, AsyncClientConfig, StoreBuilder};

	#[cfg(feature = "std")]
	pub use crate::mock_client::{MockBulletinClient, MockClientConfig, MockOperation};
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
