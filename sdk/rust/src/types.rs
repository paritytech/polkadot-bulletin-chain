// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Common types and error definitions for the Bulletin SDK.

use crate::cid::{CidCodec, HashingAlgorithm};
use alloc::{string::String, vec::Vec};
use codec::{Decode, Encode};
use scale_info::TypeInfo;

/// Result type for SDK operations.
pub type Result<T> = core::result::Result<T, Error>;

/// SDK error types.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum Error {
	/// Chunk size exceeds maximum allowed (2 MiB).
	#[cfg_attr(
		feature = "std",
		error("Chunk size {0} exceeds maximum allowed size of 2097152 bytes (2 MiB)")
	)]
	ChunkTooLarge(u64),

	/// File size exceeds maximum allowed (64 MiB).
	#[cfg_attr(
		feature = "std",
		error("File size {0} exceeds maximum allowed size of 67108864 bytes (64 MiB)")
	)]
	FileTooLarge(u64),

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

	/// Authorization has expired.
	#[cfg_attr(
		feature = "std",
		error("Authorization expired at block {expired_at} (current block: {current_block})")
	)]
	AuthorizationExpired { expired_at: u32, current_block: u32 },

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

	/// Renewal target not found.
	#[cfg_attr(feature = "std", error("Renewal target not found: block {block}, index {index}"))]
	RenewalNotFound { block: u32, index: u32 },

	/// Renewal failed.
	#[cfg_attr(feature = "std", error("Renewal failed: {0}"))]
	RenewalFailed(String),

	/// CID calculation failed.
	#[cfg_attr(feature = "std", error("CID calculation failed: {0}"))]
	CidCalculationFailed(String),

	/// On-chain transaction failed (e.g., invalid, dropped, or error).
	#[cfg_attr(feature = "std", error("Transaction failed: {0}"))]
	TransactionFailed(String),

	/// Invalid chunk size.
	#[cfg_attr(feature = "std", error("Invalid chunk size: {0}"))]
	InvalidChunkSize(String),
}

impl Error {
	/// Returns a `SCREAMING_SNAKE_CASE` error code matching the TypeScript SDK.
	pub fn code(&self) -> &'static str {
		match self {
			Error::ChunkTooLarge(_) => "CHUNK_TOO_LARGE",
			Error::FileTooLarge(_) => "FILE_TOO_LARGE",
			Error::EmptyData => "EMPTY_DATA",
			Error::InvalidCid(_) => "INVALID_CID",
			Error::AuthorizationNotFound(_) => "AUTHORIZATION_NOT_FOUND",
			Error::InsufficientAuthorization { .. } => "INSUFFICIENT_AUTHORIZATION",
			Error::AuthorizationExpired { .. } => "AUTHORIZATION_EXPIRED",
			Error::StorageFailed(_) => "STORAGE_FAILED",
			Error::DagEncodingFailed(_) => "DAG_ENCODING_FAILED",
			Error::NetworkError(_) => "NETWORK_ERROR",
			Error::InvalidConfig(_) => "INVALID_CONFIG",
			Error::ChunkingFailed(_) => "CHUNKING_FAILED",
			Error::RetrievalFailed(_) => "RETRIEVAL_FAILED",
			Error::RenewalNotFound { .. } => "RENEWAL_NOT_FOUND",
			Error::RenewalFailed(_) => "RENEWAL_FAILED",
			Error::CidCalculationFailed(_) => "CID_CALCULATION_FAILED",
			Error::TransactionFailed(_) => "TRANSACTION_FAILED",
			Error::InvalidChunkSize(_) => "INVALID_CHUNK_SIZE",
		}
	}

	/// Returns `true` if this error is likely transient and retrying may succeed.
	pub fn is_retryable(&self) -> bool {
		matches!(
			self,
			Error::AuthorizationExpired { .. } |
				Error::NetworkError(_) |
				Error::StorageFailed(_) |
				Error::TransactionFailed(_) |
				Error::RetrievalFailed(_) |
				Error::RenewalFailed(_)
		)
	}

	/// Returns an actionable recovery suggestion for this error.
	pub fn recovery_hint(&self) -> &'static str {
		match self {
			Error::ChunkTooLarge(_) => "Reduce chunk size to 2 MiB or less",
			Error::FileTooLarge(_) => "Reduce file size or use chunked upload",
			Error::EmptyData => "Provide non-empty data",
			Error::InvalidCid(_) => "Verify CID format",
			Error::AuthorizationNotFound(_) =>
				"Call authorizeAccount() or authorizePreimage() first",
			Error::InsufficientAuthorization { .. } => "Request additional authorization",
			Error::AuthorizationExpired { .. } =>
				"Call refreshAccountAuthorization() to extend expiry",
			Error::StorageFailed(_) => "Check node connectivity and try again",
			Error::DagEncodingFailed(_) => "Check chunk CIDs and data integrity",
			Error::NetworkError(_) => "Check network connectivity to the RPC endpoint",
			Error::InvalidConfig(_) => "Check configuration parameters",
			Error::ChunkingFailed(_) => "Verify data integrity and chunker configuration",
			Error::RetrievalFailed(_) => "The data may not be available yet; try again",
			Error::RenewalNotFound { .. } => "Verify the block number and extrinsic index",
			Error::RenewalFailed(_) => "Check that storage hasn't expired, then retry",
			Error::CidCalculationFailed(_) => "Verify data and hash algorithm",
			Error::TransactionFailed(_) => "Verify transaction parameters and account nonce",
			Error::InvalidChunkSize(_) => "Use a chunk size between 1 byte and 8 MiB",
		}
	}
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
	/// Index of this chunk in the sequence.
	pub index: u32,
	/// Total number of chunks.
	pub total_chunks: u32,
}

