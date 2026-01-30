// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Async client with full transaction submission support.
//!
//! This module provides a complete client that handles both data preparation
//! and transaction submission to the Bulletin Chain.

use crate::{
	authorization::AuthorizationManager,
	chunker::{Chunker, FixedSizeChunker},
	dag::{DagBuilder, UnixFsDagBuilder},
	submit::{TransactionReceipt, TransactionSubmitter},
	types::{
		ChunkDetails, ChunkedStoreResult, ChunkerConfig, Error, ProgressCallback, ProgressEvent,
		Result, StoreOptions, StoreResult,
	},
};
use alloc::vec::Vec;
use sp_runtime::AccountId32;

/// Configuration for the async Bulletin client.
#[derive(Debug, Clone)]
pub struct AsyncClientConfig {
	/// Default chunk size for large files (default: 1 MiB).
	pub default_chunk_size: u32,
	/// Maximum parallel uploads (default: 8).
	pub max_parallel: u32,
	/// Whether to create manifests for chunked uploads (default: true).
	pub create_manifest: bool,
	/// Check authorization before uploading to fail fast (default: true).
	/// Queries blockchain for current authorization and validates before submission.
	pub check_authorization_before_upload: bool,
	/// Threshold for automatic chunking (default: 2 MiB).
	/// Data larger than this will be automatically chunked by `store()`.
	pub chunking_threshold: u32,
}

impl Default for AsyncClientConfig {
	fn default() -> Self {
		Self {
			default_chunk_size: 1024 * 1024, // 1 MiB
			max_parallel: 8,
			create_manifest: true,
			check_authorization_before_upload: true,
			chunking_threshold: 2 * 1024 * 1024, // 2 MiB
		}
	}
}

/// Async Bulletin client that submits transactions to the chain.
///
/// This client provides a complete interface for storing data on Bulletin Chain,
/// handling everything from chunking to transaction submission.
pub struct AsyncBulletinClient<S: TransactionSubmitter> {
	/// Client configuration.
	pub config: AsyncClientConfig,
	/// Authorization manager.
	pub auth_manager: AuthorizationManager,
	/// Transaction submitter.
	submitter: S,
	/// Account for authorization checks (optional).
	/// If set and check_authorization_before_upload is enabled, the client will
	/// query and validate authorization before uploading.
	account: Option<AccountId32>,
}

impl<S: TransactionSubmitter> AsyncBulletinClient<S> {
	/// Create a new async client with the given submitter.
	pub fn new(submitter: S) -> Self {
		Self {
			config: AsyncClientConfig::default(),
			auth_manager: AuthorizationManager::new(),
			submitter,
			account: None,
		}
	}

	/// Create a client with custom configuration.
	pub fn with_config(submitter: S, config: AsyncClientConfig) -> Self {
		Self { config, auth_manager: AuthorizationManager::new(), submitter, account: None }
	}

	/// Set the authorization manager.
	pub fn with_auth_manager(mut self, auth_manager: AuthorizationManager) -> Self {
		self.auth_manager = auth_manager;
		self
	}

	/// Set the account for authorization checks.
	///
	/// If set and `check_authorization_before_upload` is enabled, the client will
	/// query authorization state before uploading and fail fast if insufficient.
	pub fn with_account(mut self, account: AccountId32) -> Self {
		self.account = Some(account);
		self
	}

	/// Store data on Bulletin Chain with default options.
	///
	/// Uses default CID codec (raw) and hash algorithm (blake2b-256).
	/// Automatically chunks data if it exceeds the configured threshold.
	///
	/// This handles the complete workflow:
	/// 1. Check authorization (if enabled)
	/// 2. Decide whether to chunk based on data size
	/// 3. Calculate CID(s)
	/// 4. Submit transaction(s)
	/// 5. Wait for finalization
	///
	/// # Arguments
	///
	/// * `data` - Data to store
	/// * `progress_callback` - Optional callback for progress tracking (only called for chunked
	///   uploads)
	pub async fn store(
		&self,
		data: Vec<u8>,
		progress_callback: Option<ProgressCallback>,
	) -> Result<StoreResult> {
		self.store_with_options(data, StoreOptions::default(), progress_callback).await
	}

