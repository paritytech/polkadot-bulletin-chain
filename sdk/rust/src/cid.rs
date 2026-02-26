// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! CID (Content Identifier) utilities and re-exports.
//!
//! This module re-exports CID types from the pallet and provides
//! additional utility functions for working with CIDs in the SDK.

extern crate alloc;

use crate::types::{CidCodec, Error, HashAlgorithm, Result};

// Re-export CID types from the pallet
pub use pallet_transaction_storage::cids::{
	calculate_cid, Cid, CidConfig, CidData, CidError, ContentHash, HashingAlgorithm,
};

/// Convert SDK CidCodec enum to pallet CidCodec (u64).
pub fn codec_to_u64(codec: CidCodec) -> u64 {
	codec.code()
}

/// Convert SDK HashAlgorithm to pallet HashingAlgorithm.
///
/// Returns an error if the algorithm is not supported by the pallet.
pub fn hash_algorithm_to_pallet(algo: HashAlgorithm) -> Result<HashingAlgorithm> {
	match algo {
		HashAlgorithm::Blake2b256 => Ok(HashingAlgorithm::Blake2b256),
		HashAlgorithm::Sha2_256 => Ok(HashingAlgorithm::Sha2_256),
		HashAlgorithm::Sha2_512 =>
			Err(Error::UnsupportedHashAlgorithm("SHA2-512 is not supported by the pallet".into())),
		HashAlgorithm::Keccak256 => Ok(HashingAlgorithm::Keccak256),
	}
}

/// Create a CidConfig from SDK types.
///
/// Returns an error if the hash algorithm is not supported.
pub fn create_config(codec: CidCodec, hash_algo: HashAlgorithm) -> Result<CidConfig> {
	Ok(CidConfig { codec: codec_to_u64(codec), hashing: hash_algorithm_to_pallet(hash_algo)? })
}

/// Calculate CID for data using SDK configuration types.
///
/// Returns an error if the hash algorithm is not supported or CID calculation fails.
pub fn calculate_cid_with_config(
	data: &[u8],
	codec: CidCodec,
	hash_algo: HashAlgorithm,
) -> Result<CidData> {
	let config = create_config(codec, hash_algo)?;
	calculate_cid(data, config).map_err(|_| Error::InvalidCid("Failed to calculate CID".into()))
}

/// Calculate CID with default configuration (raw codec, blake2b-256).
pub fn calculate_cid_default(data: &[u8]) -> Result<CidData> {
	let config = CidConfig { codec: CidCodec::Raw.code(), hashing: HashingAlgorithm::Blake2b256 };
	calculate_cid(data, config).map_err(|_| Error::InvalidCid("Failed to calculate CID".into()))
}

/// Convert CidData to bytes (CIDv1 format).
pub fn cid_to_bytes(cid_data: &CidData) -> Result<Cid> {
	cid_data
		.to_bytes()
		.ok_or_else(|| Error::InvalidCid("Failed to serialize CID to bytes".into()))
}

/// Parse CID from bytes.
#[cfg(feature = "std")]
pub fn cid_from_bytes(bytes: &[u8]) -> Result<cid::Cid> {
	cid::Cid::try_from(bytes)
		.map_err(|e| Error::InvalidCid(alloc::format!("Failed to parse CID from bytes: {e:?}")))
}

/// Convert CID to base32 string representation.
#[cfg(feature = "std")]
pub fn cid_to_string(cid: &cid::Cid) -> String {
	cid.to_string()
}

/// Parse CID from base32 string.
#[cfg(feature = "std")]
pub fn cid_from_string(s: &str) -> Result<cid::Cid> {
	use core::str::FromStr;
	cid::Cid::from_str(s)
		.map_err(|e| Error::InvalidCid(alloc::format!("Failed to parse CID from string: {e:?}")))
}

