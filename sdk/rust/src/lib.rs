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
//! ### Simple Store (< 2 MiB)
//!
//! ```ignore
//! use bulletin_sdk_rust::{BulletinClient, types::StoreOptions};
//!
//! let client = BulletinClient::new();
//! let data = b"Hello, Bulletin!".to_vec();
//! let options = StoreOptions::default();
//!
//! let operation = client.prepare_store(data, options)?;
//! // Submit operation.data using subxt to TransactionStorage.store
//! ```
//!
//! ### Chunked Store (large files)
//!
//! ```ignore
//! use bulletin_sdk_rust::{BulletinClient, types::{ChunkerConfig, StoreOptions}};
//!
//! let client = BulletinClient::new();
//! let large_data = vec![0u8; 100_000_000]; // 100 MB
//!
//! let config = ChunkerConfig {
//!     chunk_size: 1024 * 1024, // 1 MiB
//!     max_parallel: 8,
//!     create_manifest: true,
//! };
//!
//! let (batch, manifest) = client.prepare_store_chunked(
//!     &large_data,
//!     Some(config),
//!     StoreOptions::default(),
//!     Some(|event| {
//!         println!("Progress: {:?}", event);
//!     }),
//! )?;
//!
//! // Submit each chunk in batch.operations using subxt
//! // Then submit the manifest data if present
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

// Transaction submission (std-only)
#[cfg(feature = "std")]
pub mod submit;

// Transaction submitters for different client libraries (std-only)
#[cfg(feature = "std")]
pub mod submitters;

// Async client with full transaction support (std-only)
#[cfg(feature = "std")]
pub mod async_client;

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
	pub use crate::async_client::{AsyncBulletinClient, AsyncClientConfig};

	#[cfg(feature = "std")]
	pub use crate::submit::{Call, TransactionBuilder, TransactionReceipt, TransactionSubmitter};

	#[cfg(feature = "std")]
	pub use crate::submitters::{MockSubmitter, SubxtSubmitter};
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
