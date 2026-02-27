// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! DAG-PB (Directed Acyclic Graph - Protocol Buffers) utilities.
//!
//! This module provides functionality to create IPFS-compatible DAG-PB
//! manifests for chunked data, following the UnixFS specification.

extern crate alloc;

/// Maximum number of chunks supported in a single DAG manifest.
///
/// This limit prevents excessive memory allocation when building manifests.
/// With 1 MiB chunks, this allows files up to ~1 TB. For larger files,
/// consider using a hierarchical DAG structure or streaming approach.
pub const MAX_MANIFEST_CHUNKS: usize = 1_000_000;

use crate::{
	cid::{calculate_cid, CidConfig, CidData, HashingAlgorithm},
	types::{Chunk, CidCodec, Error, Result},
};
use alloc::vec::Vec;

/// A DAG-PB manifest representing a file composed of multiple chunks.
#[derive(Debug, Clone)]
pub struct DagManifest {
	/// The root CID of the manifest.
	pub root_cid: CidData,
	/// CIDs of all chunks in order.
	pub chunk_cids: Vec<CidData>,
	/// Total size of the file in bytes.
	pub total_size: u64,
	/// Encoded DAG-PB bytes.
	pub dag_bytes: Vec<u8>,
}

/// Builder for creating DAG-PB manifests.
pub trait DagBuilder {
	/// Build a DAG-PB manifest from chunks.
	fn build(&self, chunks: &[Chunk], hash_algo: HashingAlgorithm) -> Result<DagManifest>;
}

/// UnixFS DAG-PB builder following IPFS UnixFS v1 specification.
#[derive(Debug, Clone, Default)]
pub struct UnixFsDagBuilder;

impl UnixFsDagBuilder {
	/// Create a new UnixFS DAG builder.
	pub fn new() -> Self {
		Self
	}

	/// Encode UnixFS file metadata.
	///
	/// UnixFS wire format (protobuf):
	/// ```proto
	/// message Data {
	///   enum DataType {
	///     Raw = 0;
	///     Directory = 1;
	///     File = 2;
	///     Metadata = 3;
	///     Symlink = 4;
	///     HAMTShard = 5;
	///   }
	///   required DataType Type = 1;
	///   optional bytes Data = 2;
	///   optional uint64 filesize = 3;
	///   repeated uint64 blocksizes = 4;
	///   optional uint64 hashType = 5;
	///   optional uint64 fanout = 6;
	/// }
	/// ```
	fn encode_unixfs_file_data(block_sizes: &[u64], total_size: u64) -> Vec<u8> {
		let mut buf = Vec::new();

		// Field 1: Type = 2 (File)
		// Wire format: field_number << 3 | wire_type
		// wire_type for varint = 0
		buf.push(1 << 3);
		buf.push(2); // File type

		// Field 3: filesize (optional)
		buf.push(3 << 3);
		encode_varint(total_size, &mut buf);

		// Field 4: blocksizes (repeated)
		for &size in block_sizes {
			buf.push(4 << 3);
			encode_varint(size, &mut buf);
		}

		buf
	}

	/// Encode a DAG-PB link.
	///
	/// DAG-PB wire format (protobuf):
	/// ```proto
	/// message PBLink {
	///   optional bytes Hash = 1;
	///   optional string Name = 2;
	///   optional uint64 Tsize = 3;
	/// }
	/// ```
	fn encode_dag_link(cid_bytes: &[u8], name: &str, tsize: u64) -> Vec<u8> {
		let mut buf = Vec::new();

		// Field 1: Hash (bytes)
		buf.push((1 << 3) | 2); // wire_type 2 = length-delimited
		encode_varint(cid_bytes.len() as u64, &mut buf);
		buf.extend_from_slice(cid_bytes);

		// Field 2: Name (string)
		if !name.is_empty() {
			buf.push((2 << 3) | 2);
			encode_varint(name.len() as u64, &mut buf);
			buf.extend_from_slice(name.as_bytes());
		}

		// Field 3: Tsize (uint64)
		buf.push(3 << 3); // wire_type 0 = varint
		encode_varint(tsize, &mut buf);

		buf
	}

	/// Encode a DAG-PB node.
	///
	/// DAG-PB wire format (protobuf):
	/// ```proto
	/// message PBNode {
	///   repeated PBLink Links = 2;
	///   optional bytes Data = 1;
	/// }
	/// ```
	///
	/// Note: The canonical encoding order is Links first, then Data. This ordering
	/// is required for IPFS compatibility due to a historical bug in the original
	/// protobuf encoder. See: <https://ipld.io/specs/codecs/dag-pb/spec/>
	fn encode_dag_node(links: &[Vec<u8>], data: &[u8]) -> Vec<u8> {
		let mut buf = Vec::new();

		// Field 2: Links (repeated) - MUST come first for canonical DAG-PB encoding
		for link in links {
			buf.push((2 << 3) | 2); // wire_type 2 = length-delimited
			encode_varint(link.len() as u64, &mut buf);
			buf.extend_from_slice(link);
		}

		// Field 1: Data (bytes) - comes second despite lower field number
		if !data.is_empty() {
			buf.push((1 << 3) | 2); // wire_type 2 = length-delimited
			encode_varint(data.len() as u64, &mut buf);
			buf.extend_from_slice(data);
		}

		buf
	}
}