	/// Store data on Bulletin Chain with custom options.
	///
	/// Allows specifying custom CID codec, hash algorithm, and finalization behavior.
	/// Automatically chunks data if it exceeds the configured threshold.
	///
	/// This handles the complete workflow:
	/// 1. Check authorization (if enabled)
	/// 2. Decide whether to chunk based on data size
	/// 3. Calculate CID(s)
	/// 4. Submit transaction(s)
	/// 5. Wait for finalization
	///
	/// # Arguments
	///
	/// * `data` - Data to store
	/// * `options` - Storage options (CID codec, hash algorithm)
	/// * `progress_callback` - Optional callback for progress tracking (only called for chunked
	///   uploads)
	pub async fn store_with_options(
		&self,
		data: Vec<u8>,
		options: StoreOptions,
		progress_callback: Option<ProgressCallback>,
	) -> Result<StoreResult> {
		if data.is_empty() {
			return Err(Error::EmptyData);
		}

		// Decide whether to chunk based on threshold
		if data.len() > self.config.chunking_threshold as usize {
			// Large data - use chunking
			self.store_internal_chunked(&data, None, options, progress_callback).await
		} else {
			// Small data - single transaction
			self.store_internal_single(data, options).await
		}
	}

	/// Internal: Store data in a single transaction (no chunking).
	async fn store_internal_single(
		&self,
		data: Vec<u8>,
		options: StoreOptions,
	) -> Result<StoreResult> {
		if data.is_empty() {
			return Err(Error::EmptyData);
		}

		// Check authorization before upload if enabled
		if self.config.check_authorization_before_upload {
			if let Some(account) = &self.account {
				// Query current authorization
				if let Some(auth) =
					self.submitter.query_account_authorization(account.clone()).await?
				{
					// Check if authorization has expired
					if let Some(expires_at) = auth.expires_at {
						if let Some(current_block) = self.submitter.query_current_block().await? {
							if expires_at <= current_block {
								return Err(Error::AuthorizationExpired {
									expired_at: expires_at,
									current_block,
								});
							}
						}
					}

					// Check if sufficient for this upload (1 transaction, data size)
					self.auth_manager.check_authorization(&auth, data.len() as u64, 1)?;
				}
				// If no authorization found, let it proceed - on-chain validation will catch it
			}
		}

		// Calculate CID
		let cid_data = crate::cid::calculate_cid_with_config(
			&data,
			options.cid_codec,
			options.hash_algorithm,
		)?;

		let cid_bytes = crate::cid::cid_to_bytes(&cid_data)?;

		// Submit transaction
		let receipt = self.submitter.submit_store(data.clone()).await?;

		Ok(StoreResult {
			cid: cid_bytes,
			size: data.len() as u64,
			block_number: receipt.block_number,
			chunks: None, // No chunking for single upload
		})
	}

