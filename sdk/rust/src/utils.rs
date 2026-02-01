// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Utility functions and helpers for Bulletin SDK

use crate::{
	cid::ContentHash,
	types::{CidCodec, Error, HashAlgorithm, Result},
};
use alloc::{string::String, vec::Vec};

#[cfg(feature = "std")]
use sp_runtime::AccountId32;

/// Convert hex string to bytes
///
/// # Example
/// ```
/// use bulletin_sdk_rust::utils::hex_to_bytes;
///
/// let bytes = hex_to_bytes("deadbeef").unwrap();
/// assert_eq!(bytes, vec![0xde, 0xad, 0xbe, 0xef]);
/// ```
pub fn hex_to_bytes(hex: &str) -> Result<Vec<u8>> {
	let hex = hex.trim_start_matches("0x");

	if !hex.len().is_multiple_of(2) {
		return Err(Error::InvalidConfig("Hex string must have even length".into()));
	}

	let mut bytes = Vec::with_capacity(hex.len() / 2);
	for i in (0..hex.len()).step_by(2) {
		let byte = u8::from_str_radix(&hex[i..i + 2], 16)
			.map_err(|_| Error::InvalidConfig("Invalid hex character".into()))?;
		bytes.push(byte);
	}

	Ok(bytes)
}

/// Convert bytes to hex string
///
/// # Example
/// ```
/// use bulletin_sdk_rust::utils::bytes_to_hex;
///
/// let hex = bytes_to_hex(&[0xde, 0xad, 0xbe, 0xef]);
/// assert_eq!(hex, "deadbeef");
/// ```
pub fn bytes_to_hex(bytes: &[u8]) -> String {
	let mut hex = String::with_capacity(bytes.len() * 2);
	for byte in bytes {
		hex.push_str(&alloc::format!("{byte:02x}"));
	}
	hex
}

/// Convert SS58 address to AccountId32
///
/// # Example
/// ```no_run
/// use bulletin_sdk_rust::utils::ss58_to_account_id;
///
/// let account = ss58_to_account_id("5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY").unwrap();
/// ```
#[cfg(feature = "std")]
pub fn ss58_to_account_id(ss58: &str) -> Result<AccountId32> {
	use sp_core::crypto::Ss58Codec;

	AccountId32::from_ss58check(ss58)
		.map_err(|e| Error::InvalidConfig(alloc::format!("Invalid SS58 address: {e:?}")))
}

/// Convert AccountId32 to SS58 address
///
/// # Example
/// ```
/// use bulletin_sdk_rust::utils::account_id_to_ss58;
/// use sp_runtime::AccountId32;
///
/// let account = AccountId32::new([0u8; 32]);
/// let ss58 = account_id_to_ss58(&account, 42);
/// ```
#[cfg(feature = "std")]
pub fn account_id_to_ss58(account: &AccountId32, prefix: u16) -> String {
	use sp_core::crypto::Ss58Codec;

	account.to_ss58check_with_version(prefix.into())
}

/// Parse CID from string representation
///
/// # Example
/// ```no_run
/// use bulletin_sdk_rust::utils::parse_cid_string;
///
/// let cid = parse_cid_string("bafkreiabcd1234...").unwrap();
/// ```
pub fn parse_cid_string(cid_str: &str) -> Result<Vec<u8>> {
	// Try to parse as base58 or base32 CID
	// This is a simplified version - full CID parsing would require the `cid` crate
	if cid_str.starts_with("Qm") || cid_str.starts_with("bafy") || cid_str.starts_with("bafk") {
		// These are IPFS CID formats
		// For full implementation, use: cid::Cid::try_from(cid_str)
		Err(Error::InvalidCid("CID parsing requires 'cid' crate".into()))
	} else if cid_str.starts_with("0x") {
		// Raw hex format
		hex_to_bytes(cid_str)
	} else {
		Err(Error::InvalidCid("Unknown CID format".into()))
	}
}

/// Format CID bytes as string
///
/// # Example
/// ```
/// use bulletin_sdk_rust::utils::format_cid;
///
/// let cid_bytes = vec![0x12, 0x20, 0xab, 0xcd];
/// let formatted = format_cid(&cid_bytes);
/// assert_eq!(formatted, "0x1220abcd");
/// ```
pub fn format_cid(cid: &[u8]) -> String {
	alloc::format!("0x{}", bytes_to_hex(cid))
}