impl DagBuilder for UnixFsDagBuilder {
	fn build(&self, chunks: &[Chunk], hash_algo: HashingAlgorithm) -> Result<DagManifest> {
		if chunks.is_empty() {
			return Err(Error::EmptyData);
		}

		// Prevent excessive memory allocation for very large files
		if chunks.len() > MAX_MANIFEST_CHUNKS {
			return Err(Error::DagEncodingFailed(alloc::format!(
				"Too many chunks ({}) for single manifest. Maximum is {}. \
				 Consider using hierarchical DAG structure for files this large.",
				chunks.len(),
				MAX_MANIFEST_CHUNKS
			)));
		}

		// Calculate CIDs for all chunks (using raw codec)
		let mut chunk_cids = Vec::with_capacity(chunks.len());
		let mut block_sizes = Vec::with_capacity(chunks.len());
		let mut total_size = 0u64;

		for chunk in chunks {
			let cid_config = CidConfig { codec: CidCodec::Raw.code(), hashing: hash_algo };

			let cid_data = calculate_cid(&chunk.data, cid_config)
				.map_err(|_| Error::DagEncodingFailed("Failed to calculate chunk CID".into()))?;

			let chunk_size = chunk.data.len() as u64;
			block_sizes.push(chunk_size);
			total_size += chunk_size;
			chunk_cids.push(cid_data);
		}

		// Encode UnixFS file metadata
		let unixfs_data = Self::encode_unixfs_file_data(&block_sizes, total_size);

		// Encode DAG-PB links for each chunk
		let mut links = Vec::with_capacity(chunks.len());
		for (i, cid_data) in chunk_cids.iter().enumerate() {
			let cid_bytes = cid_data
				.to_bytes()
				.ok_or_else(|| Error::DagEncodingFailed("Failed to serialize CID".into()))?;

			let link = Self::encode_dag_link(&cid_bytes, "", chunks[i].data.len() as u64);
			links.push(link);
		}

		// Encode DAG-PB node
		let dag_bytes = Self::encode_dag_node(&links, &unixfs_data);

		// Calculate root CID (using dag-pb codec)
		let root_config = CidConfig { codec: CidCodec::DagPb.code(), hashing: hash_algo };

		let root_cid = calculate_cid(&dag_bytes, root_config)
			.map_err(|_| Error::DagEncodingFailed("Failed to calculate root CID".into()))?;

		Ok(DagManifest { root_cid, chunk_cids, total_size, dag_bytes })
	}
}

/// Encode a varint (variable-length integer) using protobuf encoding.
fn encode_varint(mut value: u64, buf: &mut Vec<u8>) {
	loop {
		let mut byte = (value & 0x7F) as u8;
		value >>= 7;
		if value != 0 {
			byte |= 0x80;
		}
		buf.push(byte);
		if value == 0 {
			break;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		chunker::{Chunker, FixedSizeChunker},
		types::ChunkerConfig,
	};

	#[test]
	fn test_encode_varint() {
		let mut buf = Vec::new();
		encode_varint(0, &mut buf);
		assert_eq!(buf, vec![0]);

		let mut buf = Vec::new();
		encode_varint(127, &mut buf);
		assert_eq!(buf, vec![127]);

		let mut buf = Vec::new();
		encode_varint(128, &mut buf);
		assert_eq!(buf, vec![128, 1]);

		let mut buf = Vec::new();
		encode_varint(300, &mut buf);
		assert_eq!(buf, vec![172, 2]);
	}

	#[test]
	fn test_build_dag_manifest() {
		let data = vec![1u8; 5000];
		let config = ChunkerConfig { chunk_size: 2000, max_parallel: 8, create_manifest: true };

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		let builder = UnixFsDagBuilder::new();
		let manifest = builder.build(&chunks, HashingAlgorithm::Blake2b256).unwrap();

		assert_eq!(manifest.chunk_cids.len(), 3);
		assert_eq!(manifest.total_size, 5000);
		assert!(!manifest.dag_bytes.is_empty());
		assert_eq!(manifest.root_cid.codec, CidCodec::DagPb.code());
	}

	#[test]
	fn test_build_dag_manifest_single_chunk() {
		let data = vec![42u8; 100];
		let config = ChunkerConfig { chunk_size: 1000, max_parallel: 8, create_manifest: true };

		let chunker = FixedSizeChunker::new(config).unwrap();
		let chunks = chunker.chunk(&data).unwrap();

		let builder = UnixFsDagBuilder::new();
		let manifest = builder.build(&chunks, HashingAlgorithm::Blake2b256).unwrap();

		assert_eq!(manifest.chunk_cids.len(), 1);
		assert_eq!(manifest.total_size, 100);
	}

	#[test]
	fn test_build_dag_manifest_empty_chunks() {
		let chunks: Vec<Chunk> = vec![];
		let builder = UnixFsDagBuilder::new();
		let result = builder.build(&chunks, HashingAlgorithm::Blake2b256);
		assert!(result.is_err());
	}

	#[test]
	fn test_max_manifest_chunks_constant() {
		// Verify the constant is set to a reasonable value
		// With 1 MiB chunks, 1M chunks = ~1 TB max file size
		assert_eq!(MAX_MANIFEST_CHUNKS, 1_000_000);
	}
}