	/// Internal: Store data with chunking (returns unified StoreResult).
	async fn store_internal_chunked(
		&self,
		data: &[u8],
		config: Option<ChunkerConfig>,
		options: StoreOptions,
		progress_callback: Option<ProgressCallback>,
	) -> Result<StoreResult> {
		if data.is_empty() {
			return Err(Error::EmptyData);
		}

		// Use provided config or default
		let chunker_config = config.unwrap_or(ChunkerConfig {
			chunk_size: self.config.default_chunk_size,
			max_parallel: self.config.max_parallel,
			create_manifest: self.config.create_manifest,
		});

		// Chunk the data
		let chunker = FixedSizeChunker::new(chunker_config.clone())?;
		let chunks = chunker.chunk(data)?;

		// Check authorization before upload if enabled
		if self.config.check_authorization_before_upload {
			if let Some(account) = &self.account {
				// Calculate requirements
				let (txs_needed, bytes_needed) = self.auth_manager.calculate_requirements(
					data.len() as u64,
					chunks.len(),
					chunker_config.create_manifest,
				);

				// Query current authorization
				if let Some(auth) =
					self.submitter.query_account_authorization(account.clone()).await?
				{
					// Check if authorization has expired
					if let Some(expires_at) = auth.expires_at {
						if let Some(current_block) = self.submitter.query_current_block().await? {
							if expires_at <= current_block {
								return Err(Error::AuthorizationExpired {
									expired_at: expires_at,
									current_block,
								});
							}
						}
					}

					// Check if sufficient
					self.auth_manager.check_authorization(&auth, bytes_needed, txs_needed)?;
				}
				// If no authorization found, let it proceed - on-chain validation will catch it
			}
		}

		let mut chunk_cids = Vec::new();
		let total_chunks = chunks.len();
		let mut last_block_number = None;

		// Submit each chunk
		for chunk in &chunks {
			if let Some(callback) = progress_callback {
				callback(ProgressEvent::ChunkStarted {
					index: chunk.index,
					total: chunk.total_chunks,
				});
			}

			// Calculate CID for this chunk
			let cid_data = crate::cid::calculate_cid_with_config(
				&chunk.data,
				options.cid_codec,
				options.hash_algorithm,
			)?;

			let cid_bytes = crate::cid::cid_to_bytes(&cid_data)?;

			// Submit chunk
			match self.submitter.submit_store(chunk.data.clone()).await {
				Ok(receipt) => {
					chunk_cids.push(cid_bytes.clone());
					last_block_number = receipt.block_number;

					if let Some(callback) = progress_callback {
						callback(ProgressEvent::ChunkCompleted {
							index: chunk.index,
							total: chunk.total_chunks,
							cid: cid_bytes,
						});
					}
				},
				Err(e) => {
					if let Some(callback) = progress_callback {
						callback(ProgressEvent::ChunkFailed {
							index: chunk.index,
							total: chunk.total_chunks,
							error: format!("{e:?}"),
						});
					}
					return Err(e);
				},
			}
		}

		// Optionally create and submit manifest
		let manifest_cid = if chunker_config.create_manifest {
			if let Some(callback) = progress_callback {
				callback(ProgressEvent::ManifestStarted);
			}

			let builder = UnixFsDagBuilder::new();
			let manifest = builder.build(&chunks, options.hash_algorithm)?;

			// Submit manifest
			let manifest_cid_bytes = crate::cid::cid_to_bytes(&manifest.root_cid)?;
			let receipt = self.submitter.submit_store(manifest.dag_bytes).await?;
			last_block_number = receipt.block_number;

			if let Some(callback) = progress_callback {
				callback(ProgressEvent::ManifestCreated { cid: manifest_cid_bytes.clone() });
			}

			Some(manifest_cid_bytes)
		} else {
			None
		};

		if let Some(callback) = progress_callback {
			callback(ProgressEvent::Completed { manifest_cid: manifest_cid.clone() });
		}

		// Return unified StoreResult
		Ok(StoreResult {
			cid: manifest_cid.clone().unwrap_or_else(|| chunk_cids[0].clone()),
			size: data.len() as u64,
			block_number: last_block_number,
			chunks: Some(ChunkDetails { chunk_cids, num_chunks: total_chunks as u32 }),
		})
	}

