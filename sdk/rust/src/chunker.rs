// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Data chunking utilities for splitting large files into smaller pieces.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use crate::types::{Chunk, ChunkerConfig, Error, Result};

/// Maximum chunk size allowed (8 MiB, matches pallet limit).
pub const MAX_CHUNK_SIZE: usize = 8 * 1024 * 1024;

/// Default chunk size (1 MiB).
pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

/// Trait for chunking strategies.
pub trait Chunker {
	/// Split data into chunks.
	fn chunk(&self, data: &[u8]) -> Result<Vec<Chunk>>;

	/// Validate chunk size.
	fn validate_chunk_size(&self, size: usize) -> Result<()> {
		if size == 0 {
			return Err(Error::InvalidConfig("Chunk size cannot be zero".into()));
		}
		if size > MAX_CHUNK_SIZE {
			return Err(Error::ChunkTooLarge(size as u64));
		}
		Ok(())
	}
}

/// Fixed-size chunker that splits data into equal-sized chunks.
#[derive(Debug, Clone)]
pub struct FixedSizeChunker {
	config: ChunkerConfig,
}

impl FixedSizeChunker {
	/// Create a new fixed-size chunker with the given configuration.
	pub fn new(config: ChunkerConfig) -> Result<Self> {
		if config.chunk_size == 0 {
			return Err(Error::InvalidConfig("Chunk size cannot be zero".into()));
		}
		if config.chunk_size > MAX_CHUNK_SIZE as u32 {
			return Err(Error::ChunkTooLarge(config.chunk_size as u64));
		}
		Ok(Self { config })
	}

	/// Create a chunker with default configuration.
	pub fn default_config() -> Self {
		Self {
			config: ChunkerConfig::default(),
		}
	}

	/// Get the chunk size.
	pub fn chunk_size(&self) -> usize {
		self.config.chunk_size as usize
	}

	/// Calculate the number of chunks needed for the given data size.
	pub fn num_chunks(&self, data_size: usize) -> usize {
		if data_size == 0 {
			return 0;
		}
		let chunk_size = self.config.chunk_size as usize;
		(data_size + chunk_size - 1) / chunk_size
	}
}

impl Chunker for FixedSizeChunker {
	fn chunk(&self, data: &[u8]) -> Result<Vec<Chunk>> {
		if data.is_empty() {
			return Err(Error::EmptyData);
		}

		let chunk_size = self.config.chunk_size as usize;
		let total_chunks = self.num_chunks(data.len()) as u32;
		let mut chunks = Vec::with_capacity(total_chunks as usize);

		for (index, chunk_data) in data.chunks(chunk_size).enumerate() {
			let chunk = Chunk::new(chunk_data.to_vec(), index as u32, total_chunks);
			chunks.push(chunk);
		}

		Ok(chunks)
	}
}

/// Reassemble chunks back into the original data.
pub fn reassemble_chunks(chunks: &[Chunk]) -> Result<Vec<u8>> {
	if chunks.is_empty() {
		return Err(Error::EmptyData);
	}

	// Validate chunk indices are sequential
	for (i, chunk) in chunks.iter().enumerate() {
		if chunk.index != i as u32 {
			return Err(Error::ChunkingFailed(
				alloc::format!("Chunk index mismatch: expected {}, got {}", i, chunk.index),
			));
		}
	}

	// Calculate total size
	let total_size: usize = chunks.iter().map(|c| c.data.len()).sum();
	let mut result = Vec::with_capacity(total_size);

	// Concatenate all chunks
	for chunk in chunks {
		result.extend_from_slice(&chunk.data);
	}

	Ok(result)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_fixed_size_chunker_single_chunk() {
		let data = vec![1u8; 100];
		let config = ChunkerConfig {
			chunk_size: 1024,
			max_parallel: 8,
			create_manifest: true,
		};

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		assert_eq!(chunks.len(), 1);
		assert_eq!(chunks[0].data.len(), 100);
		assert_eq!(chunks[0].index, 0);
		assert_eq!(chunks[0].total_chunks, 1);
	}

	#[test]
	fn test_fixed_size_chunker_multiple_chunks() {
		let data = vec![1u8; 2500];
		let config = ChunkerConfig {
			chunk_size: 1000,
			max_parallel: 8,
			create_manifest: true,
		};

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		assert_eq!(chunks.len(), 3);
		assert_eq!(chunks[0].data.len(), 1000);
		assert_eq!(chunks[1].data.len(), 1000);
		assert_eq!(chunks[2].data.len(), 500);

		for (i, chunk) in chunks.iter().enumerate() {
			assert_eq!(chunk.index, i);
			assert_eq!(chunk.total_chunks, 3);
		}
	}

	#[test]
	fn test_reassemble_chunks() {
		let original_data = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
		let config = ChunkerConfig {
			chunk_size: 3,
			max_parallel: 8,
			create_manifest: true,
		};

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&original_data).unwrap();
		let reassembled = reassemble_chunks(&chunks).unwrap();

		assert_eq!(original_data, reassembled);
	}

	#[test]
	fn test_chunk_size_too_large() {
		let config = ChunkerConfig {
			chunk_size: MAX_CHUNK_SIZE + 1,
			max_parallel: 8,
			create_manifest: true,
		};

		let result = FixedSizeChunker::new(config);
		assert!(result.is_err());
	}

	#[test]
	fn test_empty_data() {
		let data: Vec<u8> = vec![];
		let chunker = FixedSizeChunker::default_config();
		let result = chunker.chunk(&data);
		assert!(result.is_err());
	}

	#[test]
	fn test_num_chunks_calculation() {
		let chunker = FixedSizeChunker::default_config();

		assert_eq!(chunker.num_chunks(0), 0);
		assert_eq!(chunker.num_chunks(1024), 1);
		assert_eq!(chunker.num_chunks(1024 * 1024), 1);
		assert_eq!(chunker.num_chunks(1024 * 1024 + 1), 2);
		assert_eq!(chunker.num_chunks(1024 * 1024 * 2), 2);
	}
}
