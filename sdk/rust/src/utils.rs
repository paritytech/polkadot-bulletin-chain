// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Utility functions and helpers for Bulletin SDK

use crate::{
	cid::{ContentHash, HashingAlgorithm},
	types::{Error, Result},
};
use alloc::{string::String, vec::Vec};

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

	if hex.len() % 2 != 0 {
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

/// Get hash algorithm name as string
///
/// # Example
/// ```
/// use bulletin_sdk_rust::{utils::hash_algorithm_name, HashingAlgorithm};
///
/// assert_eq!(hash_algorithm_name(HashingAlgorithm::Blake2b256), "blake2b-256");
/// assert_eq!(hash_algorithm_name(HashingAlgorithm::Sha2_256), "sha2-256");
/// ```
pub fn hash_algorithm_name(algo: HashingAlgorithm) -> &'static str {
	match algo {
		HashingAlgorithm::Blake2b256 => "blake2b-256",
		HashingAlgorithm::Sha2_256 => "sha2-256",
		HashingAlgorithm::Keccak256 => "keccak-256",
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
		assert!(validate_chunk_size(2 * 1024 * 1024).is_ok()); // 2 MiB - valid
		assert!(validate_chunk_size(8 * 1024 * 1024).is_ok()); // 8 MiB - valid (max)
		assert!(validate_chunk_size(0).is_err()); // Zero - invalid
		assert!(validate_chunk_size(9 * 1024 * 1024).is_err()); // 9 MiB - exceeds limit
	}

	#[test]
	fn test_optimal_chunk_size() {
		assert_eq!(optimal_chunk_size(500_000), 500_000);
		assert_eq!(optimal_chunk_size(100_000_000), 1_048_576); // 1 MiB
		assert_eq!(optimal_chunk_size(1_000_000_000), 8_388_608); // 8 MiB (max)
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
	fn test_hash_algorithm_name() {
		assert_eq!(hash_algorithm_name(HashingAlgorithm::Blake2b256), "blake2b-256");
		assert_eq!(hash_algorithm_name(HashingAlgorithm::Sha2_256), "sha2-256");
		assert_eq!(hash_algorithm_name(HashingAlgorithm::Keccak256), "keccak-256");
	}
}
