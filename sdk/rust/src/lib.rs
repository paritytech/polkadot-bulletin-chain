// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

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
//! ## Data Retrieval (Not Supported)
//!
//! This SDK does not provide data retrieval. Bulletin nodes and smoldot serve
//! stored blocks through the `bitswap_v1_get` JSON-RPC method (raw blocks, no DAG
//! assembly); call it through your own RPC client. Public IPFS gateways (e.g.
//! `https://ipfs.io/ipfs/{cid}`) are centralized and not recommended.
//!
//! ## Usage
//!
//! Uploads go through one primitive: [`estimate_upload`] → [`submit`]. The
//! estimate plans the upload and sizes the authorization (with `skip_existing`,
//! leaving out units already on chain); `submit` drives the items to finality through a
//! wave-batched, reconcile-driven pipeline with exactly-once guarantees, fetching each unit's
//! bytes lazily from the source.
//!
//! [`estimate_upload`]: crate::TransactionClient::estimate_upload
//! [`submit`]: crate::TransactionClient::submit
//!
//! ```ignore
//! use bulletin_sdk_rust::prelude::*;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
//!     // Connect (use `from_endpoints` for multi-provider broadcast).
//!     let client = TransactionClient::new("ws://localhost:10000").await?;
//!     let signer = subxt_signer::sr25519::dev::alice();
//!
//!     // Upload in-memory items: plan, then submit.
//!     let items = vec![UploadItem::new(b"hello".to_vec())];
//!     let datas: Vec<Vec<u8>> = items.iter().map(|i| i.data.clone()).collect();
//!     let estimate = client
//!         .estimate_upload(UploadInput::Items(items), UploadEstimateOptions::default())
//!         .await?;
//!     let source: Arc<dyn SeekableSource> = Arc::new(blob_from_items(datas));
//!     let result = client.submit(&signer, estimate, source, UploadConfig::default()).await?;
//!     println!("stored CIDs: {:?}", result.cids);
//!     Ok(())
//! }
//! ```
//!
//! ### Streaming a large file
//!
//! For files, pass a [`SeekableSource`] — chunked (with a DAG-PB manifest when it
//! spans more than one chunk), streamed once for the estimate, then range-read
//! lazily during submission so resident memory tracks the in-flight window, not
//! the whole file.
//!
//! ```ignore
//! use bulletin_sdk_rust::prelude::*;
//! use std::sync::Arc;
//!
//! let source: Arc<dyn SeekableSource> = Arc::new(blob_from_bytes(file_bytes));
//! let estimate = client
//!     .estimate_upload(UploadInput::Source(source.clone()), UploadEstimateOptions::default())
//!     .await?;
//! let result = client.submit(&signer, estimate, source, UploadConfig::default()).await?;
//! // With a manifest, `result.cids` ends with its root CID.
//! ```
//!
//! Use [`submit_unsigned`] for the preimage-authorized (no-signer) path.
//!
//! [`submit_unsigned`]: crate::TransactionClient::submit_unsigned
//! [`SeekableSource`]: crate::SeekableSource
//!
//! ### Offline preparation
//!
//! For custom submission, [`BulletinClient`] prepares operations (CID,
//! chunking, DAG building) without network access; submit them via your own
//! subxt client.
//!
//! ```ignore
//! use bulletin_sdk_rust::prelude::*;
//!
//! let client = BulletinClient::new();
//! let operation = client.prepare_store(b"Hello, Bulletin!".to_vec(), StoreOptions::default())?;
//! println!("CID: {:?}", cid_to_bytes(&operation.calculate_cid()?)?);
//! ```
//!
//! ## Feature Flags
//!
//! - `std` (default): Enable standard library support and subxt helpers
//!
//! ## no_std Support
//!
//! The SDK core is no_std compatible for use in constrained environments:
//!
//! ```toml
//! [dependencies]
//! bulletin-sdk-rust = { git = "https://github.com/paritytech/polkadot-bulletin-chain", default-features = false }
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

