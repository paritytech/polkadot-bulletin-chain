// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Mock transaction submitter for testing.
//!
//! This module provides a [`MockSubmitter`] that simulates transaction submission
//! without connecting to a real blockchain. Useful for:
//! - Unit testing
//! - Integration testing without a node
//! - Development and prototyping

use crate::{
	authorization::Authorization,
	cid::ContentHash,
	submit::{TransactionReceipt, TransactionSubmitter},
	types::Result,
};
use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use sp_runtime::AccountId32;
use std::sync::Mutex;

/// Mock transaction submitter that simulates blockchain interaction.
///
/// This submitter doesn't actually submit transactions to a blockchain.
/// Instead, it generates mock receipts for testing purposes.
///
/// # Example
///
/// ```
/// use bulletin_sdk_rust::submitters::MockSubmitter;
/// use bulletin_sdk_rust::async_client::AsyncBulletinClient;
/// use bulletin_sdk_rust::types::StoreOptions;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let submitter = MockSubmitter::new();
/// let client = AsyncBulletinClient::new(submitter);
///
/// let data = b"Hello, Bulletin!".to_vec();
/// let result = client.store(data, StoreOptions::default()).await?;
///
/// println!("Mock CID: {:?}", result.cid);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct MockSubmitter {
	/// Counter for generating unique block numbers.
	block_counter: core::sync::atomic::AtomicU32,
	/// Whether to simulate failures.
	pub fail_submissions: bool,
	/// Mock authorization storage (account -> authorization).
	account_authorizations: Arc<Mutex<BTreeMap<AccountId32, Authorization>>>,
	/// Mock authorization storage (content_hash -> authorization).
	preimage_authorizations: Arc<Mutex<BTreeMap<ContentHash, Authorization>>>,
}

impl Default for MockSubmitter {
	fn default() -> Self {
		Self::new()
	}
}

impl MockSubmitter {
	/// Create a new mock submitter.
	pub fn new() -> Self {
		Self {
			block_counter: core::sync::atomic::AtomicU32::new(1),
			fail_submissions: false,
			account_authorizations: Arc::new(Mutex::new(BTreeMap::new())),
			preimage_authorizations: Arc::new(Mutex::new(BTreeMap::new())),
		}
	}

	/// Create a mock submitter that fails all submissions.
	pub fn failing() -> Self {
		Self {
			block_counter: core::sync::atomic::AtomicU32::new(1),
			fail_submissions: true,
			account_authorizations: Arc::new(Mutex::new(BTreeMap::new())),
			preimage_authorizations: Arc::new(Mutex::new(BTreeMap::new())),
		}
	}

	/// Set mock authorization for an account (for testing).
	pub fn set_account_authorization(&self, account: AccountId32, auth: Authorization) {
		self.account_authorizations.lock().unwrap().insert(account, auth);
	}

	/// Set mock authorization for a preimage (for testing).
	pub fn set_preimage_authorization(&self, content_hash: ContentHash, auth: Authorization) {
		self.preimage_authorizations.lock().unwrap().insert(content_hash, auth);
	}

	/// Generate a mock transaction receipt.
	fn generate_receipt(&self) -> TransactionReceipt {
		let block_number = self.block_counter.fetch_add(1, core::sync::atomic::Ordering::SeqCst);

		TransactionReceipt {
			block_hash: alloc::format!("0xmock_block_{block_number}"),
			extrinsic_hash: alloc::format!("0xmock_extrinsic_{block_number}"),
			block_number: Some(block_number),
		}
	}
}

#[async_trait::async_trait]
impl TransactionSubmitter for MockSubmitter {
	async fn submit_store(&self, _data: Vec<u8>) -> Result<TransactionReceipt> {
		if self.fail_submissions {
			return Err(crate::types::Error::SubmissionFailed("Mock failure".into()));
		}
		Ok(self.generate_receipt())
	}

	async fn submit_authorize_account(
		&self,
		_who: AccountId32,
		_transactions: u32,
		_bytes: u64,
	) -> Result<TransactionReceipt> {
		if self.fail_submissions {
			return Err(crate::types::Error::SubmissionFailed("Mock failure".into()));
		}
		Ok(self.generate_receipt())
	}

	async fn submit_authorize_preimage(
		&self,
		_content_hash: ContentHash,
		_max_size: u64,
	) -> Result<TransactionReceipt> {
		if self.fail_submissions {
			return Err(crate::types::Error::SubmissionFailed("Mock failure".into()));
		}
		Ok(self.generate_receipt())
	}

	async fn submit_renew(&self, _block: u32, _index: u32) -> Result<TransactionReceipt> {
		if self.fail_submissions {
			return Err(crate::types::Error::SubmissionFailed("Mock failure".into()));
		}
		Ok(self.generate_receipt())
	}

