// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Mock client for testing without a blockchain connection.
//!
//! This module provides a mock implementation of the Bulletin client that
//! doesn't require a running node. It's useful for:
//! - Unit testing application logic
//! - Integration tests without node setup
//! - Development and prototyping

#[cfg(feature = "std")]
use {
	crate::{
		authorization::AuthorizationManager,
		types::{
			CidCodec, Error, HashAlgorithm, ProgressCallback, Result, StoreOptions, StoreResult,
		},
	},
	alloc::{
		string::{String, ToString},
		vec::Vec,
	},
	sp_runtime::AccountId32,
};

/// Configuration for the mock Bulletin client.
#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub struct MockClientConfig {
	/// Default chunk size for large files (default: 1 MiB).
	pub default_chunk_size: u32,
	/// Maximum parallel uploads (default: 8).
	pub max_parallel: u32,
	/// Whether to create manifests for chunked uploads (default: true).
	pub create_manifest: bool,
	/// Check authorization before uploading to fail fast (default: true).
	pub check_authorization_before_upload: bool,
	/// Threshold for automatic chunking (default: 2 MiB).
	pub chunking_threshold: u32,
	/// Simulate authorization failures (for testing error paths).
	pub simulate_auth_failure: bool,
	/// Simulate storage failures (for testing error paths).
	pub simulate_storage_failure: bool,
}

#[cfg(feature = "std")]
impl Default for MockClientConfig {
	fn default() -> Self {
		Self {
			default_chunk_size: 1024 * 1024, // 1 MiB
			max_parallel: 8,
			create_manifest: true,
			check_authorization_before_upload: true,
			chunking_threshold: 2 * 1024 * 1024, // 2 MiB
			simulate_auth_failure: false,
			simulate_storage_failure: false,
		}
	}
}

/// Builder for mock store operations with fluent API.
#[cfg(feature = "std")]
pub struct MockStoreBuilder<'a> {
	client: &'a MockBulletinClient,
	data: Vec<u8>,
	options: StoreOptions,
	callback: Option<ProgressCallback>,
}

#[cfg(feature = "std")]
impl<'a> MockStoreBuilder<'a> {
	/// Create a new mock store builder.
	fn new(client: &'a MockBulletinClient, data: Vec<u8>) -> Self {
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

	/// Execute the mock store operation.
	pub async fn send(self) -> Result<StoreResult> {
		self.client.store_with_options(self.data, self.options, self.callback).await
	}
}

/// Mock Bulletin client for testing.
///
/// This client simulates blockchain operations without requiring a running node.
/// It calculates CIDs correctly and tracks operations but doesn't actually submit
/// transactions to a chain.
///
/// # Example
///
/// ```ignore
/// use bulletin_sdk_rust::MockBulletinClient;
///
/// // Create mock client
/// let client = MockBulletinClient::new();
///
/// // Store data (no blockchain required)
/// let result = client.store(data).send().await?;
/// println!("Mock CID: {:?}", result.cid);
///
/// // Check what operations were performed
/// let ops = client.operations();
/// assert_eq!(ops.len(), 1);
/// ```
#[cfg(feature = "std")]
pub struct MockBulletinClient {
	/// Client configuration.
	pub config: MockClientConfig,
	/// Authorization manager.
	pub auth_manager: AuthorizationManager,
	/// Account for authorization checks (optional).
	account: Option<AccountId32>,
	/// Operations performed (for testing verification).
	operations: std::sync::Arc<std::sync::Mutex<Vec<MockOperation>>>,
}

/// Record of a mock operation performed.
#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub enum MockOperation {
	/// Store operation with data size and CID.
	Store { data_size: usize, cid: String },
	/// Authorize account operation.
	AuthorizeAccount { who: AccountId32, transactions: u32, bytes: u64 },
	/// Authorize preimage operation.
	AuthorizePreimage { content_hash: Vec<u8>, max_size: u64 },
}

#[cfg(feature = "std")]
impl MockBulletinClient {
	/// Create a new mock client with default configuration.
	pub fn new() -> Self {
		Self {
			config: MockClientConfig::default(),
			auth_manager: AuthorizationManager::new(),
			account: None,
			operations: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
		}
	}

	/// Create a mock client with custom configuration.
	pub fn with_config(config: MockClientConfig) -> Self {
		Self {
			config,
			auth_manager: AuthorizationManager::new(),
			account: None,
			operations: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
		}
	}

	/// Set the authorization manager.
	pub fn with_auth_manager(mut self, auth_manager: AuthorizationManager) -> Self {
		self.auth_manager = auth_manager;
		self
	}