// Core modules (pub(crate) — public API is exposed via re-exports and prelude)
pub(crate) mod authorization;
pub(crate) mod chunker;
pub(crate) mod cid;
pub(crate) mod client;
pub(crate) mod dag;
pub(crate) mod renewal;
pub(crate) mod storage;
pub(crate) mod types;

// Transaction submission client (std-only)
#[cfg(feature = "std")]
pub(crate) mod transaction;

// Wave-batched upload pipeline (std-only)
#[cfg(feature = "std")]
pub(crate) mod pipeline;

// Re-openable byte sources + streaming plan/estimate (std-only)
#[cfg(feature = "std")]
pub(crate) mod blob_source;

// Partial metadata for incompatibly-changed items on supported chains, and
// the shape registry that dispatches between them (std-only). Public so
// integration tests and diagnostics can probe which shape a chain speaks.
#[cfg(feature = "std")]
pub mod compat;

// Re-export commonly used types
pub use client::{BulletinClient, ClientConfig};
pub use renewal::{RenewalOperation, RenewalTracker, TrackedEntry};
pub use types::{
	AuthorizationScope, Chunk, ChunkProgressEvent, ChunkedStoreResult, ChunkerConfig, Error,
	ProgressCallback, ProgressEvent, RenewalResult, Result, StorageRef, StoreOptions, StoreResult,
	TransactionStatusEvent, WaitFor,
};

// Re-export CID types from pallet
pub use cid::{calculate_cid, Cid, CidCodec, CidConfig, CidData, ContentHash, HashingAlgorithm};

// Re-export the renewal entry reference from the pallet primitives
pub use bulletin_transaction_storage_primitives::TransactionRef;

// Re-export pipeline upload types (std-only)
#[cfg(feature = "std")]
pub use pipeline::{
	BlockLimits, BroadcastArgs, ItemBroadcastResult, NonceTrackingStrategy, SubmissionStrategy,
	SubmissionStrategyKind, UploadCallback, UploadConfig, UploadEvent, UploadItem, UploadResult,
	UploadStatus, WaveResult, DEFAULT_BLOCK_LIMITS,
};

// Re-export streaming source + plan/estimate types (std-only)
#[cfg(feature = "std")]
pub use blob_source::{
	blob_from_bytes, blob_from_factory, blob_from_items, collect_blob, plan_stream, BlobSource,
	ChunkPlan, SeekableSource, SkipReason, StreamEstimate, UploadEstimate, UploadEstimateItem,
	UploadEstimateOptions,
};

// Re-export the transaction client + receipts (std-only)
#[cfg(feature = "std")]
pub use transaction::{
	AuthorizationReceipt, AuthorizeAccountEntry, PreimageAuthorizationReceipt, RenewReceipt,
	StoreReceipt, TransactionClient, UploadInput,
};

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
			CidCodec, CidConfig, CidData, ContentHash, HashingAlgorithm,
		},
		client::{BulletinClient, ClientConfig},
		dag::{DagBuilder, DagManifest, UnixFsDagBuilder},
		renewal::{RenewalOperation, RenewalTracker, TrackedEntry},
		storage::{BatchStorageOperation, StorageOperation},
		types::*,
		TransactionRef,
	};

	#[cfg(feature = "std")]
	pub use crate::transaction::{
		AuthorizationReceipt, AuthorizeAccountEntry, PreimageAuthorizationReceipt, RenewReceipt,
		StoreReceipt, TransactionClient, UploadInput,
	};

	#[cfg(feature = "std")]
	pub use crate::pipeline::{
		BlockLimits, BroadcastArgs, ItemBroadcastResult, NonceTrackingStrategy, SubmissionStrategy,
		SubmissionStrategyKind, UploadCallback, UploadConfig, UploadEvent, UploadItem,
		UploadResult, UploadStatus, WaveResult, DEFAULT_BLOCK_LIMITS,
	};

	#[cfg(feature = "std")]
	pub use crate::blob_source::{
		blob_from_bytes, blob_from_factory, blob_from_items, collect_blob, plan_stream, BlobSource,
		ChunkPlan, SeekableSource, SkipReason, StreamEstimate, UploadEstimate, UploadEstimateItem,
		UploadEstimateOptions,
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