	/// Store large data with automatic chunking and manifest creation.
	///
	/// This handles the complete workflow:
	/// 1. Chunk the data
	/// 2. Check authorization (if enabled)
	/// 3. Calculate CIDs for each chunk
	/// 4. Submit each chunk as a separate transaction
	/// 5. Create and submit DAG-PB manifest (if enabled)
	/// 6. Return all CIDs and receipt information
	pub async fn store_chunked(
		&self,
		data: &[u8],
		config: Option<ChunkerConfig>,
		options: StoreOptions,
		progress_callback: Option<ProgressCallback>,
	) -> Result<ChunkedStoreResult> {
		if data.is_empty() {
			return Err(Error::EmptyData);
		}

		// Use provided config or default
		let chunker_config = config.unwrap_or(ChunkerConfig {
			chunk_size: self.config.default_chunk_size,
			max_parallel: self.config.max_parallel,
			create_manifest: self.config.create_manifest,
		});

		// Chunk the data
		let chunker = FixedSizeChunker::new(chunker_config.clone())?;
		let chunks = chunker.chunk(data)?;

		// Check authorization before upload if enabled
		if self.config.check_authorization_before_upload {
			if let Some(account) = &self.account {
				// Calculate requirements
				let (txs_needed, bytes_needed) = self.auth_manager.calculate_requirements(
					data.len() as u64,
					chunks.len(),
					chunker_config.create_manifest,
				);

				// Query current authorization
				if let Some(auth) =
					self.submitter.query_account_authorization(account.clone()).await?
				{
					// Check if authorization has expired
					if let Some(expires_at) = auth.expires_at {
						if let Some(current_block) = self.submitter.query_current_block().await? {
							if expires_at <= current_block {
								return Err(Error::AuthorizationExpired {
									expired_at: expires_at,
									current_block,
								});
							}
						}
					}

					// Check if sufficient
					self.auth_manager.check_authorization(&auth, bytes_needed, txs_needed)?;
				}
				// If no authorization found, let it proceed - on-chain validation will catch it
			}
		}

		let mut chunk_cids = Vec::new();
		let total_chunks = chunks.len();

		// Submit each chunk
		for chunk in &chunks {
			if let Some(callback) = progress_callback {
				callback(ProgressEvent::ChunkStarted {
					index: chunk.index,
					total: chunk.total_chunks,
				});
			}

			// Calculate CID for this chunk
			let cid_data = crate::cid::calculate_cid_with_config(
				&chunk.data,
				options.cid_codec,
				options.hash_algorithm,
			)?;

			let cid_bytes = crate::cid::cid_to_bytes(&cid_data)?;

			// Submit chunk
			match self.submitter.submit_store(chunk.data.clone()).await {
				Ok(_receipt) => {
					chunk_cids.push(cid_bytes.clone());

					if let Some(callback) = progress_callback {
						callback(ProgressEvent::ChunkCompleted {
							index: chunk.index,
							total: chunk.total_chunks,
							cid: cid_bytes,
						});
					}
				},
				Err(e) => {
					if let Some(callback) = progress_callback {
						callback(ProgressEvent::ChunkFailed {
							index: chunk.index,
							total: chunk.total_chunks,
							error: format!("{e:?}"),
						});
					}
					return Err(e);
				},
			}
		}

		// Optionally create and submit manifest
		let manifest_cid = if chunker_config.create_manifest {
			if let Some(callback) = progress_callback {
				callback(ProgressEvent::ManifestStarted);
			}

			let builder = UnixFsDagBuilder::new();
			let manifest = builder.build(&chunks, options.hash_algorithm)?;

			// Submit manifest
			let manifest_cid_bytes = crate::cid::cid_to_bytes(&manifest.root_cid)?;
			self.submitter.submit_store(manifest.dag_bytes).await?;

			if let Some(callback) = progress_callback {
				callback(ProgressEvent::ManifestCreated { cid: manifest_cid_bytes.clone() });
			}

			Some(manifest_cid_bytes)
		} else {
			None
		};

		if let Some(callback) = progress_callback {
			callback(ProgressEvent::Completed { manifest_cid: manifest_cid.clone() });
		}

		Ok(ChunkedStoreResult {
			chunk_cids,
			manifest_cid,
			total_size: data.len() as u64,
			num_chunks: total_chunks as u32,
		})
	}

	/// Authorize an account to store data.
	///
	/// Requires sudo/authorizer privileges.
	pub async fn authorize_account(
		&self,
		who: AccountId32,
		transactions: u32,
		bytes: u64,
	) -> Result<TransactionReceipt> {
		self.submitter.submit_authorize_account(who, transactions, bytes).await
	}

	/// Authorize a preimage (by content hash) to be stored.
	///
	/// Requires sudo/authorizer privileges.
	pub async fn authorize_preimage(
		&self,
		content_hash: [u8; 32],
		max_size: u64,
	) -> Result<TransactionReceipt> {
		self.submitter.submit_authorize_preimage(content_hash, max_size).await
	}

	/// Renew/extend retention period for stored data.
	pub async fn renew(&self, block: u32, index: u32) -> Result<TransactionReceipt> {
		self.submitter.submit_renew(block, index).await
	}

	/// Refresh an account authorization (extends expiry).
	///
	/// Requires sudo/authorizer privileges.
	pub async fn refresh_account_authorization(
		&self,
		who: AccountId32,
	) -> Result<TransactionReceipt> {
		self.submitter.submit_refresh_account_authorization(who).await
	}

	/// Refresh a preimage authorization (extends expiry).
	///
	/// Requires sudo/authorizer privileges.
	pub async fn refresh_preimage_authorization(
		&self,
		content_hash: [u8; 32],
	) -> Result<TransactionReceipt> {
		self.submitter.submit_refresh_preimage_authorization(content_hash).await
	}

	/// Remove an expired account authorization.
	pub async fn remove_expired_account_authorization(
		&self,
		who: AccountId32,
	) -> Result<TransactionReceipt> {
		self.submitter.submit_remove_expired_account_authorization(who).await
	}

	/// Remove an expired preimage authorization.
	pub async fn remove_expired_preimage_authorization(
		&self,
		content_hash: [u8; 32],
	) -> Result<TransactionReceipt> {
		self.submitter.submit_remove_expired_preimage_authorization(content_hash).await
	}

	/// Estimate authorization needed for storing data.
	pub fn estimate_authorization(&self, data_size: u64) -> (u32, u64) {
		self.auth_manager.estimate_authorization(data_size, self.config.create_manifest)
	}
}