	async fn submit_refresh_account_authorization(
		&self,
		_who: AccountId32,
	) -> Result<TransactionReceipt> {
		if self.fail_submissions {
			return Err(crate::types::Error::SubmissionFailed("Mock failure".into()));
		}
		Ok(self.generate_receipt())
	}

	async fn submit_refresh_preimage_authorization(
		&self,
		_content_hash: ContentHash,
	) -> Result<TransactionReceipt> {
		if self.fail_submissions {
			return Err(crate::types::Error::SubmissionFailed("Mock failure".into()));
		}
		Ok(self.generate_receipt())
	}

	async fn submit_remove_expired_account_authorization(
		&self,
		_who: AccountId32,
	) -> Result<TransactionReceipt> {
		if self.fail_submissions {
			return Err(crate::types::Error::SubmissionFailed("Mock failure".into()));
		}
		Ok(self.generate_receipt())
	}

	async fn submit_remove_expired_preimage_authorization(
		&self,
		_content_hash: ContentHash,
	) -> Result<TransactionReceipt> {
		if self.fail_submissions {
			return Err(crate::types::Error::SubmissionFailed("Mock failure".into()));
		}
		Ok(self.generate_receipt())
	}

	async fn query_account_authorization(&self, who: AccountId32) -> Result<Option<Authorization>> {
		Ok(self.account_authorizations.lock().unwrap().get(&who).cloned())
	}

	async fn query_preimage_authorization(
		&self,
		content_hash: ContentHash,
	) -> Result<Option<Authorization>> {
		Ok(self.preimage_authorizations.lock().unwrap().get(&content_hash).cloned())
	}

