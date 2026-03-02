// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Storage operations for submitting data to Bulletin Chain.
//!
//! This module provides helpers for building and submitting storage transactions.
//! The actual submission requires integration with `subxt` (enabled with `std` feature).

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
	/// Passed through to callers to decide wait behavior when submitting via subxt.
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
			.map_err(|e| Error::StorageFailed(alloc::format!("Failed to calculate CID: {e:?}")))
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

		if self.data.len() > crate::chunker::MAX_CHUNK_SIZE {
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
	/// Passed through to callers to decide wait behavior when submitting via subxt.
	pub wait_finalization: bool,
}

impl BatchStorageOperation {
	/// Create a new batch operation by borrowing chunk data.
	#[must_use = "batch operation must be submitted to the blockchain"]
	pub fn new(chunks: &[Chunk], options: StoreOptions) -> Result<Self> {
		Self::from_chunks(chunks.iter().map(|c| c.data.clone()).collect(), options)
	}

	/// Create a new batch operation by taking ownership of chunk data.
	///
	/// Avoids cloning the data when the caller no longer needs the chunks.
	#[must_use = "batch operation must be submitted to the blockchain"]
	pub fn from_chunks(chunk_data: Vec<Vec<u8>>, options: StoreOptions) -> Result<Self> {
		let cid_config =
			CidConfig { codec: options.cid_codec.code(), hashing: options.hash_algorithm };
		let mut operations = Vec::with_capacity(chunk_data.len());

		for data in chunk_data {
			let op = StorageOperation {
				data,
				cid_config: cid_config.clone(),
				wait_finalization: options.wait_for_finalization,
			};
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::cid::{CidCodec, HashingAlgorithm};

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