/// Calculate content hash (Blake2b-256) from data
///
/// # Example
/// ```
/// use bulletin_sdk_rust::utils::hash_data;
///
/// let data = b"Hello, Bulletin!";
/// let hash = hash_data(data);
/// assert_eq!(hash.len(), 32);
/// ```
pub fn hash_data(data: &[u8]) -> ContentHash {
	sp_io::hashing::blake2_256(data)
}

/// Retry helper for async operations
///
/// # Example
/// ```no_run
/// use bulletin_sdk_rust::utils::retry_async;
///
/// # async fn example() {
/// let result = retry_async(3, 1000, || async {
///     // Your async operation
///     Ok::<_, bulletin_sdk_rust::Error>(())
/// }).await;
/// # }
/// ```
#[cfg(feature = "std")]
pub async fn retry_async<F, Fut, T>(max_retries: u32, delay_ms: u64, mut f: F) -> Result<T>
where
	F: FnMut() -> Fut,
	Fut: core::future::Future<Output = Result<T>>,
{
	let mut last_error = None;

	for attempt in 0..=max_retries {
		match f().await {
			Ok(result) => return Ok(result),
			Err(e) => {
				last_error = Some(e);
				if attempt < max_retries {
					tokio::time::sleep(tokio::time::Duration::from_millis(
						delay_ms * (attempt as u64 + 1),
					))
					.await;
				}
			},
		}
	}

	Err(last_error.unwrap_or_else(|| Error::SubmissionFailed("Retry failed".into())))
}

/// Validate chunk size
///
/// # Example
/// ```
/// use bulletin_sdk_rust::utils::validate_chunk_size;
///
/// assert!(validate_chunk_size(1_048_576).is_ok()); // 1 MiB - valid
/// assert!(validate_chunk_size(10_000_000).is_err()); // > 2 MiB - invalid
/// ```
pub fn validate_chunk_size(size: u64) -> Result<()> {
	use crate::chunker::MAX_CHUNK_SIZE;

	if size == 0 {
		return Err(Error::InvalidConfig("Chunk size cannot be zero".into()));
	}

	if size > MAX_CHUNK_SIZE as u64 {
		return Err(Error::InvalidConfig(alloc::format!(
			"Chunk size {size} exceeds maximum {MAX_CHUNK_SIZE}"
		)));
	}

	Ok(())
}

/// Calculate optimal chunk size for given data size
///
/// Returns a chunk size that balances transaction overhead and throughput.
///
/// # Example
/// ```
/// use bulletin_sdk_rust::utils::optimal_chunk_size;
///
/// let size = optimal_chunk_size(100_000_000); // 100 MB
/// assert!(size >= 1_048_576); // At least 1 MiB
/// ```
pub fn optimal_chunk_size(data_size: u64) -> u64 {
	use crate::chunker::{MAX_CHUNK_SIZE, MIN_CHUNK_SIZE};

	const OPTIMAL_CHUNKS: u64 = 100; // Target chunk count

	if data_size <= MIN_CHUNK_SIZE as u64 {
		return data_size;
	}

	let optimal_size = data_size / OPTIMAL_CHUNKS;

	if optimal_size < MIN_CHUNK_SIZE as u64 {
		MIN_CHUNK_SIZE as u64
	} else if optimal_size > MAX_CHUNK_SIZE as u64 {
		MAX_CHUNK_SIZE as u64
	} else {
		// Round to nearest MiB
		(optimal_size / 1_048_576) * 1_048_576
	}
}

/// Estimate transaction fees for given data size
///
/// This is a rough estimate and actual fees may vary.
///
/// # Example
/// ```
/// use bulletin_sdk_rust::utils::estimate_fees;
///
/// let fees = estimate_fees(1_000_000); // 1 MB
/// assert!(fees > 0);
/// ```
pub fn estimate_fees(data_size: u64) -> u64 {
	// Base fee + per-byte fee
	// These are placeholder values - actual fees depend on chain configuration
	const BASE_FEE: u64 = 1_000_000; // Base transaction fee
	const PER_BYTE_FEE: u64 = 100; // Fee per byte

	BASE_FEE + (data_size * PER_BYTE_FEE)
}

