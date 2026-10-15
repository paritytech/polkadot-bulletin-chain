// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Common types and error definitions for the Bulletin SDK.

use crate::cid::{CidCodec, HashingAlgorithm};
use alloc::{string::String, vec::Vec};
use codec::{Decode, Encode};
use scale_info::TypeInfo;

/// Result type for SDK operations.
pub type Result<T> = core::result::Result<T, Error>;

/// SDK error types. The codes from [`Error::code`] are shared with the TypeScript SDK.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum Error {
	/// Chunk size exceeds maximum allowed (2 MiB).
	#[cfg_attr(
		feature = "std",
		error("Chunk size {0} exceeds maximum allowed size of 2097152 bytes (2 MiB)")
	)]
	ChunkTooLarge(u64),

	#[cfg_attr(feature = "std", error("Data too large: {0}"))]
	DataTooLarge(String),

	/// Data is empty.
	#[cfg_attr(feature = "std", error("Data cannot be empty"))]
	EmptyData,

	/// Invalid CID format.
	#[cfg_attr(feature = "std", error("Invalid CID: {0}"))]
	InvalidCid(String),

	/// No unexpired authorization for the account or content hash.
	#[cfg_attr(feature = "std", error("Authorization not found for account: {0}"))]
	AuthorizationNotFound(String),

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

	/// CID calculation failed.
	#[cfg_attr(feature = "std", error("CID calculation failed: {0}"))]
	CidCalculationFailed(String),

	/// A submitted transaction failed: dispatch error, dropped, or the status stream ended.
	#[cfg_attr(feature = "std", error("Transaction failed: {0}"))]
	TransactionFailed(String),

	#[cfg_attr(feature = "std", error("Transaction invalid: {message}"))]
	TransactionInvalid { custom_code: Option<u8>, message: String },

	/// Invalid chunk size.
	#[cfg_attr(feature = "std", error("Invalid chunk size: {0}"))]
	InvalidChunkSize(String),

	#[cfg_attr(feature = "std", error("Unsupported operation: {0}"))]
	UnsupportedOperation(String),

	/// The connected chain's runtime predates `BulletinTransactionStorageApi`.
	#[cfg_attr(feature = "std", error("Authorization runtime API is not available on this chain"))]
	AuthorizationApiUnavailable,

	/// Upload pipeline received no chainHead progress within the stall window
	/// (the RPC connection is likely unhealthy).
	#[cfg_attr(feature = "std", error("Store stalled: {0}"))]
	StoreStalled(String),

	/// An item's nonce slot was repeatedly taken by other transactions from the
	/// same signer; the per-item retry budget was exhausted.
	#[cfg_attr(feature = "std", error("Hijack budget exceeded: {0}"))]
	HijackBudgetExceeded(String),

	/// The upload run completed, but some items exhausted their retry budget
	/// and were NOT stored (their `ItemFailed` events carry the per-item cause).
	#[cfg_attr(feature = "std", error("Upload incomplete: {0}"))]
	UploadIncomplete(String),
}