impl Chunk {
	/// Create a new chunk.
	pub fn new(data: Vec<u8>, index: u32, total_chunks: u32) -> Self {
		Self { data, index, total_chunks }
	}

	/// Get the size of this chunk.
	pub fn size(&self) -> usize {
		self.data.len()
	}
}

/// Result of a storage operation.
///
/// This result type works for both single-transaction uploads and chunked uploads.
/// For chunked uploads, the `cid` field contains the manifest CID, and `chunks`
/// contains details about the individual chunks.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct StoreResult {
	/// The primary CID of the stored data as bytes.
	/// - For single uploads: CID of the data
	/// - For chunked uploads: CID of the manifest
	pub cid: Vec<u8>,
	/// Total size of the stored data in bytes.
	pub size: u64,
	/// Block number where data was stored (or last chunk was stored).
	pub block_number: Option<u32>,
	/// Chunk details (only present for chunked uploads).
	pub chunks: Option<ChunkDetails>,
}

/// Details about chunks in a chunked upload.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct ChunkDetails {
	/// CIDs of all stored chunks as bytes.
	pub chunk_cids: Vec<Vec<u8>>,
	/// Number of chunks.
	pub num_chunks: u32,
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

/// Reference to a stored transaction for renewal.
///
/// This identifies a previous `store` or `renew` transaction that can be renewed.
#[derive(Debug, Clone, Copy, Encode, Decode, TypeInfo)]
pub struct StorageRef {
	/// Block number where the data was stored or last renewed.
	pub block: u32,
	/// Transaction index within that block (from `Stored` or `Renewed` event).
	pub index: u32,
}

impl StorageRef {
	/// Create a new storage reference.
	pub fn new(block: u32, index: u32) -> Self {
		Self { block, index }
	}
}

/// Result of a renewal operation.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct RenewalResult {
	/// The new storage reference (for the next renewal).
	pub new_ref: StorageRef,
	/// Content hash of the renewed data.
	pub content_hash: Vec<u8>,
	/// Size of the renewed data.
	pub size: u64,
}

/// Options for storing data.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct StoreOptions {
	/// CID codec to use (default: raw).
	pub cid_codec: CidCodec,
	/// Hashing algorithm to use (default: blake2b-256).
	pub hash_algorithm: HashingAlgorithm,
	/// Whether to wait for finalization (default: false).
	pub wait_for_finalization: bool,
}

