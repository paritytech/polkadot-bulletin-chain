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
	cid::ContentHash,
	submit::{TransactionReceipt, TransactionSubmitter},
	types::Result,
};
use alloc::vec::Vec;
use sp_runtime::AccountId32;

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
#[derive(Debug, Clone, Default)]
pub struct MockSubmitter {
	/// Counter for generating unique block numbers.
	block_counter: core::sync::atomic::AtomicU32,
	/// Whether to simulate failures.
	pub fail_submissions: bool,
}

impl MockSubmitter {
	/// Create a new mock submitter.
	pub fn new() -> Self {
		Self { block_counter: core::sync::atomic::AtomicU32::new(1), fail_submissions: false }
	}

	/// Create a mock submitter that fails all submissions.
	pub fn failing() -> Self {
		Self { block_counter: core::sync::atomic::AtomicU32::new(1), fail_submissions: true }
	}

	/// Generate a mock transaction receipt.
	fn generate_receipt(&self) -> TransactionReceipt {
		let block_number =
			self.block_counter.fetch_add(1, core::sync::atomic::Ordering::SeqCst);

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
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::async_client::AsyncBulletinClient;
	use crate::types::StoreOptions;

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
		let result = client.store(data, StoreOptions::default()).await;

		assert!(result.is_ok());
		let store_result = result.unwrap();
		assert_eq!(store_result.size, 17);
		assert_eq!(store_result.cid.len(), 36); // Blake2b-256 CID length
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
}