impl Error {
	/// Returns a `SCREAMING_SNAKE_CASE` error code matching the TypeScript SDK.
	pub fn code(&self) -> &'static str {
		match self {
			Error::ChunkTooLarge(_) => "CHUNK_TOO_LARGE",
			Error::DataTooLarge(_) => "DATA_TOO_LARGE",
			Error::EmptyData => "EMPTY_DATA",
			Error::InvalidCid(_) => "INVALID_CID",
			Error::AuthorizationNotFound(_) => "AUTHORIZATION_NOT_FOUND",
			Error::DagEncodingFailed(_) => "DAG_ENCODING_FAILED",
			Error::NetworkError(_) => "NETWORK_ERROR",
			Error::InvalidConfig(_) => "INVALID_CONFIG",
			Error::ChunkingFailed(_) => "CHUNKING_FAILED",
			Error::CidCalculationFailed(_) => "CID_CALCULATION_FAILED",
			Error::TransactionFailed(_) => "TRANSACTION_FAILED",
			Error::TransactionInvalid { .. } => "TRANSACTION_INVALID",
			Error::InvalidChunkSize(_) => "INVALID_CHUNK_SIZE",
			Error::UnsupportedOperation(_) | Error::AuthorizationApiUnavailable =>
				"UNSUPPORTED_OPERATION",
			Error::StoreStalled(_) => "STORE_STALLED",
			Error::HijackBudgetExceeded(_) => "HIJACK_BUDGET_EXCEEDED",
			Error::UploadIncomplete(_) => "UPLOAD_INCOMPLETE",
		}
	}

	/// Returns `true` if this error is likely transient and retrying may succeed.
	pub fn is_retryable(&self) -> bool {
		matches!(
			self,
			Error::NetworkError(_) | Error::TransactionFailed(_) | Error::StoreStalled(_)
		)
	}

	/// Returns an actionable recovery suggestion for this error.
	pub fn recovery_hint(&self) -> &'static str {
		match self {
			Error::ChunkTooLarge(_) => "Reduce chunk size to 2 MiB or less",
			Error::DataTooLarge(_) => "Reduce the data size or use a chunked upload",
			Error::EmptyData => "Provide non-empty data",
			Error::InvalidCid(_) => "Verify CID format",
			Error::AuthorizationNotFound(_) =>
				"Authorize the account or the content hash first, then retry",
			Error::DagEncodingFailed(_) => "Check chunk CIDs and data integrity",
			Error::NetworkError(_) => "Check network connectivity to the RPC endpoint, then retry",
			Error::InvalidConfig(_) => "Check configuration parameters",
			Error::ChunkingFailed(_) => "Verify data integrity and chunker configuration",
			Error::CidCalculationFailed(_) => "Verify data and hash algorithm",
			Error::TransactionFailed(_) =>
				"Verify transaction parameters and account nonce, then retry",
			Error::TransactionInvalid { .. } =>
				"The transaction pool rejected the transaction; check the validity error before retrying",
			Error::InvalidChunkSize(_) => "Use a chunk size between 1 byte and 2 MiB",
			Error::UnsupportedOperation(_) | Error::AuthorizationApiUnavailable =>
				"This operation is not supported by this client or the connected runtime",
			Error::StoreStalled(_) =>
				"The RPC connection sent no chainHead progress; retry on a fresh client",
			Error::HijackBudgetExceeded(_) =>
				"Another transaction from this signer keeps taking the nonce; avoid concurrent submissions on this account",
			Error::UploadIncomplete(_) =>
				"Some items were not stored; re-run the estimate with skip-existing enabled, then submit to retry only the missing items",
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
	/// CID of this chunk as bytes (computed after encoding); `None` until set.
	/// Mirrors the TypeScript SDK's optional `Chunk.cid`.
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
	/// Transaction index from the `Stored` event — the storage slot among the
	/// block's stored transactions that `renew(block_number, transaction_index)`
	/// targets. This is NOT the extrinsic index in the block body.
	pub transaction_index: Option<u32>,
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

/// Transaction confirmation level.
///
/// Controls when transaction submission methods resolve:
/// - `InBlock`: Return when the transaction is included in a best block (faster, may reorg)
/// - `Finalized`: Return when the transaction is finalized (safer, slower)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub enum WaitFor {
	/// Return when the transaction is in a best block.
	/// Faster but the block may be reorganized.
	InBlock,
	/// Return when the transaction is finalized.
	/// Slower but guarantees the transaction is permanent.
	#[default]
	Finalized,
}

/// Options for storing data.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct StoreOptions {
	/// CID codec to use (default: raw).
	pub cid_codec: CidCodec,
	/// Hashing algorithm to use (default: blake2b-256).
	pub hash_algorithm: HashingAlgorithm,
	/// What to wait for before returning (default: finalized).
	pub wait_for: WaitFor,
}

