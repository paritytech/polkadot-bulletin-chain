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

use crate::LOG_TARGET;
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
	/// - Blake2b-256 = 0xb2 (178)
	/// - SHA2-256 = 0x12 (18)
	/// - Keccak-256 = 0x1b (27)
	pub fn multihash_code(&self) -> u64 {
		match self {
			HashingAlgorithm::Blake2b256 => 0xb2,
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
pub struct CidData {
	/// 32-byte content hash of the input data.
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
	let (hashing, codec) = if let Some(config) = config {
		(config.hashing, config.codec)
	} else {
		// Defaults: raw codec (0x55) and Blake2b-256 hash
		(HashingAlgorithm::Blake2b256, 0x55)
	};

	// Hash the data
	let content_hash = hashing.hash(data);

	// Wrap hash into a multihash
	let multihash_code = hashing.multihash_code();
	let mh = Multihash::<32>::wrap(multihash_code, &content_hash).map_err(|e| {
		log::warn!(
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

// TODO: add here more tests for compatibility.

