// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Common types and error definitions for the Bulletin SDK.

extern crate alloc;

use alloc::{string::String, vec::Vec};
use codec::{Decode, Encode};
use scale_info::TypeInfo;

/// Result type for SDK operations.
pub type Result<T> = core::result::Result<T, Error>;

/// SDK error types.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum Error {
	/// Chunk size exceeds maximum allowed (8 MiB).
	#[cfg_attr(
		feature = "std",
		error("Chunk size {0} exceeds maximum allowed size of 8388608 bytes")
	)]
	ChunkTooLarge(u64),

	/// Data is empty.
	#[cfg_attr(feature = "std", error("Data cannot be empty"))]
	EmptyData,

	/// Invalid CID format.
	#[cfg_attr(feature = "std", error("Invalid CID: {0}"))]
	InvalidCid(String),

	/// Authorization not found for account.
	#[cfg_attr(feature = "std", error("Authorization not found for account: {0}"))]
	AuthorizationNotFound(String),

	/// Insufficient authorization.
	#[cfg_attr(
		feature = "std",
		error("Insufficient authorization: need {need} bytes, have {available} bytes")
	)]
	InsufficientAuthorization { need: u64, available: u64 },

	/// Storage operation failed.
	#[cfg_attr(feature = "std", error("Storage operation failed: {0}"))]
	StorageFailed(String),

	/// DAG-PB encoding failed.
	#[cfg_attr(feature = "std", error("DAG-PB encoding failed: {0}"))]
	DagEncodingFailed(String),

	/// Network error.
	#[cfg_attr(feature = "std", error("Network error: {0}"))]
	NetworkError(String),

	/// Invalid configuration.
	#[cfg_attr(feature = "std", error("Invalid configuration: {0}"))]
	InvalidConfig(String),

	/// Chunking failed.
	#[cfg_attr(feature = "std", error("Chunking failed: {0}"))]
	ChunkingFailed(String),

	/// Retrieval failed.
	#[cfg_attr(feature = "std", error("Retrieval failed: {0}"))]
	RetrievalFailed(String),

	/// Submission failed.
	#[cfg_attr(feature = "std", error("Submission failed: {0}"))]
	SubmissionFailed(String),
}

/// Configuration for chunking large data.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct ChunkerConfig {
	/// Size of each chunk in bytes (default: 1 MiB).
	pub chunk_size: u32,
	/// Maximum number of parallel uploads (default: 8).
	pub max_parallel: u32,
	/// Whether to create a DAG-PB manifest (default: true).
	pub create_manifest: bool,
}

impl Default for ChunkerConfig {
	fn default() -> Self {
		Self {
			chunk_size: 1024 * 1024, // 1 MiB
			max_parallel: 8,
			create_manifest: true,
		}
	}
}

/// A single chunk of data.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct Chunk {
	/// The chunk data.
	pub data: Vec<u8>,
	/// The CID of this chunk as bytes (calculated after encoding).
	#[codec(skip)]
	#[scale_info(skip_type_params(T))]
	pub cid: Option<Vec<u8>>,
	/// Index of this chunk in the sequence.
	pub index: u32,
	/// Total number of chunks.
	pub total_chunks: u32,
}

impl Chunk {
	/// Create a new chunk.
	pub fn new(data: Vec<u8>, index: u32, total_chunks: u32) -> Self {
		Self { data, cid: None, index, total_chunks }
	}

	/// Get the size of this chunk.
	pub fn size(&self) -> usize {
		self.data.len()
	}
}

/// Result of a storage operation.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct StoreResult {
	/// The CID of the stored data as bytes.
	pub cid: Vec<u8>,
	/// Size of the stored data in bytes.
	pub size: u64,
	/// Block number where data was stored.
	pub block_number: Option<u32>,
}

/// Result of a chunked storage operation.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct ChunkedStoreResult {
	/// CIDs of all stored chunks as bytes.
	pub chunk_cids: Vec<Vec<u8>>,
	/// The manifest CID (if manifest was created) as bytes.
	pub manifest_cid: Option<Vec<u8>>,
	/// Total size of all chunks in bytes.
	pub total_size: u64,
	/// Number of chunks.
	pub num_chunks: u32,
}

/// Options for storing data.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct StoreOptions {
	/// CID codec to use (default: raw).
	pub cid_codec: CidCodec,
	/// Hashing algorithm to use (default: blake2b-256).
	pub hash_algorithm: HashAlgorithm,
	/// Whether to wait for finalization (default: false).
	pub wait_for_finalization: bool,
}

impl Default for StoreOptions {
	fn default() -> Self {
		Self {
			cid_codec: CidCodec::Raw,
			hash_algorithm: HashAlgorithm::Blake2b256,
			wait_for_finalization: false,
		}
	}
}

/// CID codec types.
#[derive(Debug, Clone, Copy, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub enum CidCodec {
	/// Raw binary (0x55).
	Raw,
	/// DAG-PB (0x70).
	DagPb,
	/// DAG-CBOR (0x71).
	DagCbor,
}

impl CidCodec {
	/// Get the multicodec code.
	pub fn code(&self) -> u64 {
		match self {
			CidCodec::Raw => 0x55,
			CidCodec::DagPb => 0x70,
			CidCodec::DagCbor => 0x71,
		}
	}
}

/// Hash algorithm types.
#[derive(Debug, Clone, Copy, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub enum HashAlgorithm {
	/// BLAKE2b-256 (0xb220).
	Blake2b256,
	/// SHA2-256 (0x12).
	Sha2_256,
	/// SHA2-512 (0x13).
	Sha2_512,
	/// Keccak-256 (0x1b).
	Keccak256,
}

impl HashAlgorithm {
	/// Get the multihash code.
	pub fn code(&self) -> u64 {
		match self {
			HashAlgorithm::Blake2b256 => 0xb220,
			HashAlgorithm::Sha2_256 => 0x12,
			HashAlgorithm::Sha2_512 => 0x13,
			HashAlgorithm::Keccak256 => 0x1b,
		}
	}
}

/// Authorization scope types.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub enum AuthorizationScope {
	/// Account-based authorization (more flexible).
	Account,
	/// Preimage-based authorization (content-addressed).
	Preimage,
}

/// Progress event types.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
	/// A chunk upload has started.
	ChunkStarted { index: u32, total: u32 },
	/// A chunk upload has completed.
	ChunkCompleted { index: u32, total: u32, cid: Vec<u8> },
	/// A chunk upload has failed.
	ChunkFailed { index: u32, total: u32, error: String },
	/// Manifest creation started.
	ManifestStarted,
	/// Manifest has been created and stored.
	ManifestCreated { cid: Vec<u8> },
	/// All uploads completed successfully.
	Completed { manifest_cid: Option<Vec<u8>> },
}

/// Progress callback type.
pub type ProgressCallback = fn(ProgressEvent);