impl Default for StoreOptions {
	fn default() -> Self {
		Self {
			cid_codec: CidCodec::Raw,
			hash_algorithm: HashingAlgorithm::Blake2b256,
			wait_for_finalization: false,
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

/// Progress event types for chunked uploads.
#[derive(Debug, Clone)]
pub enum ChunkProgressEvent {
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

/// Transaction status event types (mirrors subxt's TxStatus).
#[derive(Debug, Clone)]
pub enum TransactionStatusEvent {
	/// Transaction has been validated and added to the transaction pool.
	Validated,
	/// Transaction has been broadcast to peers.
	Broadcasted,
	/// Transaction is now in a best block.
	InBestBlock { block_hash: String, block_number: Option<u32>, extrinsic_index: Option<u32> },
	/// Transaction has been finalized.
	Finalized { block_hash: String, block_number: Option<u32>, extrinsic_index: Option<u32> },
	/// Transaction was in a block that got reorganized.
	NoLongerInBestBlock,
	/// Transaction is not valid anymore (e.g., nonce too low).
	Invalid { error: String },
	/// Transaction was dropped from the pool.
	Dropped { error: String },
}

impl TransactionStatusEvent {
	/// Returns a human-readable description of this transaction status event.
	pub fn description(&self) -> String {
		match self {
			TransactionStatusEvent::Validated =>
				"Transaction validated and added to the pool".into(),
			TransactionStatusEvent::Broadcasted => "Transaction broadcast to peers".into(),
			TransactionStatusEvent::InBestBlock { block_hash, block_number, .. } =>
				match block_number {
					Some(n) => alloc::format!("Transaction in best block #{n} ({block_hash})"),
					None => alloc::format!("Transaction in best block ({block_hash})"),
				},
			TransactionStatusEvent::Finalized { block_hash, block_number, .. } =>
				match block_number {
					Some(n) => alloc::format!("Transaction finalized in block #{n} ({block_hash})"),
					None => alloc::format!("Transaction finalized in block ({block_hash})"),
				},
			TransactionStatusEvent::NoLongerInBestBlock =>
				"Transaction no longer in best block (reorg occurred)".into(),
			TransactionStatusEvent::Invalid { error } =>
				alloc::format!("Transaction invalid: {error}"),
			TransactionStatusEvent::Dropped { error } =>
				alloc::format!("Transaction dropped from pool: {error}"),
		}
	}
}

/// Combined progress event types.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
	/// Chunk upload progress event.
	Chunk(ChunkProgressEvent),
	/// Transaction status event.
	Transaction(TransactionStatusEvent),
}

// Convenience constructors for backward compatibility
impl ProgressEvent {
	/// Create a ChunkStarted event.
	pub fn chunk_started(index: u32, total: u32) -> Self {
		ProgressEvent::Chunk(ChunkProgressEvent::ChunkStarted { index, total })
	}

	/// Create a ChunkCompleted event.
	pub fn chunk_completed(index: u32, total: u32, cid: Vec<u8>) -> Self {
		ProgressEvent::Chunk(ChunkProgressEvent::ChunkCompleted { index, total, cid })
	}

	/// Create a ChunkFailed event.
	pub fn chunk_failed(index: u32, total: u32, error: String) -> Self {
		ProgressEvent::Chunk(ChunkProgressEvent::ChunkFailed { index, total, error })
	}

	/// Create a ManifestStarted event.
	pub fn manifest_started() -> Self {
		ProgressEvent::Chunk(ChunkProgressEvent::ManifestStarted)
	}

	/// Create a ManifestCreated event.
	pub fn manifest_created(cid: Vec<u8>) -> Self {
		ProgressEvent::Chunk(ChunkProgressEvent::ManifestCreated { cid })
	}

	/// Create a Completed event.
	pub fn completed(manifest_cid: Option<Vec<u8>>) -> Self {
		ProgressEvent::Chunk(ChunkProgressEvent::Completed { manifest_cid })
	}

	/// Create a Validated transaction event.
	pub fn tx_validated() -> Self {
		ProgressEvent::Transaction(TransactionStatusEvent::Validated)
	}

	/// Create a Broadcasted transaction event.
	pub fn tx_broadcasted() -> Self {
		ProgressEvent::Transaction(TransactionStatusEvent::Broadcasted)
	}

	/// Create an InBestBlock transaction event.
	pub fn tx_in_best_block(
		block_hash: String,
		block_number: Option<u32>,
		extrinsic_index: Option<u32>,
	) -> Self {
		ProgressEvent::Transaction(TransactionStatusEvent::InBestBlock {
			block_hash,
			block_number,
			extrinsic_index,
		})
	}

	/// Create a Finalized transaction event.
	pub fn tx_finalized(
		block_hash: String,
		block_number: Option<u32>,
		extrinsic_index: Option<u32>,
	) -> Self {
		ProgressEvent::Transaction(TransactionStatusEvent::Finalized {
			block_hash,
			block_number,
			extrinsic_index,
		})
	}
}

/// Progress callback type.
///
/// Uses `Arc<dyn Fn>` to allow closures with captured state while remaining
/// cloneable and thread-safe. This enables patterns like:
///
/// ```ignore
/// use std::sync::Arc;
/// use bulletin_sdk_rust::ProgressCallback;
///
/// let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
/// let counter_clone = counter.clone();
///
/// let callback: ProgressCallback = Arc::new(move |event| {
///     counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
///     println!("Progress: {:?}", event);
/// });
/// ```
pub type ProgressCallback = alloc::sync::Arc<dyn Fn(ProgressEvent) + Send + Sync>;

#[cfg(test)]
mod tests {
	use super::*;

	/// All error variants for exhaustive testing.
	fn all_errors() -> Vec<Error> {
		vec![
			Error::ChunkTooLarge(100),
			Error::FileTooLarge(200),
			Error::EmptyData,
			Error::InvalidCid("bad".into()),
			Error::AuthorizationNotFound("acct".into()),
			Error::InsufficientAuthorization { need: 10, available: 5 },
			Error::AuthorizationExpired { expired_at: 1, current_block: 2 },
			Error::StorageFailed("fail".into()),
			Error::DagEncodingFailed("bad dag".into()),
			Error::NetworkError("timeout".into()),
			Error::InvalidConfig("bad config".into()),
			Error::ChunkingFailed("chunk err".into()),
			Error::RetrievalFailed("not found".into()),
			Error::RenewalNotFound { block: 1, index: 0 },
			Error::RenewalFailed("renew err".into()),
			Error::CidCalculationFailed("calc fail".into()),
			Error::TransactionFailed("tx fail".into()),
			Error::InvalidChunkSize("zero".into()),
		]
	}

	#[test]
	fn test_error_code_returns_screaming_snake_case() {
		let expected = vec![
			"CHUNK_TOO_LARGE",
			"FILE_TOO_LARGE",
			"EMPTY_DATA",
			"INVALID_CID",
			"AUTHORIZATION_NOT_FOUND",
			"INSUFFICIENT_AUTHORIZATION",
			"AUTHORIZATION_EXPIRED",
			"STORAGE_FAILED",
			"DAG_ENCODING_FAILED",
			"NETWORK_ERROR",
			"INVALID_CONFIG",
			"CHUNKING_FAILED",
			"RETRIEVAL_FAILED",
			"RENEWAL_NOT_FOUND",
			"RENEWAL_FAILED",
			"CID_CALCULATION_FAILED",
			"TRANSACTION_FAILED",
			"INVALID_CHUNK_SIZE",
		];

		for (error, expected_code) in all_errors().iter().zip(expected.iter()) {
			assert_eq!(error.code(), *expected_code, "Mismatch for {error:?}");
		}
	}

	#[test]
	fn test_is_retryable() {
		let retryable_codes = [
			"AUTHORIZATION_EXPIRED",
			"NETWORK_ERROR",
			"STORAGE_FAILED",
			"TRANSACTION_FAILED",
			"RETRIEVAL_FAILED",
			"RENEWAL_FAILED",
		];

		for error in all_errors() {
			let expected = retryable_codes.contains(&error.code());
			assert_eq!(
				error.is_retryable(),
				expected,
				"is_retryable mismatch for {} ({:?})",
				error.code(),
				error
			);
		}
	}

	#[test]
	fn test_recovery_hint_non_empty_for_all_variants() {
		for error in all_errors() {
			let hint = error.recovery_hint();
			assert!(!hint.is_empty(), "Empty recovery hint for {} ({:?})", error.code(), error);
		}
	}

	#[test]
	fn test_transaction_status_event_description() {
		let events = vec![
			(TransactionStatusEvent::Validated, "validated"),
			(TransactionStatusEvent::Broadcasted, "broadcast"),
			(
				TransactionStatusEvent::InBestBlock {
					block_hash: "0xabc".into(),
					block_number: Some(42),
					extrinsic_index: None,
				},
				"#42",
			),
			(
				TransactionStatusEvent::Finalized {
					block_hash: "0xdef".into(),
					block_number: Some(100),
					extrinsic_index: None,
				},
				"#100",
			),
			(TransactionStatusEvent::NoLongerInBestBlock, "no longer"),
			(TransactionStatusEvent::Invalid { error: "nonce".into() }, "nonce"),
			(TransactionStatusEvent::Dropped { error: "pool full".into() }, "pool full"),
		];

		for (event, expected_substring) in events {
			let desc = event.description();
			assert!(
				desc.contains(expected_substring),
				"Description {desc:?} should contain {expected_substring:?}"
			);
		}
	}

	#[test]
	fn test_transaction_status_event_description_without_block_number() {
		let event = TransactionStatusEvent::InBestBlock {
			block_hash: "0xabc".into(),
			block_number: None,
			extrinsic_index: None,
		};
		let desc = event.description();
		assert!(desc.contains("0xabc"));
		assert!(!desc.contains('#'));

		let event = TransactionStatusEvent::Finalized {
			block_hash: "0xdef".into(),
			block_number: None,
			extrinsic_index: None,
		};
		let desc = event.description();
		assert!(desc.contains("0xdef"));
		assert!(!desc.contains('#'));
	}
}
