// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! High-level client for interacting with Bulletin Chain.
//!
//! This module provides a simplified API for storing and retrieving data.
//! Full blockchain integration requires the `std` feature and `subxt`.

extern crate alloc;

use alloc::vec::Vec;
use crate::{
	authorization::AuthorizationManager,
	chunker::{Chunker, FixedSizeChunker},
	dag::{DagBuilder, UnixFsDagBuilder},
	storage::{BatchStorageOperation, StorageOperation},
	types::{
		ChunkerConfig, Error, ProgressCallback, ProgressEvent, Result,
		StoreOptions,
	},
};

/// Configuration for the Bulletin client.
#[derive(Debug, Clone)]
pub struct ClientConfig {
	/// Default chunk size for large files (default: 1 MiB).
	pub default_chunk_size: u32,
	/// Maximum parallel uploads (default: 8).
	pub max_parallel: u32,
	/// Whether to create manifests for chunked uploads (default: true).
	pub create_manifest: bool,
}

impl Default for ClientConfig {
	fn default() -> Self {
		Self {
			default_chunk_size: 1024 * 1024, // 1 MiB
			max_parallel: 8,
			create_manifest: true,
		}
	}
}

/// High-level client for Bulletin Chain operations.
///
/// This provides a simplified API for common operations like storing
/// and retrieving data, with automatic chunking and manifest creation.
///
/// For full functionality with blockchain integration, enable the `std` feature.
#[derive(Debug, Clone)]
pub struct BulletinClient {
	/// Client configuration.
	pub config: ClientConfig,
	/// Authorization manager.
	pub auth_manager: AuthorizationManager,
}

impl BulletinClient {
	/// Create a new Bulletin client with default configuration.
	pub fn new() -> Self {
		Self {
			config: ClientConfig::default(),
			auth_manager: AuthorizationManager::new(),
		}
	}

	/// Create a client with custom configuration.
	pub fn with_config(config: ClientConfig) -> Self {
		Self {
			config,
			auth_manager: AuthorizationManager::new(),
		}
	}

	/// Set the authorization manager.
	pub fn with_auth_manager(mut self, auth_manager: AuthorizationManager) -> Self {
		self.auth_manager = auth_manager;
		self
	}

	/// Prepare a simple store operation (data < 8 MiB).
	///
	/// This creates a storage operation that can be submitted to the blockchain.
	/// For actual submission, use `subxt` to call `TransactionStorage.store`.
	pub fn prepare_store(&self, data: Vec<u8>, options: StoreOptions) -> Result<StorageOperation> {
		let op = StorageOperation::new(data, options);
		op.validate()?;
		Ok(op)
	}

	/// Prepare a chunked store operation (data >= 8 MiB or when chunking is desired).
	///
	/// This chunks the data, calculates CIDs, and optionally creates a DAG-PB manifest.
	/// Returns the batch operation and optionally the manifest data.
	pub fn prepare_store_chunked(
		&self,
		data: &[u8],
		config: Option<ChunkerConfig>,
		options: StoreOptions,
		progress_callback: Option<ProgressCallback>,
	) -> Result<(BatchStorageOperation, Option<Vec<u8>>)> {
		if data.is_empty() {
			return Err(Error::EmptyData);
		}

		// Use provided config or default
		let chunker_config = config.unwrap_or_else(|| ChunkerConfig {
			chunk_size: self.config.default_chunk_size,
			max_parallel: self.config.max_parallel,
			create_manifest: self.config.create_manifest,
		});

		// Chunk the data
		let chunker = FixedSizeChunker::new(chunker_config.clone())?;
		let chunks = chunker.chunk(data)?;

		// Notify progress
		if let Some(callback) = progress_callback {
			callback(ProgressEvent::ChunkStarted { index: 0, total: chunks.len() as u32 });
		}

		// Create batch operation
		let batch = BatchStorageOperation::new(&chunks, options.clone())?;

		// Optionally create manifest
		let manifest_data = if chunker_config.create_manifest {
			if let Some(callback) = progress_callback {
				callback(ProgressEvent::ManifestStarted);
			}

			let builder = UnixFsDagBuilder::new();
			let manifest = builder.build(&chunks, options.hash_algorithm)?;

			if let Some(callback) = progress_callback {
				let cid_bytes = manifest.root_cid.to_bytes().ok_or_else(|| {
					Error::DagEncodingFailed("Failed to convert manifest CID to bytes".into())
				})?;
				callback(ProgressEvent::ManifestCreated { cid: cid_bytes });
			}

			Some(manifest.dag_bytes)
		} else {
			None
		};

		Ok((batch, manifest_data))
	}