/// Helper to convert multihash code to HashingAlgorithm.
pub fn multihash_code_to_algorithm(code: u64) -> Option<HashingAlgorithm> {
	match code {
		0xb220 => Some(HashingAlgorithm::Blake2b256),
		0x12 => Some(HashingAlgorithm::Sha2_256),
		0x1b => Some(HashingAlgorithm::Keccak256),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_calculate_cid_default() {
		let data = b"Hello, Bulletin!";
		let result = calculate_cid_default(data);
		assert!(result.is_ok());

		let cid_data = result.unwrap();
		assert_eq!(cid_data.codec, 0x55); // raw
		assert_eq!(cid_data.hashing.multihash_code(), 0xb220); // blake2b-256
	}

	#[test]
	fn test_calculate_cid_with_config() {
		let data = b"Hello, Bulletin!";
		let result = calculate_cid_with_config(data, CidCodec::DagPb, HashAlgorithm::Sha2_256);
		assert!(result.is_ok());

		let cid_data = result.unwrap();
		assert_eq!(cid_data.codec, 0x70); // dag-pb
		assert_eq!(cid_data.hashing.multihash_code(), 0x12); // sha2-256
	}

	#[test]
	fn test_cid_to_bytes() {
		let data = b"Hello, Bulletin!";
		let cid_data = calculate_cid_default(data).unwrap();
		let bytes = cid_to_bytes(&cid_data);
		assert!(bytes.is_ok());
		assert!(!bytes.unwrap().is_empty());
	}

	#[test]
	#[cfg(feature = "std")]
	fn test_cid_roundtrip() {
		let data = b"Hello, Bulletin!";
		let cid_data = calculate_cid_default(data).unwrap();
		let bytes = cid_to_bytes(&cid_data).unwrap();
		let parsed = cid_from_bytes(&bytes);
		assert!(parsed.is_ok());

		let cid = parsed.unwrap();
		let string = cid_to_string(&cid);
		let reparsed = cid_from_string(&string);
		assert!(reparsed.is_ok());
		assert_eq!(cid, reparsed.unwrap());
	}

	#[test]
	fn test_sha2_512_returns_error() {
		let data = b"Hello, Bulletin!";
		let result = calculate_cid_with_config(data, CidCodec::Raw, HashAlgorithm::Sha2_512);
		assert!(result.is_err());
		assert!(matches!(result.unwrap_err(), Error::UnsupportedHashAlgorithm(_)));
	}

	#[test]
	fn test_hash_algorithm_to_pallet_supported() {
		assert!(hash_algorithm_to_pallet(HashAlgorithm::Blake2b256).is_ok());
		assert!(hash_algorithm_to_pallet(HashAlgorithm::Sha2_256).is_ok());
		assert!(hash_algorithm_to_pallet(HashAlgorithm::Keccak256).is_ok());
	}

	#[test]
	fn test_hash_algorithm_to_pallet_unsupported() {
		assert!(hash_algorithm_to_pallet(HashAlgorithm::Sha2_512).is_err());
	}

	// ==================== Malformed CID Handling Tests ====================

	#[test]
	#[cfg(feature = "std")]
	fn test_cid_from_empty_bytes() {
		let result = cid_from_bytes(&[]);
		assert!(result.is_err());
		assert!(matches!(result.unwrap_err(), Error::InvalidCid(_)));
	}

	#[test]
	#[cfg(feature = "std")]
	fn test_cid_from_truncated_bytes() {
		// Valid CID bytes but truncated
		let truncated = &[0x01, 0x55]; // CIDv1 prefix but missing hash
		let result = cid_from_bytes(truncated);
		assert!(result.is_err());
	}

	#[test]
	#[cfg(feature = "std")]
	fn test_cid_from_invalid_version() {
		// Invalid CID version byte
		let invalid = &[0xFF, 0x55, 0xb2, 0x20, 0x20];
		let result = cid_from_bytes(invalid);
		assert!(result.is_err());
	}

	#[test]
	#[cfg(feature = "std")]
	fn test_cid_from_garbage_bytes() {
		// Random garbage that doesn't represent a valid CID
		let garbage = &[0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE];
		let result = cid_from_bytes(garbage);
		assert!(result.is_err());
	}

	#[test]
	#[cfg(feature = "std")]
	fn test_cid_from_invalid_string() {
		let result = cid_from_string("not-a-valid-cid");
		assert!(result.is_err());
		assert!(matches!(result.unwrap_err(), Error::InvalidCid(_)));
	}

	#[test]
	#[cfg(feature = "std")]
	fn test_cid_from_empty_string() {
		let result = cid_from_string("");
		assert!(result.is_err());
	}

	#[test]
	#[cfg(feature = "std")]
	fn test_cid_from_string_with_invalid_characters() {
		// Base32 doesn't use these characters
		let result = cid_from_string("bafybeig0!!!invalid###chars");
		assert!(result.is_err());
	}

	#[test]
	#[cfg(feature = "std")]
	fn test_cid_from_string_with_whitespace() {
		let result =
			cid_from_string("  bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi  ");
		// Should fail because whitespace is not valid in CIDs
		assert!(result.is_err());
	}

	#[test]
	fn test_multihash_code_to_algorithm_unknown() {
		// Unknown multihash code
		assert!(multihash_code_to_algorithm(0xFFFF).is_none());
		assert!(multihash_code_to_algorithm(0x00).is_none());
		assert!(multihash_code_to_algorithm(0x99).is_none());
	}

	#[test]
	fn test_multihash_code_to_algorithm_known() {
		assert_eq!(multihash_code_to_algorithm(0xb220), Some(HashingAlgorithm::Blake2b256));
		assert_eq!(multihash_code_to_algorithm(0x12), Some(HashingAlgorithm::Sha2_256));
		assert_eq!(multihash_code_to_algorithm(0x1b), Some(HashingAlgorithm::Keccak256));
	}

	#[test]
	fn test_calculate_cid_empty_data() {
		// Empty data should still produce a valid CID
		let result = calculate_cid_default(&[]);
		assert!(result.is_ok());
	}

	#[test]
	fn test_calculate_cid_large_data() {
		// 1 MB of data
		let large_data = vec![0xABu8; 1024 * 1024];
		let result = calculate_cid_default(&large_data);
		assert!(result.is_ok());
	}

	#[test]
	fn test_cid_codec_values() {
		// Verify codec values match IPFS standards
		assert_eq!(CidCodec::Raw.code(), 0x55);
		assert_eq!(CidCodec::DagPb.code(), 0x70);
		assert_eq!(CidCodec::DagCbor.code(), 0x71);
	}

	#[test]
	fn test_cid_deterministic() {
		// Same data should always produce same CID
		let data = b"deterministic test data";

		let cid1 = calculate_cid_default(data).unwrap();
		let cid2 = calculate_cid_default(data).unwrap();

		assert_eq!(cid1.codec, cid2.codec);
		assert_eq!(cid1.content_hash, cid2.content_hash);
	}

	#[test]
	fn test_different_data_different_cid() {
		let data1 = b"data one";
		let data2 = b"data two";

		let cid1 = calculate_cid_default(data1).unwrap();
		let cid2 = calculate_cid_default(data2).unwrap();

		// Different data should produce different hashes
		assert_ne!(cid1.content_hash, cid2.content_hash);
	}
}