	/// Set the account for authorization checks.
	pub fn with_account(mut self, account: AccountId32) -> Self {
		self.account = Some(account);
		self
	}

	/// Get all operations performed by this client.
	pub fn operations(&self) -> Vec<MockOperation> {
		self.operations.lock().unwrap().clone()
	}

	/// Clear recorded operations.
	pub fn clear_operations(&self) {
		self.operations.lock().unwrap().clear();
	}

	/// Store data using builder pattern.
	pub fn store(&self, data: Vec<u8>) -> MockStoreBuilder {
		MockStoreBuilder::new(self, data)
	}

	/// Store data with custom options (internal, used by builder).
	pub async fn store_with_options(
		&self,
		data: Vec<u8>,
		_options: StoreOptions,
		_progress_callback: Option<ProgressCallback>,
	) -> Result<StoreResult> {
		if data.is_empty() {
			return Err(Error::EmptyData);
		}

		// Simulate authorization check failure
		if self.config.check_authorization_before_upload && self.config.simulate_auth_failure {
			return Err(Error::InsufficientAuthorization { need: 100, available: 0 });
		}

		// Simulate storage failure
		if self.config.simulate_storage_failure {
			return Err(Error::SubmissionFailed("Simulated storage failure".to_string()));
		}

		// Calculate CID (this is real, not mocked)
		let cid = crate::cid::calculate_cid_default(&data)?;
		let cid_bytes = crate::cid::cid_to_bytes(&cid)?;

		// Record the operation
		self.operations
			.lock()
			.unwrap()
			.push(MockOperation::Store { data_size: data.len(), cid: format!("{cid:?}") });

		// Return a mock receipt
		Ok(StoreResult {
			cid: cid_bytes.to_vec(),
			size: data.len() as u64,
			block_number: Some(1),
			chunks: None,
		})
	}

	/// Estimate authorization needed for data size.
	///
	/// Returns (transactions, bytes) needed.
	pub fn estimate_authorization(&self, data_size: u64) -> (u32, u64) {
		self.auth_manager.estimate_authorization(data_size, self.config.create_manifest)
	}
}

#[cfg(feature = "std")]
impl Default for MockBulletinClient {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn test_mock_store() {
		let client = MockBulletinClient::new();
		let data = b"Hello, Mock Bulletin!".to_vec();

		let result = client.store(data.clone()).send().await.unwrap();

		assert_eq!(result.size, data.len() as u64);
		assert_eq!(result.block_number, Some(1));

		let ops = client.operations();
		assert_eq!(ops.len(), 1);
		match &ops[0] {
			MockOperation::Store { data_size, .. } => {
				assert_eq!(*data_size, data.len());
			},
			_ => panic!("Expected Store operation"),
		}
	}

	#[tokio::test]
	async fn test_mock_empty_data() {
		let client = MockBulletinClient::new();
		let result = client.store(Vec::new()).send().await;

		assert!(result.is_err());
		assert!(matches!(result.unwrap_err(), Error::EmptyData));
	}

	#[tokio::test]
	async fn test_mock_auth_failure() {
		let config = MockClientConfig { simulate_auth_failure: true, ..Default::default() };

		let client = MockBulletinClient::with_config(config);
		let data = b"Test data".to_vec();

		let result = client.store(data).send().await;

		assert!(result.is_err());
		assert!(matches!(result.unwrap_err(), Error::InsufficientAuthorization { .. }));
	}

	#[tokio::test]
	async fn test_mock_storage_failure() {
		let config = MockClientConfig { simulate_storage_failure: true, ..Default::default() };

		let client = MockBulletinClient::with_config(config);
		let data = b"Test data".to_vec();

		let result = client.store(data).send().await;

		assert!(result.is_err());
		match result.unwrap_err() {
			Error::SubmissionFailed(msg) => {
				assert_eq!(msg, "Simulated storage failure");
			},
			_ => panic!("Expected SubmissionFailed error"),
		}
	}

	#[tokio::test]
	async fn test_mock_builder_pattern() {
		let client = MockBulletinClient::new();
		let data = b"Builder pattern test".to_vec();

		let result = client
			.store(data.clone())
			.with_codec(CidCodec::Raw)
			.with_hash_algorithm(HashAlgorithm::Blake2b256)
			.with_finalization(true)
			.send()
			.await
			.unwrap();

		assert_eq!(result.size, data.len() as u64);
	}

	#[tokio::test]
	async fn test_mock_clear_operations() {
		let client = MockBulletinClient::new();
		let data = b"Test".to_vec();

		client.store(data.clone()).send().await.unwrap();
		assert_eq!(client.operations().len(), 1);

		client.clear_operations();
		assert_eq!(client.operations().len(), 0);
	}
}