	/// Estimate authorization needed for storing data.
	pub fn estimate_authorization(&self, data_size: u64) -> (u32, u64) {
		self.auth_manager.estimate_authorization(data_size, self.config.create_manifest)
	}
}

impl Default for BulletinClient {
	fn default() -> Self {
		Self::new()
	}
}

/// Simplified client operations (requires std and subxt for full functionality).
#[cfg(feature = "std")]
pub mod async_client {
	use super::*;

	/// Async Bulletin client wrapper.
	///
	/// This would integrate with `subxt` for actual blockchain interaction.
	/// Users should implement their own version based on their subxt setup.
	pub struct AsyncBulletinClient {
		/// The underlying client.
		pub client: BulletinClient,
	}

	impl AsyncBulletinClient {
		/// Create a new async client.
		pub fn new(config: ClientConfig) -> Self {
			Self {
				client: BulletinClient::with_config(config),
			}
		}

		/// Store data (placeholder - requires subxt integration).
		///
		/// Example integration:
		/// ```ignore
		/// use subxt::OnlineClient;
		///
		/// async fn store(&self, api: &OnlineClient<PolkadotConfig>, data: Vec<u8>) -> Result<StoreResult> {
		///     let op = self.client.prepare_store(data, StoreOptions::default())?;
		///     let cid_config = // convert op.cid_config to runtime type
		///     let tx = bulletin::tx()
		///         .transaction_storage()
		///         .store(op.data, Some(cid_config));
		///     let result = api.tx().sign_and_submit_then_watch_default(&tx, signer).await?;
		///     // Process result and return StoreResult
		/// }
		/// ```
		pub async fn store_placeholder(&self, _data: Vec<u8>, _options: StoreOptions) -> Result<()> {
			// Placeholder - users should implement with their subxt setup
			Err(Error::StorageFailed(
				"This is a placeholder. Implement with subxt integration.".into(),
			))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_client_new() {
		let client = BulletinClient::new();
		assert_eq!(client.config.default_chunk_size, 1024 * 1024);
		assert_eq!(client.config.max_parallel, 8);
		assert!(client.config.create_manifest);
	}

	#[test]
	fn test_prepare_store() {
		let client = BulletinClient::new();
		let data = vec![1, 2, 3, 4, 5];
		let options = StoreOptions::default();

		let result = client.prepare_store(data, options);
		assert!(result.is_ok());
	}

	#[test]
	fn test_prepare_store_empty() {
		let client = BulletinClient::new();
		let data = vec![];
		let options = StoreOptions::default();

		let result = client.prepare_store(data, options);
		assert!(result.is_err());
	}

	#[test]
	fn test_prepare_store_chunked() {
		let client = BulletinClient::new();
		let data = vec![1u8; 5000];
		let config = Some(ChunkerConfig {
			chunk_size: 2000,
			max_parallel: 8,
			create_manifest: true,
		});
		let options = StoreOptions::default();

		let result = client.prepare_store_chunked(&data, config, options, None);
		assert!(result.is_ok());

		let (batch, manifest) = result.unwrap();
		assert_eq!(batch.len(), 3);
		assert!(manifest.is_some());
	}

	#[test]
	fn test_prepare_store_chunked_no_manifest() {
		let client = BulletinClient::new();
		let data = vec![1u8; 5000];
		let config = Some(ChunkerConfig {
			chunk_size: 2000,
			max_parallel: 8,
			create_manifest: false,
		});
		let options = StoreOptions::default();

		let result = client.prepare_store_chunked(&data, config, options, None);
		assert!(result.is_ok());

		let (batch, manifest) = result.unwrap();
		assert_eq!(batch.len(), 3);
		assert!(manifest.is_none());
	}

	#[test]
	fn test_estimate_authorization() {
		let client = BulletinClient::new();
		let (txs, bytes) = client.estimate_authorization(10_000_000);

		// 10 MB = 10 chunks + 1 manifest
		assert_eq!(txs, 11);
		assert!(bytes > 10_000_000);
	}
}