	async fn query_current_block(&self) -> Result<Option<u32>> {
		// Return the current block counter value
		Ok(Some(self.block_counter.load(core::sync::atomic::Ordering::SeqCst)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		async_client::AsyncBulletinClient,
		types::{AuthorizationScope, StoreOptions},
	};

	#[tokio::test]
	async fn test_mock_submitter_success() {
		let submitter = MockSubmitter::new();
		let receipt = submitter.submit_store(vec![1, 2, 3]).await.unwrap();

		assert!(receipt.block_hash.starts_with("0xmock_block_"));
		assert!(receipt.extrinsic_hash.starts_with("0xmock_extrinsic_"));
		assert_eq!(receipt.block_number, Some(1));
	}

	#[tokio::test]
	async fn test_mock_submitter_failure() {
		let submitter = MockSubmitter::failing();
		let result = submitter.submit_store(vec![1, 2, 3]).await;

		assert!(result.is_err());
	}

	#[tokio::test]
	async fn test_mock_submitter_with_client() {
		let submitter = MockSubmitter::new();
		let client = AsyncBulletinClient::new(submitter);

		let data = b"Hello, Bulletin!".to_vec();
		let result = client.store(data, StoreOptions::default(), None).await;

		assert!(result.is_ok());
		let store_result = result.unwrap();
		assert_eq!(store_result.size, 16); // "Hello, Bulletin!" is 16 bytes
		assert!(!store_result.cid.is_empty()); // CID should be present
	}

	#[tokio::test]
	async fn test_mock_submitter_increments_blocks() {
		let submitter = MockSubmitter::new();

		let receipt1 = submitter.submit_store(vec![1]).await.unwrap();
		let receipt2 = submitter.submit_store(vec![2]).await.unwrap();
		let receipt3 = submitter.submit_store(vec![3]).await.unwrap();

		assert_eq!(receipt1.block_number, Some(1));
		assert_eq!(receipt2.block_number, Some(2));
		assert_eq!(receipt3.block_number, Some(3));
	}

	#[tokio::test]
	async fn test_authorization_query() {
		let submitter = MockSubmitter::new();
		let account = AccountId32::from([1u8; 32]);

		// Initially no authorization
		let auth = submitter.query_account_authorization(account.clone()).await.unwrap();
		assert!(auth.is_none());

		// Set authorization
		let test_auth = Authorization {
			scope: AuthorizationScope::Account,
			transactions: 100,
			max_size: 1_000_000,
			expires_at: Some(1000),
		};
		submitter.set_account_authorization(account.clone(), test_auth.clone());

		// Query should return it
		let auth = submitter.query_account_authorization(account).await.unwrap();
		assert!(auth.is_some());
		let auth = auth.unwrap();
		assert_eq!(auth.transactions, 100);
		assert_eq!(auth.max_size, 1_000_000);
	}

	#[tokio::test]
	async fn test_authorization_check_with_client() {
		use crate::{async_client::AsyncClientConfig, types::StoreOptions};

		let submitter = MockSubmitter::new();
		let account = AccountId32::from([1u8; 32]);

		// Set authorization for 10 transactions and 10KB
		submitter.set_account_authorization(
			account.clone(),
			Authorization {
				scope: AuthorizationScope::Account,
				transactions: 10,
				max_size: 10_000,
				expires_at: None,
			},
		);

		// Create client with authorization checking enabled
		let config =
			AsyncClientConfig { check_authorization_before_upload: true, ..Default::default() };
		let client = AsyncBulletinClient::with_config(submitter, config).with_account(account);

		// Should succeed - 16 bytes is within limits
		let data = b"Hello, Bulletin!".to_vec();
		let result = client.store(data, StoreOptions::default(), None).await;
		assert!(result.is_ok());
	}

	#[tokio::test]
	async fn test_insufficient_authorization_fails() {
		use crate::{async_client::AsyncClientConfig, types::StoreOptions};

		let submitter = MockSubmitter::new();
		let account = AccountId32::from([1u8; 32]);

		// Set authorization for only 10 bytes
		submitter.set_account_authorization(
			account.clone(),
			Authorization {
				scope: AuthorizationScope::Account,
				transactions: 10,
				max_size: 10, // Only 10 bytes allowed
				expires_at: None,
			},
		);

		// Create client with authorization checking enabled
		let config =
			AsyncClientConfig { check_authorization_before_upload: true, ..Default::default() };
		let client = AsyncBulletinClient::with_config(submitter, config).with_account(account);

		// Should fail - 16 bytes exceeds limits
		let data = b"Hello, Bulletin!".to_vec();
		let result = client.store(data, StoreOptions::default(), None).await;
		assert!(result.is_err());
		assert!(matches!(
			result.unwrap_err(),
			crate::types::Error::InsufficientAuthorization { .. }
		));
	}

	#[tokio::test]
	async fn test_unified_api_small_data() {
		use crate::{async_client::AsyncClientConfig, types::StoreOptions};

		let submitter = MockSubmitter::new();
		let config =
			AsyncClientConfig { chunking_threshold: 2 * 1024 * 1024, ..Default::default() };
		let client = AsyncBulletinClient::with_config(submitter, config);

		// Small data (16 bytes < 2 MiB threshold) - should use single transaction
		let data = b"Hello, Bulletin!".to_vec();
		let result = client.store(data, StoreOptions::default(), None).await;

		assert!(result.is_ok());
		let store_result = result.unwrap();
		assert_eq!(store_result.size, 16);
		assert!(store_result.chunks.is_none()); // No chunking for small data
	}

	#[tokio::test]
	async fn test_unified_api_large_data() {
		use crate::{async_client::AsyncClientConfig, types::StoreOptions};

		let submitter = MockSubmitter::new();
		let config = AsyncClientConfig {
			chunking_threshold: 100,
			default_chunk_size: 50,
			..Default::default()
		};
		let client = AsyncBulletinClient::with_config(submitter, config);

		// Large data (150 bytes > 100 byte threshold) - should auto-chunk
		let data = vec![0u8; 150];
		let result = client.store(data, StoreOptions::default(), None).await;

		assert!(result.is_ok());
		let store_result = result.unwrap();
		assert_eq!(store_result.size, 150);
		assert!(store_result.chunks.is_some()); // Should have chunks
		let chunks = store_result.chunks.unwrap();
		assert_eq!(chunks.num_chunks, 3); // 150 bytes / 50 byte chunks = 3 chunks
		assert_eq!(chunks.chunk_cids.len(), 3);
	}

	#[tokio::test]
	async fn test_authorization_expiration() {
		use crate::{async_client::AsyncClientConfig, types::StoreOptions};

		let submitter = MockSubmitter::new();
		let account = AccountId32::from([1u8; 32]);

		// Advance the mock submitter's block counter to block 10
		for _ in 0..10 {
			submitter.submit_store(vec![1, 2, 3]).await.unwrap();
		}

		// Set authorization that expires at block 5 (already expired)
		submitter.set_account_authorization(
			account.clone(),
			Authorization {
				scope: AuthorizationScope::Account,
				transactions: 100,
				max_size: 10_000,
				expires_at: Some(5), // Expires at block 5 (current is 11)
			},
		);

		// Create client with authorization checking enabled
		let config =
			AsyncClientConfig { check_authorization_before_upload: true, ..Default::default() };
		let client = AsyncBulletinClient::with_config(submitter, config).with_account(account);

		// Should fail with expiration error - authorization expired at block 5, current is 11
		let data = b"Hello".to_vec();
		let result = client.store(data, StoreOptions::default(), None).await;

		assert!(result.is_err());
		assert!(matches!(result.unwrap_err(), crate::types::Error::AuthorizationExpired { .. }));
	}
}