impl Default for StoreOptions {
	fn default() -> Self {
		Self {
			cid_codec: CidCodec::Raw,
			hash_algorithm: HashingAlgorithm::Blake2b256,
			wait_for: WaitFor::default(),
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

/// Transaction status event types.
///
/// Mirrors the TypeScript SDK's `TxStatus` / `TransactionStatusEvent` union
/// name-for-name. `chunk_index` is an optional trace id correlating an event to
/// a chunk in a multi-item upload; `tx_index` is the `Stored`-event transaction
/// index (the `renew` slot), NOT the extrinsic index in the block body. (TS
/// marks `blockNumber` required; here it stays optional because the client
/// fetches it fallibly.)
#[derive(Debug, Clone)]
pub enum TransactionStatusEvent {
	/// Transaction has been signed.
	Signed { tx_hash: String, chunk_index: Option<u32> },
	/// Transaction has been validated and added to the transaction pool.
	Validated { chunk_index: Option<u32> },
	/// Transaction has been broadcast to peers.
	Broadcasted { chunk_index: Option<u32> },
	/// Transaction is now in a best block.
	InBlock {
		block_hash: String,
		block_number: Option<u32>,
		tx_index: Option<u32>,
		chunk_index: Option<u32>,
	},
	/// Transaction has been finalized.
	Finalized {
		block_hash: String,
		block_number: Option<u32>,
		tx_index: Option<u32>,
		chunk_index: Option<u32>,
	},
	/// Transaction was in a block that got reorganized.
	NoLongerInBlock { chunk_index: Option<u32> },
	/// Transaction is not valid anymore (e.g., nonce too low).
	Invalid { error: String, chunk_index: Option<u32> },
	/// Transaction was dropped from the pool.
	Dropped { error: String, chunk_index: Option<u32> },
}

impl TransactionStatusEvent {
	/// Returns a human-readable description of this transaction status event.
	pub fn description(&self) -> String {
		match self {
			TransactionStatusEvent::Signed { tx_hash, .. } =>
				alloc::format!("Transaction signed ({tx_hash})"),
			TransactionStatusEvent::Validated { .. } =>
				"Transaction validated and added to the pool".into(),
			TransactionStatusEvent::Broadcasted { .. } => "Transaction broadcast to peers".into(),
			TransactionStatusEvent::InBlock { block_hash, block_number, .. } =>
				match block_number {
					Some(n) => alloc::format!("Transaction in best block #{n} ({block_hash})"),
					None => alloc::format!("Transaction in best block ({block_hash})"),
				},
			TransactionStatusEvent::Finalized { block_hash, block_number, .. } =>
				match block_number {
					Some(n) => alloc::format!("Transaction finalized in block #{n} ({block_hash})"),
					None => alloc::format!("Transaction finalized in block ({block_hash})"),
				},
			TransactionStatusEvent::NoLongerInBlock { .. } =>
				"Transaction no longer in best block (reorg occurred)".into(),
			TransactionStatusEvent::Invalid { error, .. } =>
				alloc::format!("Transaction invalid: {error}"),
			TransactionStatusEvent::Dropped { error, .. } =>
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
		ProgressEvent::Transaction(TransactionStatusEvent::Validated { chunk_index: None })
	}

	/// Create a Broadcasted transaction event.
	pub fn tx_broadcasted() -> Self {
		ProgressEvent::Transaction(TransactionStatusEvent::Broadcasted { chunk_index: None })
	}

	/// Create an InBlock transaction event.
	pub fn tx_in_block(
		block_hash: String,
		block_number: Option<u32>,
		tx_index: Option<u32>,
	) -> Self {
		ProgressEvent::Transaction(TransactionStatusEvent::InBlock {
			block_hash,
			block_number,
			tx_index,
			chunk_index: None,
		})
	}

	/// Create a Finalized transaction event.
	pub fn tx_finalized(
		block_hash: String,
		block_number: Option<u32>,
		tx_index: Option<u32>,
	) -> Self {
		ProgressEvent::Transaction(TransactionStatusEvent::Finalized {
			block_hash,
			block_number,
			tx_index,
			chunk_index: None,
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
			Error::DataTooLarge("200 bytes".into()),
			Error::EmptyData,
			Error::InvalidCid("bad".into()),
			Error::AuthorizationNotFound("acct".into()),
			Error::DagEncodingFailed("bad dag".into()),
			Error::NetworkError("timeout".into()),
			Error::InvalidConfig("bad config".into()),
			Error::ChunkingFailed("chunk err".into()),
			Error::CidCalculationFailed("calc fail".into()),
			Error::TransactionFailed("tx fail".into()),
			Error::TransactionInvalid { custom_code: Some(11), message: "Custom error: 11".into() },
			Error::InvalidChunkSize("zero".into()),
			Error::UnsupportedOperation("renew".into()),
			Error::AuthorizationApiUnavailable,
			Error::StoreStalled("no events".into()),
			Error::HijackBudgetExceeded("slot taken".into()),
			Error::UploadIncomplete("1 of 3 items failed".into()),
		]
	}

	#[test]
	fn test_error_code_returns_screaming_snake_case() {
		let expected = vec![
			"CHUNK_TOO_LARGE",
			"DATA_TOO_LARGE",
			"EMPTY_DATA",
			"INVALID_CID",
			"AUTHORIZATION_NOT_FOUND",
			"DAG_ENCODING_FAILED",
			"NETWORK_ERROR",
			"INVALID_CONFIG",
			"CHUNKING_FAILED",
			"CID_CALCULATION_FAILED",
			"TRANSACTION_FAILED",
			"TRANSACTION_INVALID",
			"INVALID_CHUNK_SIZE",
			"UNSUPPORTED_OPERATION",
			"UNSUPPORTED_OPERATION",
			"STORE_STALLED",
			"HIJACK_BUDGET_EXCEEDED",
			"UPLOAD_INCOMPLETE",
		];
		assert_eq!(all_errors().len(), expected.len(), "expected[] out of sync with all_errors()");
		for (error, expected_code) in all_errors().iter().zip(expected.iter()) {
			assert_eq!(error.code(), *expected_code, "Mismatch for {error:?}");
		}
	}

	#[test]
	fn test_is_retryable() {
		let retryable_codes = ["NETWORK_ERROR", "TRANSACTION_FAILED", "STORE_STALLED"];

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
			(TransactionStatusEvent::Validated { chunk_index: None }, "validated"),
			(TransactionStatusEvent::Broadcasted { chunk_index: None }, "broadcast"),
			(
				TransactionStatusEvent::InBlock {
					block_hash: "0xabc".into(),
					block_number: Some(42),
					tx_index: None,
					chunk_index: None,
				},
				"#42",
			),
			(
				TransactionStatusEvent::Finalized {
					block_hash: "0xdef".into(),
					block_number: Some(100),
					tx_index: None,
					chunk_index: None,
				},
				"#100",
			),
			(TransactionStatusEvent::NoLongerInBlock { chunk_index: None }, "no longer"),
			(TransactionStatusEvent::Invalid { error: "nonce".into(), chunk_index: None }, "nonce"),
			(
				TransactionStatusEvent::Dropped { error: "pool full".into(), chunk_index: None },
				"pool full",
			),
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
		let event = TransactionStatusEvent::InBlock {
			block_hash: "0xabc".into(),
			block_number: None,
			tx_index: None,
			chunk_index: None,
		};
		let desc = event.description();
		assert!(desc.contains("0xabc"));
		assert!(!desc.contains('#'));

		let event = TransactionStatusEvent::Finalized {
			block_hash: "0xdef".into(),
			block_number: None,
			tx_index: None,
			chunk_index: None,
		};
		let desc = event.description();
		assert!(desc.contains("0xdef"));
		assert!(!desc.contains('#'));
	}
}
