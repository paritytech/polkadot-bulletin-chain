// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Async client with full transaction submission support.
//!
//! This module provides a complete client that handles both data preparation
//! and transaction submission to the Bulletin Chain using subxt.

#[cfg(feature = "std")]
use {
	crate::{
		authorization::AuthorizationManager,
		types::{
			ChunkerConfig, CidCodec, Error, HashAlgorithm, ProgressCallback, Result, StoreOptions,
			StoreResult,
		},
	},
	alloc::vec::Vec,
	sp_runtime::AccountId32,
	subxt::{OnlineClient, PolkadotConfig},
};

/// Configuration for the async Bulletin client.
#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
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

/// Builder for store operations with fluent API.
///
/// # Example
///
/// ```ignore
/// let result = client
///     .store(data)
///     .with_codec(CidCodec::DagPb)
///     .with_hash_algorithm(HashAlgorithm::Sha256)
///     .with_callback(|event| {
///         println!("Progress: {:?}", event);
///     })
///     .send()
///     .await?;
/// ```
#[cfg(feature = "std")]
pub struct StoreBuilder<'a> {
	client: &'a AsyncBulletinClient,
	data: Vec<u8>,
	options: StoreOptions,
	callback: Option<ProgressCallback>,
}

#[cfg(feature = "std")]
impl<'a> StoreBuilder<'a> {
	/// Create a new store builder.
	fn new(client: &'a AsyncBulletinClient, data: Vec<u8>) -> Self {
		Self { client, data, options: StoreOptions::default(), callback: None }
	}

	/// Set the CID codec.
	pub fn with_codec(mut self, codec: CidCodec) -> Self {
		self.options.cid_codec = codec;
		self
	}

	/// Set the hash algorithm.
	pub fn with_hash_algorithm(mut self, algorithm: HashAlgorithm) -> Self {
		self.options.hash_algorithm = algorithm;
		self
	}

	/// Set whether to wait for finalization.
	pub fn with_finalization(mut self, wait: bool) -> Self {
		self.options.wait_for_finalization = wait;
		self
	}

	/// Set custom store options.
	pub fn with_options(mut self, options: StoreOptions) -> Self {
		self.options = options;
		self
	}

	/// Set progress callback for chunked uploads.
	pub fn with_callback(mut self, callback: ProgressCallback) -> Self {
		self.callback = Some(callback);
		self
	}

	/// Execute the store operation.
	///
	/// This consumes the builder and performs the actual storage operation.
	pub async fn send(self) -> Result<StoreResult> {
		self.client.store_with_options(self.data, self.options, self.callback).await
	}
}

/// Async Bulletin client that submits transactions to the chain.
///
/// This client is tightly coupled to subxt for blockchain interaction.
/// Users must provide a configured `OnlineClient` with the appropriate
/// runtime metadata and configuration for their Bulletin Chain node.
///
/// # Example
///
/// ```ignore
/// use subxt::{OnlineClient, PolkadotConfig};
/// use bulletin_sdk_rust::AsyncBulletinClient;
///
/// // User sets up subxt with their metadata
/// let api = OnlineClient::<PolkadotConfig>::from_url("ws://localhost:9944").await?;
///
/// // Create SDK client
/// let client = AsyncBulletinClient::new(api);
///
/// // Store data
/// let result = client.store(data).send().await?;
/// ```
#[cfg(feature = "std")]
pub struct AsyncBulletinClient {
	/// Subxt client for blockchain interaction.
	pub api: OnlineClient<PolkadotConfig>,
	/// Client configuration.
	pub config: AsyncClientConfig,
	/// Authorization manager.
	pub auth_manager: AuthorizationManager,
	/// Account for authorization checks (optional).
	account: Option<AccountId32>,
}

#[cfg(feature = "std")]
impl AsyncBulletinClient {
	/// Create a new async client with the given subxt client.
	///
	/// The subxt client must be configured with the correct runtime metadata
	/// for your Bulletin Chain node.
	pub fn new(api: OnlineClient<PolkadotConfig>) -> Self {
		Self {
			api,
			config: AsyncClientConfig::default(),
			auth_manager: AuthorizationManager::new(),
			account: None,
		}
	}

	/// Create a client with custom configuration.
	pub fn with_config(api: OnlineClient<PolkadotConfig>, config: AsyncClientConfig) -> Self {
		Self { api, config, auth_manager: AuthorizationManager::new(), account: None }
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

	/// Store data on Bulletin Chain using builder pattern.
	///
	/// Returns a builder that allows fluent configuration of store options.
	///
	/// # Example
	///
	/// ```ignore
	/// let result = client
	///     .store(data)
	///     .with_codec(CidCodec::DagPb)
	///     .with_hash_algorithm(HashAlgorithm::Sha256)
	///     .with_callback(|event| {
	///         println!("Progress: {:?}", event);
	///     })
	///     .send()
	///     .await?;
	/// ```
	pub fn store(&self, data: Vec<u8>) -> StoreBuilder<'_> {
		StoreBuilder::new(self, data)
	}

	/// Store data on Bulletin Chain with custom options (internal, used by builder).
	///
	/// **Note**: This method is public for use by the builder but users should prefer
	/// the builder pattern via `store()`.
	///
	/// Automatically chunks data if it exceeds the configured threshold.
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
		_options: StoreOptions,
	) -> Result<StoreResult> {
		if data.is_empty() {
			return Err(Error::EmptyData);
		}

		// TODO: Check authorization before upload if enabled
		// TODO: Calculate CID
		// TODO: Submit transaction via self.api
		// TODO: Wait for finalization
		// TODO: Return receipt

		// Placeholder implementation
		Err(Error::SubmissionFailed(
			"Direct subxt integration not yet implemented - requires runtime metadata codegen"
				.to_string(),
		))
	}

	/// Internal: Store data with chunking.
	async fn store_internal_chunked(
		&self,
		_data: &[u8],
		_chunker_config: Option<ChunkerConfig>,
		_options: StoreOptions,
		_progress_callback: Option<ProgressCallback>,
	) -> Result<StoreResult> {
		// Placeholder implementation
		Err(Error::SubmissionFailed(
			"Chunked upload not yet implemented for direct subxt integration".to_string(),
		))
	}

	/// Estimate authorization needed for data size.
	///
	/// Returns (transactions, bytes) needed.
	pub fn estimate_authorization(&self, data_size: u64) -> (u32, u64) {
		self.auth_manager.estimate_authorization(data_size, self.config.create_manifest)
	}
}

// Note: Authorization methods (authorize_account, etc.) would need to be implemented
// once we have the metadata codegen pattern figured out for the SDK.
// For now, users should use the subxt client directly for authorization operations.