/// Get codec name as string
///
/// # Example
/// ```
/// use bulletin_sdk_rust::{utils::codec_name, types::CidCodec};
///
/// assert_eq!(codec_name(CidCodec::Raw), "raw");
/// assert_eq!(codec_name(CidCodec::DagPb), "dag-pb");
/// ```
pub fn codec_name(codec: CidCodec) -> &'static str {
	match codec {
		CidCodec::Raw => "raw",
		CidCodec::DagPb => "dag-pb",
		CidCodec::DagCbor => "dag-cbor",
	}
}

/// Get hash algorithm name as string
///
/// # Example
/// ```
/// use bulletin_sdk_rust::{utils::hash_algorithm_name, types::HashAlgorithm};
///
/// assert_eq!(hash_algorithm_name(HashAlgorithm::Blake2b256), "blake2b-256");
/// assert_eq!(hash_algorithm_name(HashAlgorithm::Sha2_256), "sha2-256");
/// ```
pub fn hash_algorithm_name(algo: HashAlgorithm) -> &'static str {
	match algo {
		HashAlgorithm::Blake2b256 => "blake2b-256",
		HashAlgorithm::Sha2_256 => "sha2-256",
		HashAlgorithm::Sha2_512 => "sha2-512",
		HashAlgorithm::Keccak256 => "keccak-256",
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_hex_conversion() {
		let bytes = vec![0xde, 0xad, 0xbe, 0xef];
		let hex = bytes_to_hex(&bytes);
		assert_eq!(hex, "deadbeef");

		let decoded = hex_to_bytes(&hex).unwrap();
		assert_eq!(decoded, bytes);
	}

	#[test]
	fn test_hex_with_prefix() {
		let decoded = hex_to_bytes("0xdeadbeef").unwrap();
		assert_eq!(decoded, vec![0xde, 0xad, 0xbe, 0xef]);
	}

	#[test]
	fn test_invalid_hex() {
		assert!(hex_to_bytes("xyz").is_err());
		assert!(hex_to_bytes("abc").is_err()); // Odd length
	}

	#[test]
	fn test_validate_chunk_size() {
		assert!(validate_chunk_size(1_048_576).is_ok()); // 1 MiB - valid
		assert!(validate_chunk_size(2 * 1024 * 1024).is_ok()); // 2 MiB - valid (max)
		assert!(validate_chunk_size(0).is_err()); // Zero - invalid
		assert!(validate_chunk_size(3 * 1024 * 1024).is_err()); // 3 MiB - exceeds limit
		assert!(validate_chunk_size(10_000_000).is_err()); // > 2 MiB - invalid
	}

	#[test]
	fn test_optimal_chunk_size() {
		assert_eq!(optimal_chunk_size(500_000), 500_000);
		assert_eq!(optimal_chunk_size(100_000_000), 1_048_576); // 1 MiB
		assert_eq!(optimal_chunk_size(1_000_000_000), 2_097_152); // 2 MiB (max)
	}

	#[test]
	fn test_estimate_fees() {
		let fees = estimate_fees(1_000_000);
		assert!(fees > 0);
		assert_eq!(fees, 1_000_000 + 1_000_000 * 100);
	}

	#[test]
	fn test_hash_data() {
		let data = b"Hello, Bulletin!";
		let hash = hash_data(data);
		assert_eq!(hash.len(), 32);

		// Same data should produce same hash
		let hash2 = hash_data(data);
		assert_eq!(hash, hash2);

		// Different data should produce different hash
		let hash3 = hash_data(b"Different data");
		assert_ne!(hash, hash3);
	}

	#[test]
	fn test_format_cid() {
		let cid = vec![0x12, 0x20, 0xab, 0xcd];
		let formatted = format_cid(&cid);
		assert_eq!(formatted, "0x1220abcd");
	}

	#[test]
	fn test_codec_name() {
		assert_eq!(codec_name(CidCodec::Raw), "raw");
		assert_eq!(codec_name(CidCodec::DagPb), "dag-pb");
		assert_eq!(codec_name(CidCodec::DagCbor), "dag-cbor");
	}

	#[test]
	fn test_hash_algorithm_name() {
		assert_eq!(hash_algorithm_name(HashAlgorithm::Blake2b256), "blake2b-256");
		assert_eq!(hash_algorithm_name(HashAlgorithm::Sha2_256), "sha2-256");
		assert_eq!(hash_algorithm_name(HashAlgorithm::Keccak256), "keccak-256");
	}
}
