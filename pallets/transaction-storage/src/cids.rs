// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Utilities for working with CIDs (Content Identifiers).
//!
//! This module provides types and functions to compute CIDs for raw data or
//! DAG blocks using supported hashing algorithms and codecs.
//!
//! See [`CidData`].

use crate::LOG_TARGET;
use alloc::vec::Vec;
use cid::{multihash::Multihash, CidGeneric};
use codec::{Decode, DecodeWithMemTracking, Encode, MaxEncodedLen};
use polkadot_sdk_frame::deps::sp_io;

/// 32-byte hash of a stored blob of data.
pub type ContentHash = [u8; 32];

/// CIDv1 serialized bytes (codec + multihash(ContentHash)).
pub type Cid = Vec<u8>;

/// Type alias representing a CID codec (e.g., raw = 0x55, dag-pb = 0x70).
pub type CidCodec = u64;

/// Supported hashing algorithms for computing CIDs.
#[derive(
	Clone,
	PartialEq,
	Eq,
	Encode,
	Debug,
	Decode,
	DecodeWithMemTracking,
	scale_info::TypeInfo,
	MaxEncodedLen,
)]
pub enum HashingAlgorithm {
	/// Blake2b-256 hash function.
	Blake2b256,
	/// SHA2-256 hash function.
	Sha2_256,
	/// Keccak-256 hash function.
	Keccak256,
}

impl HashingAlgorithm {
	/// Compute the hash of the given data using the selected algorithm.
	pub fn hash(&self, data: &[u8]) -> ContentHash {
		match self {
			HashingAlgorithm::Blake2b256 => sp_io::hashing::blake2_256(data),
			HashingAlgorithm::Sha2_256 => sp_io::hashing::sha2_256(data),
			HashingAlgorithm::Keccak256 => sp_io::hashing::keccak_256(data),
		}
	}

	/// Return the multihash code corresponding to this hashing algorithm.
	///
	/// These codes follow the [multihash table](https://github.com/multiformats/multicodec/blob/master/table.csv):
	/// - Blake2b-256 = 0xb220
	/// - SHA2-256 = 0x12
	/// - Keccak-256 = 0x1b
	pub fn multihash_code(&self) -> u64 {
		match self {
			HashingAlgorithm::Blake2b256 => 0xb220,
			HashingAlgorithm::Sha2_256 => 0x12,
			HashingAlgorithm::Keccak256 => 0x1b,
		}
	}
}

/// Configuration for generating a CID.
#[derive(
	Clone,
	PartialEq,
	Eq,
	Encode,
	Debug,
	Decode,
	DecodeWithMemTracking,
	scale_info::TypeInfo,
	MaxEncodedLen,
)]
pub struct CidConfig {
	/// CID codec (e.g., raw = 0x55, dag-pb = 0x70).
	pub codec: CidCodec,
	/// Hashing algorithm to use for computing the content hash.
	pub hashing: HashingAlgorithm,
}

/// Representation of a generated CID.
#[derive(Debug, PartialEq, Eq)]
pub struct CidData {
	/// 32-byte content hash of the input data.
	///
	/// Note: This is used for indexing transactions and retrieving
	/// `self.client.indexed_transaction(hash)`. Note: This is equal to `cid.hash().digest()`.
	pub content_hash: ContentHash,
	/// Hashing algorithm used.
	pub hashing: HashingAlgorithm,
	/// Codec used for the CIDv1.
	pub codec: CidCodec,
	/// CIDv1 serialized bytes (codec + multihash(content_hash)).
	pub cid: Cid,
}

/// Compute a CIDv1 for the given data with optional configuration.
///
/// If no configuration is provided, defaults are:
/// - Codec: raw (0x55)
/// - Hashing: Blake2b-256
///
/// # Errors
/// Returns `Err(())` if multihash wrapping fails.
pub fn calculate_cid(data: &[u8], config: Option<CidConfig>) -> Result<CidData, ()> {
	// Determine hashing algorithm and codec
	let (hashing, codec) = config.map_or_else(
		|| {
			// Defaults: raw codec (0x55) and Blake2b-256 hash
			(HashingAlgorithm::Blake2b256, 0x55)
		},
		|c| (c.hashing, c.codec),
	);

	// Hash the data
	let content_hash = hashing.hash(data);

	// Wrap hash into a multihash
	let multihash_code = hashing.multihash_code();
	let mh = Multihash::<32>::wrap(multihash_code, &content_hash).map_err(|e| {
		tracing::warn!(
			target: LOG_TARGET,
			"Failed to create Multihash for content_hash: {content_hash:?}, multihash_code: {multihash_code:?}, error: {e:?}"
		);
		()
	})?;

	// Create CIDv1 bytes
	let cid_bytes = CidGeneric::<32>::new_v1(codec, mh).to_bytes();

	// Return all relevant data
	Ok(CidData { content_hash, hashing, codec, cid: cid_bytes })
}

