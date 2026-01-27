// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! CID (Content Identifier) utilities and re-exports.
//!
//! This module re-exports CID types from the pallet and provides
//! additional utility functions for working with CIDs in the SDK.

#![cfg_attr(not(feature = "std"), no_std)]

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
pub fn hash_algorithm_to_pallet(algo: HashAlgorithm) -> HashingAlgorithm {
	match algo {
		HashAlgorithm::Blake2b256 => HashingAlgorithm::Blake2b256,
		HashAlgorithm::Sha2_256 => HashingAlgorithm::Sha2_256,
		HashAlgorithm::Sha2_512 => {
			// Note: The pallet doesn't support SHA2-512 yet
			// Default to SHA2-256 for now
			HashingAlgorithm::Sha2_256
		},
		HashAlgorithm::Keccak256 => HashingAlgorithm::Keccak256,
	}
}

/// Create a CidConfig from SDK types.
pub fn create_config(codec: CidCodec, hash_algo: HashAlgorithm) -> CidConfig {
	CidConfig {
		codec: codec_to_u64(codec),
		hashing: hash_algorithm_to_pallet(hash_algo),
	}
}

/// Calculate CID for data using SDK configuration types.
pub fn calculate_cid_with_config(
	data: &[u8],
	codec: CidCodec,
	hash_algo: HashAlgorithm,
) -> Result<CidData> {
	let config = create_config(codec, hash_algo);
	calculate_cid(data, Some(config)).map_err(|_| Error::InvalidCid("Failed to calculate CID".into()))
}

/// Calculate CID with default configuration (raw codec, blake2b-256).
pub fn calculate_cid_default(data: &[u8]) -> Result<CidData> {
	calculate_cid(data, None).map_err(|_| Error::InvalidCid("Failed to calculate CID".into()))
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
		.map_err(|e| Error::InvalidCid(alloc::format!("Failed to parse CID from bytes: {:?}", e)))
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
		.map_err(|e| Error::InvalidCid(alloc::format!("Failed to parse CID from string: {:?}", e)))
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
}
