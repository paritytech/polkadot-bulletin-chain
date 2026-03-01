// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! # Bulletin SDK for Rust
//!
//! Off-chain client SDK for Polkadot Bulletin Chain that simplifies data storage
//! with automatic chunking, authorization management, and DAG-PB manifest generation.
//!
//! ## Storage Operations (Supported)
//!
//! - **Automatic Chunking**: Split large files into optimal chunks (default 1 MiB)
//! - **DAG-PB Manifests**: Manifest generation for chunked data
//! - **Authorization Management**: Helper functions for account and preimage authorization
//! - **Progress Tracking**: Callback-based progress events for uploads
//! - **no_std Compatible**: Core functionality works in no_std environments
//!
//! ## Data Retrieval (Not Yet Supported)
//!
//! **Important**: This SDK currently does NOT provide data retrieval functionality.
//!
//! ### Deprecated: IPFS Gateway Retrieval
//!
//! Retrieving data via public IPFS gateways (e.g., `https://ipfs.io/ipfs/{cid}`) is
//! **deprecated** and not recommended. Public gateways are centralized infrastructure
//! that goes against the decentralization goals of the Bulletin Chain.
//!
//! ### Future: Smoldot Light Client Retrieval
//!
//! Data retrieval will be supported via the smoldot light client's `bitswap_block` RPC.
//! This approach allows fully decentralized data retrieval directly from Bulletin
//! validator nodes without relying on centralized gateways.
//!
//! See: <https://github.com/paritytech/polkadot-bulletin-chain/pull/264>
//!
//! ### Current Workaround: Direct P2P via libp2p
//!
//! For applications that need retrieval now, connect directly to Bulletin validator
//! nodes using libp2p with their P2P multiaddrs. This is decentralized but requires
//! additional dependencies. See the console-ui implementation for reference.
//!
//! ## Usage
//!
//! ### Using TransactionClient (Recommended)
//!
//! The SDK provides `TransactionClient` for direct transaction submission:
//!
//! ```ignore
//! use bulletin_sdk_rust::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Connect to the chain
//!     let client = TransactionClient::new("ws://localhost:9944").await?;
//!
//!     // Store data
//!     let receipt = client.store(b"Hello, Bulletin!".to_vec(), &signer).await?;
//!     println!("Stored in block: {}", receipt.block_hash);
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Low-Level: Prepare and Submit Separately
//!
//! For more control, prepare operations with `BulletinClient` and submit manually:
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

// Transaction submission client (std-only)
#[cfg(feature = "std")]
pub mod transaction;

// Re-export commonly used types
pub use client::{BulletinClient, ClientConfig};
pub use renewal::{RenewalOperation, RenewalTracker, TrackedEntry};
pub use types::{
	AuthorizationScope, Chunk, ChunkProgressEvent, ChunkedStoreResult, ChunkerConfig, Error,
	ProgressCallback, ProgressEvent, RenewalResult, Result, StorageRef, StoreOptions, StoreResult,
	TransactionStatusEvent,
};

// Re-export CID types from pallet
pub use cid::{calculate_cid, Cid, CidCodec, CidConfig, CidData, ContentHash, HashingAlgorithm};

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
	};

	#[cfg(feature = "std")]
	pub use crate::transaction::{
		AuthorizationReceipt, PreimageAuthorizationReceipt, RenewReceipt, StoreReceipt,
		TransactionClient,
	};
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