#[cfg(test)]
mod tests {
	use super::{calculate_cid, CidConfig, HashingAlgorithm};
	use cid::{
		multibase::{encode as to_base32, Base},
		CidGeneric,
	};
	use core::str::FromStr;
	use polkadot_sdk_frame::deps::sp_io;

	#[test]
	fn test_cid_raw_blake2b_256_roundtrip_works() {
		// Prepare data.
		let data = "Hello, Bulletin with PAPI - Fri Nov 21 2025 11:09:18 GMT+0000";
		let expected_content_hash = sp_io::hashing::blake2_256(data.as_bytes());

		// Expected raw CID calculated for the same data with `examples/common.js`.
		let expected_cid_base32 = "bafk2bzacedvk4eijklisgdjijnxky24pmkg7jgk5vsct4mwndj3nmx7plzz7m";
		let expected_cid = CidGeneric::<32>::from_str(expected_cid_base32).expect("valid_cid");
		assert_eq!(expected_cid.codec(), 0x55);
		assert_eq!(expected_cid.hash().code(), 0xb220);
		assert_eq!(expected_cid.hash().size(), 0x20);
		assert_eq!(expected_cid.hash().digest(), expected_content_hash);

		// Calculate CIDv1 with default raw codec and blake2b-256.
		let cid_raw = calculate_cid(data.as_ref(), None).expect("valid_cid");
		let cid_blake2b_256_raw = calculate_cid(
			data.as_ref(),
			Some(CidConfig { codec: 0x55, hashing: HashingAlgorithm::Blake2b256 }),
		)
		.expect("valid_cid");
		assert_eq!(cid_raw.cid, expected_cid.to_bytes());
		assert_eq!(to_base32(Base::Base32Lower, &cid_raw.cid), expected_cid_base32);
		assert_eq!(cid_raw.codec, expected_cid.codec());
		assert_eq!(cid_raw.hashing.multihash_code(), expected_cid.hash().code());
		assert_eq!(cid_raw.content_hash, expected_cid.hash().digest());
		assert_eq!(cid_raw, cid_blake2b_256_raw);
	}

	/// Return the HashingAlgorithm corresponding to a multihash code.
	pub fn from_multihash_code(code: u64) -> HashingAlgorithm {
		match code {
			0xb220 => HashingAlgorithm::Blake2b256,
			0x12 => HashingAlgorithm::Sha2_256,
			0x1b => HashingAlgorithm::Keccak256,
			code @ _ => panic!("{code} is not supported"),
		}
	}

	#[test]
	fn test_cid_various_codecs_and_hashes() {
		let data = "Hello, Bulletin with PAPI - Fri Nov 21 2025 11:09:18 GMT+0000";

		// Expected results from `examples/common.js`.
		let expected_cids = vec![
			// raw + blake2b_256
			("bafk2bzacedvk4eijklisgdjijnxky24pmkg7jgk5vsct4mwndj3nmx7plzz7m", 0x55, 0xb220),
			// DAG-PB + blake2b_256
			("bafykbzacedvk4eijklisgdjijnxky24pmkg7jgk5vsct4mwndj3nmx7plzz7m", 0x70, 0xb220),
			// Raw + sha2_256
			("bafkreig5pw2of63kmkldboh6utfovo3o3czig4yj7eb2ragxwca4c4jlke", 0x55, 0x12),
			// DAG-PB + sha2_256
			("bafybeig5pw2of63kmkldboh6utfovo3o3czig4yj7eb2ragxwca4c4jlke", 0x70, 0x12),
			// Raw + keccak_256
			("bafkrwifr4p73tsatchlyp3hivjee4prqqpcqayikzen46bqldwmt5mzd6e", 0x55, 0x1b),
			// DAG-PB + keccak_256
			("bafybwifr4p73tsatchlyp3hivjee4prqqpcqayikzen46bqldwmt5mzd6e", 0x70, 0x1b),
		];

		for (expected_cid_str, codec, mh_code) in expected_cids {
			let cid = CidGeneric::<32>::from_str(expected_cid_str).expect("valid CID");
			// Check codec and multihash code
			assert_eq!(cid.codec(), codec);
			assert_eq!(cid.hash().code(), mh_code);

			// Test `calculate_cid`
			let calculated = calculate_cid(
				data.as_ref(),
				Some(CidConfig { codec, hashing: from_multihash_code(mh_code) }),
			)
			.expect("calculate_cid succeeded");

			assert_eq!(to_base32(Base::Base32Lower, &calculated.cid), expected_cid_str);
			assert_eq!(calculated.codec, codec);
			assert_eq!(calculated.hashing.multihash_code(), mh_code);
			assert_eq!(calculated.content_hash, cid.hash().digest());
		}
	}
}
