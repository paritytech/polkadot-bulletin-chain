// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Storage operations for submitting data to Bulletin Chain.
//!
//! This module provides helpers for building and submitting storage transactions.
//! The actual submission requires integration with `subxt` (enabled with `std` feature).

extern crate alloc;

use crate::{
	cid::{CidConfig, CidData},
	types::{Chunk, Error, Result, StoreOptions},
};
use alloc::vec::Vec;

/// Storage operation builder for creating transactions.
#[derive(Debug, Clone)]
pub struct StorageOperation {
	/// The data to store.
	pub data: Vec<u8>,
	/// CID configuration.
	pub cid_config: CidConfig,
	/// Whether to wait for finalization.
	/// Note: Currently unused - reserved for when `AsyncBulletinClient` transaction
	/// submission is fully implemented. For now, users submit via subxt directly.
	pub wait_finalization: bool,
}

impl StorageOperation {
	/// Create a new storage operation.
	///
	/// Returns an error if the hash algorithm is not supported.
	#[must_use = "storage operation must be submitted to the blockchain"]
	pub fn new(data: Vec<u8>, options: StoreOptions) -> Result<Self> {
		let cid_config =
			CidConfig { codec: options.cid_codec.code(), hashing: options.hash_algorithm };

		Ok(Self { data, cid_config, wait_finalization: options.wait_for_finalization })
	}

	/// Calculate the CID for this operation.
	#[must_use = "CID result should be used or stored"]
	pub fn calculate_cid(&self) -> Result<CidData> {
		crate::cid::calculate_cid(&self.data, self.cid_config.clone())
			.map_err(|_| Error::StorageFailed("Failed to calculate CID".into()))
	}

	/// Get the size of the data.
	pub fn size(&self) -> usize {
		self.data.len()
	}

	/// Validate the operation.
	pub fn validate(&self) -> Result<()> {
		if self.data.is_empty() {
			return Err(Error::EmptyData);
		}

		// Check if data exceeds max chunk size (2 MiB)
		const MAX_SIZE: usize = 2 * 1024 * 1024;
		if self.data.len() > MAX_SIZE {
			return Err(Error::ChunkTooLarge(self.data.len() as u64));
		}

		Ok(())
	}
}

/// Batch storage operations for submitting multiple chunks.
#[derive(Debug, Clone)]
pub struct BatchStorageOperation {
	/// Individual storage operations.
	pub operations: Vec<StorageOperation>,
	/// Whether to wait for finalization.
	/// Note: Currently unused - reserved for when `AsyncBulletinClient` transaction
	/// submission is fully implemented. For now, users submit via subxt directly.
	pub wait_finalization: bool,
}

impl BatchStorageOperation {
	/// Create a new batch operation.
	#[must_use = "batch operation must be submitted to the blockchain"]
	pub fn new(chunks: &[Chunk], options: StoreOptions) -> Result<Self> {
		let mut operations = Vec::with_capacity(chunks.len());

		for chunk in chunks {
			let op = StorageOperation::new(chunk.data.clone(), options.clone())?;
			op.validate()?;
			operations.push(op);
		}

		Ok(Self { operations, wait_finalization: options.wait_for_finalization })
	}

	/// Get the number of operations.
	pub fn len(&self) -> usize {
		self.operations.len()
	}

	/// Check if the batch is empty.
	pub fn is_empty(&self) -> bool {
		self.operations.is_empty()
	}

	/// Get total size of all operations.
	pub fn total_size(&self) -> usize {
		self.operations.iter().map(|op| op.size()).sum()
	}

	/// Calculate CIDs for all operations.
	#[must_use = "CID results should be used or stored"]
	pub fn calculate_cids(&self) -> Result<Vec<CidData>> {
		self.operations.iter().map(|op| op.calculate_cid()).collect()
	}
}

/// Helper functions for storage operations (requires std for subxt).
#[cfg(feature = "std")]
pub mod helpers {
	use super::*;

	/// Prepare transaction call data for `store` extrinsic.
	///
	/// Note: This is a helper that prepares the parameters.
	/// Actual transaction submission requires subxt integration.
	pub fn prepare_store_call(data: Vec<u8>) -> Vec<u8> {
		// The actual call building would be done with subxt
		// This is just a placeholder to show the structure
		data
	}

	/// Prepare batch transaction call data for multiple `store` calls.
	///
	/// Note: This uses `Utility.batch_all` to submit multiple transactions atomically.
	#[must_use = "call data must be submitted to the blockchain"]
	pub fn prepare_batch_store_calls(operations: &BatchStorageOperation) -> Result<Vec<Vec<u8>>> {
		let mut calls = Vec::with_capacity(operations.len());

		for op in &operations.operations {
			calls.push(prepare_store_call(op.data.clone()));
		}

		Ok(calls)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{cid::HashingAlgorithm, types::CidCodec};

	#[test]
	fn test_storage_operation_new() {
		let data = vec![1, 2, 3, 4, 5];
		let options = StoreOptions {
			cid_codec: CidCodec::Raw,
			hash_algorithm: HashingAlgorithm::Blake2b256,
			wait_for_finalization: false,
		};

		let op = StorageOperation::new(data.clone(), options).unwrap();
		assert_eq!(op.data, data);
		assert_eq!(op.size(), 5);
	}

	#[test]
	fn test_storage_operation_calculate_cid() {
		let data = vec![1, 2, 3, 4, 5];
		let options = StoreOptions::default();
		let op = StorageOperation::new(data, options).unwrap();

		let cid = op.calculate_cid();
		assert!(cid.is_ok());
	}

	#[test]
	fn test_storage_operation_validate_empty() {
		let data = vec![];
		let options = StoreOptions::default();
		let op = StorageOperation::new(data, options).unwrap();

		let result = op.validate();
		assert!(result.is_err());
	}

	#[test]
	fn test_storage_operation_validate_too_large() {
		let data = vec![0u8; 9 * 1024 * 1024]; // 9 MB
		let options = StoreOptions::default();
		let op = StorageOperation::new(data, options).unwrap();

		let result = op.validate();
		assert!(result.is_err());
	}

	#[test]
	fn test_batch_storage_operation() {
		use crate::{
			chunker::{Chunker, FixedSizeChunker},
			types::ChunkerConfig,
		};

		let data = vec![1u8; 5000];
		let config = ChunkerConfig { chunk_size: 2000, max_parallel: 8, create_manifest: true };

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		let options = StoreOptions::default();
		let batch = BatchStorageOperation::new(&chunks, options);
		assert!(batch.is_ok());

		let batch = batch.unwrap();
		assert_eq!(batch.len(), 3);
		assert_eq!(batch.total_size(), 5000);
	}

	#[test]
	fn test_batch_calculate_cids() {
		use crate::{
			chunker::{Chunker, FixedSizeChunker},
			types::ChunkerConfig,
		};

		let data = vec![1u8; 3000];
		let config = ChunkerConfig { chunk_size: 1000, max_parallel: 8, create_manifest: true };

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		let options = StoreOptions::default();
		let batch = BatchStorageOperation::new(&chunks, options).unwrap();
		let cids = batch.calculate_cids();

		assert!(cids.is_ok());
		assert_eq!(cids.unwrap().len(), 3);
	}
}
