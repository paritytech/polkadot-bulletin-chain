// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Data chunking utilities for splitting large files into smaller pieces.

use crate::types::{Chunk, ChunkerConfig, Error, Result};
use alloc::vec::Vec;

/// Maximum chunk size allowed (2 MiB).
///
/// This limit ensures IPFS Bitswap compatibility. The chain's MaxTransactionSize
/// is 8 MiB, but chunks larger than 2 MiB are not well-supported by Bitswap peers.
pub const MAX_CHUNK_SIZE: usize = 2 * 1024 * 1024;

/// Maximum file size allowed (64 MiB).
///
/// Files larger than this must be handled by the application directly.
pub const MAX_FILE_SIZE: usize = 64 * 1024 * 1024;

/// Default chunk size (1 MiB).
///
/// This provides a good balance between transaction overhead and throughput
/// for most use cases. Users can configure up to MAX_CHUNK_SIZE (2 MiB).
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
		Self { config: ChunkerConfig::default() }
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
		data_size.div_ceil(chunk_size)
	}
}

impl Chunker for FixedSizeChunker {
	fn chunk(&self, data: &[u8]) -> Result<Vec<Chunk>> {
		if data.is_empty() {
			return Err(Error::EmptyData);
		}
		if data.len() > MAX_FILE_SIZE {
			return Err(Error::FileTooLarge(data.len() as u64));
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
///
/// # Validation
///
/// This function validates that:
/// - Chunks are not empty
/// - All chunks have consistent `total_chunks` values (belong to same file)
/// - The number of chunks matches the expected `total_chunks`
/// - Chunk indices are sequential starting from 0
pub fn reassemble_chunks(chunks: &[Chunk]) -> Result<Vec<u8>> {
	if chunks.is_empty() {
		return Err(Error::EmptyData);
	}

	let expected_total = chunks[0].total_chunks;
	let actual_count = chunks.len() as u32;

	// Validate chunk count matches expected total
	if expected_total != actual_count {
		return Err(Error::ChunkingFailed(alloc::format!(
			"Chunk count mismatch: expected {expected_total} chunks, got {actual_count}",
		)));
	}

	// Validate all chunks belong to the same file and have sequential indices
	for (i, chunk) in chunks.iter().enumerate() {
		// Verify all chunks agree on total_chunks (same file)
		if chunk.total_chunks != expected_total {
			let actual = chunk.total_chunks;
			return Err(Error::ChunkingFailed(alloc::format!(
				"Chunk {i} has inconsistent total_chunks: expected {expected_total}, got {actual}",
			)));
		}

		// Verify sequential indices
		if chunk.index != i as u32 {
			let actual = chunk.index;
			return Err(Error::ChunkingFailed(alloc::format!(
				"Chunk index mismatch: expected {i}, got {actual}",
			)));
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
		let config = ChunkerConfig { chunk_size: 1024, max_parallel: 8, create_manifest: true };

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
		let config = ChunkerConfig { chunk_size: 1000, max_parallel: 8, create_manifest: true };

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		assert_eq!(chunks.len(), 3);
		assert_eq!(chunks[0].data.len(), 1000);
		assert_eq!(chunks[1].data.len(), 1000);
		assert_eq!(chunks[2].data.len(), 500);

		for (i, chunk) in chunks.iter().enumerate() {
			assert_eq!(chunk.index, i as u32);
			assert_eq!(chunk.total_chunks, 3);
		}
	}

	#[test]
	fn test_reassemble_chunks() {
		let original_data = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
		let config = ChunkerConfig { chunk_size: 3, max_parallel: 8, create_manifest: true };

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&original_data).unwrap();
		let reassembled = reassemble_chunks(&chunks).unwrap();

		assert_eq!(original_data, reassembled);
	}

	#[test]
	fn test_chunk_size_too_large() {
		let config = ChunkerConfig {
			chunk_size: (MAX_CHUNK_SIZE + 1) as u32,
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

	#[test]
	fn test_reassemble_chunk_count_mismatch() {
		use crate::types::Chunk;

		// Create chunks where total_chunks doesn't match actual count
		let chunks = vec![
			Chunk { data: vec![1, 2, 3], index: 0, total_chunks: 3 },
			Chunk { data: vec![4, 5, 6], index: 1, total_chunks: 3 },
			// Missing third chunk - only 2 chunks but total_chunks says 3
		];

		let result = reassemble_chunks(&chunks);
		assert!(result.is_err());
		assert!(result.unwrap_err().to_string().contains("Chunk count mismatch"));
	}

	#[test]
	fn test_reassemble_inconsistent_total_chunks() {
		use crate::types::Chunk;

		// Create chunks from different files (inconsistent total_chunks)
		let chunks = vec![
			Chunk { data: vec![1, 2, 3], index: 0, total_chunks: 2 },
			Chunk { data: vec![4, 5, 6], index: 1, total_chunks: 3 }, // Wrong total
		];

		let result = reassemble_chunks(&chunks);
		assert!(result.is_err());
		assert!(result.unwrap_err().to_string().contains("inconsistent total_chunks"));
	}

	#[test]
	fn test_reassemble_empty_chunks() {
		let chunks: Vec<Chunk> = vec![];
		let result = reassemble_chunks(&chunks);
		assert!(result.is_err());
	}

	// ==================== Large File Handling Tests ====================

	#[test]
	fn test_large_file_chunking_10mb() {
		// 10 MB file with 1 MiB chunks = 10 chunks
		let data = vec![0xABu8; 10 * 1024 * 1024];
		let config = ChunkerConfig {
			chunk_size: 1024 * 1024, // 1 MiB
			max_parallel: 8,
			create_manifest: true,
		};

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		assert_eq!(chunks.len(), 10);

		// Verify all chunks have correct metadata
		for (i, chunk) in chunks.iter().enumerate() {
			assert_eq!(chunk.index, i as u32);
			assert_eq!(chunk.total_chunks, 10);
			assert_eq!(chunk.data.len(), 1024 * 1024);
		}
	}

	#[test]
	fn test_large_file_chunking_64mb() {
		// 64 MiB file (MAX_FILE_SIZE) - verify chunk count and metadata
		let data_size = 64 * 1024 * 1024; // MAX_FILE_SIZE
		let chunk_size = 2 * 1024 * 1024; // MAX_CHUNK_SIZE
		let expected_chunks = 32;

		let config =
			ChunkerConfig { chunk_size: chunk_size as u32, max_parallel: 8, create_manifest: true };

		let chunker = FixedSizeChunker::new(config).unwrap();

		// Verify num_chunks calculation for max file size
		assert_eq!(chunker.num_chunks(data_size), expected_chunks);

		// Create actual data and chunk it (this tests memory handling)
		let data = vec![0xCDu8; data_size];
		let chunks = chunker.chunk(&data).unwrap();

		assert_eq!(chunks.len(), expected_chunks);

		// Verify first and last chunk metadata
		assert_eq!(chunks[0].index, 0);
		assert_eq!(chunks[0].total_chunks, expected_chunks as u32);
		assert_eq!(chunks[31].index, 31);
		assert_eq!(chunks[31].total_chunks, expected_chunks as u32);
	}

	#[test]
	fn test_file_too_large_error() {
		// 65 MiB file (> MAX_FILE_SIZE) should fail
		let data_size = 65 * 1024 * 1024;
		let data = vec![0xEEu8; data_size];

		let chunker = FixedSizeChunker::default_config();
		let result = chunker.chunk(&data);

		assert!(result.is_err());
		match result {
			Err(Error::FileTooLarge(size)) => assert_eq!(size, data_size as u64),
			_ => panic!("Expected FileTooLarge error"),
		}
	}

	#[test]
	fn test_large_file_reassembly_integrity() {
		// Create 5 MB file with varying data pattern
		let size = 5 * 1024 * 1024;
		let mut data = Vec::with_capacity(size);
		for i in 0..size {
			data.push((i % 256) as u8);
		}

		let config = ChunkerConfig {
			chunk_size: 512 * 1024, // 512 KiB chunks
			max_parallel: 8,
			create_manifest: true,
		};

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		// Should have 10 chunks (5 MiB / 512 KiB)
		assert_eq!(chunks.len(), 10);

		// Reassemble and verify integrity
		let reassembled = reassemble_chunks(&chunks).unwrap();
		assert_eq!(data.len(), reassembled.len());
		assert_eq!(data, reassembled);
	}

	#[test]
	fn test_large_file_partial_last_chunk() {
		// File size that doesn't divide evenly into chunks
		let size = 10 * 1024 * 1024 + 12345; // 10 MB + 12345 bytes
		let data = vec![0xEFu8; size];

		let config =
			ChunkerConfig { chunk_size: 1024 * 1024, max_parallel: 8, create_manifest: true };

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		// Should have 11 chunks
		assert_eq!(chunks.len(), 11);

		// Last chunk should have the remainder
		assert_eq!(chunks[10].data.len(), 12345);

		// Verify reassembly
		let reassembled = reassemble_chunks(&chunks).unwrap();
		assert_eq!(data, reassembled);
	}

	#[test]
	fn test_chunk_index_consistency_many_chunks() {
		// Many small chunks to test index handling
		let data = vec![0x42u8; 1000];
		let config = ChunkerConfig {
			chunk_size: 10, // Very small chunks
			max_parallel: 8,
			create_manifest: true,
		};

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		assert_eq!(chunks.len(), 100);

		// Verify sequential indices
		for (i, chunk) in chunks.iter().enumerate() {
			assert_eq!(chunk.index, i as u32, "Chunk {i} has wrong index");
			assert_eq!(chunk.total_chunks, 100, "Chunk {i} has wrong total_chunks");
		}
	}

	#[test]
	fn test_reassemble_out_of_order_chunks() {
		let original_data = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
		let config = ChunkerConfig { chunk_size: 3, max_parallel: 8, create_manifest: true };

		let chunker = FixedSizeChunker::new(config).unwrap();
		let mut chunks = chunker.chunk(&original_data).unwrap();

		// Shuffle chunks out of order
		chunks.swap(0, 2);
		chunks.swap(1, 3);

		// Sort back by index before reassembly
		chunks.sort_by_key(|c| c.index);

		let reassembled = reassemble_chunks(&chunks).unwrap();
		assert_eq!(original_data, reassembled);
	}

	#[test]
	fn test_chunk_boundary_exact_multiple() {
		// Data size exactly divisible by chunk size
		let chunk_size = 1000;
		let num_chunks = 5;
		let data = vec![0xAAu8; chunk_size * num_chunks];

		let config =
			ChunkerConfig { chunk_size: chunk_size as u32, max_parallel: 8, create_manifest: true };

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		assert_eq!(chunks.len(), num_chunks);

		// All chunks should be exactly chunk_size
		for chunk in &chunks {
			assert_eq!(chunk.data.len(), chunk_size);
		}
	}

	#[test]
	fn test_default_config_values() {
		let chunker = FixedSizeChunker::default_config();

		// Verify default chunk size is 1 MiB
		assert_eq!(chunker.num_chunks(1024 * 1024), 1);
		assert_eq!(chunker.num_chunks(1024 * 1024 + 1), 2);
	}

	#[test]
	fn test_max_chunk_size_boundary() {
		// Exactly at max chunk size should work
		let config = ChunkerConfig {
			chunk_size: MAX_CHUNK_SIZE as u32,
			max_parallel: 8,
			create_manifest: true,
		};

		let result = FixedSizeChunker::new(config);
		assert!(result.is_ok());

		// One byte over should fail
		let config_over = ChunkerConfig {
			chunk_size: MAX_CHUNK_SIZE as u32 + 1,
			max_parallel: 8,
			create_manifest: true,
		};

		let result_over = FixedSizeChunker::new(config_over);
		assert!(result_over.is_err());
	}
}
