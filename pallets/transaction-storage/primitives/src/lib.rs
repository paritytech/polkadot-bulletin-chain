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

//! Primitives for the transaction storage pallet.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_runtime::transaction_validity::{
	TransactionLongevity, TransactionPriority, ValidTransaction,
};

pub mod cids;

/// 32-byte hash of a stored blob of data.
pub type ContentHash = [u8; 32];

/// Stock `EntryMeta` for runtimes wiring the renewal pallet; each variant is written
/// by exactly one pallet. A runtime needing more variants wires its own enum
/// implementing the same consumer traits.
///
/// INVARIANT: live `Transactions` entries store these exact bytes; do not reorder or
/// renumber variants. Locked by `entry_kind_encoding_is_frozen`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Encode, Decode, TypeInfo, MaxEncodedLen)]
pub enum EntryKind {
	/// Written by the storage pallet's `store` calls, as the `Default` value.
	#[default]
	#[codec(index = 0)]
	Store,
	/// Written by the renewal pallet's `renew`/`force_renew`/auto-renewal, via its
	/// `RenewMeta` impl.
	#[codec(index = 1)]
	Renew,
}

/// A [`ValidTransaction`] minus its `provides` payload. Families that must not evict each
/// other in the pool need distinct `tag_prefix`es.
#[derive(Clone, Copy, Encode, TypeInfo)]
pub struct ValidTransactionParams {
	pub tag_prefix: &'static str,
	pub priority: TransactionPriority,
	pub longevity: TransactionLongevity,
}

impl ValidTransactionParams {
	pub const fn new(
		tag_prefix: &'static str,
		priority: TransactionPriority,
		longevity: TransactionLongevity,
	) -> Self {
		Self { tag_prefix, priority, longevity }
	}

	pub fn provides(self, provides: impl Encode) -> ValidTransaction {
		ValidTransaction::with_tag_prefix(self.tag_prefix)
			.and_provides(provides)
			.priority(self.priority)
			.longevity(self.longevity)
			.into()
	}

	/// Pricing only, for calls with no dedup tag.
	pub fn untagged(self) -> ValidTransaction {
		ValidTransaction {
			priority: self.priority,
			longevity: self.longevity,
			..Default::default()
		}
	}
}

/// Panics if any two of `params` share a `tag_prefix`. Names are for the panic message.
pub fn assert_distinct_tag_prefixes(params: &[(&str, ValidTransactionParams)]) {
	for (i, (name, one)) in params.iter().enumerate() {
		for (other_name, other) in params.iter().skip(i + 1) {
			assert!(
				one.tag_prefix != other.tag_prefix,
				"{name} and {other_name} must not share the tag prefix `{}`: their pool tags \
				 would dedup against each other",
				one.tag_prefix,
			);
		}
	}
}

/// Identifies a previously-stored entry in the pallet's `Transactions` map.
#[derive(
	Clone,
	PartialEq,
	Eq,
	Debug,
	Encode,
	Decode,
	codec::DecodeWithMemTracking,
	TypeInfo,
	MaxEncodedLen,
)]
pub enum TransactionRef<BlockNumber> {
	Position { block: BlockNumber, index: u32 },
	ContentHash(ContentHash),
}

impl<BlockNumber> From<(BlockNumber, u32)> for TransactionRef<BlockNumber> {
	fn from((block, index): (BlockNumber, u32)) -> Self {
		Self::Position { block, index }
	}
}

impl<BlockNumber> From<ContentHash> for TransactionRef<BlockNumber> {
	fn from(hash: ContentHash) -> Self {
		Self::ContentHash(hash)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const A: ValidTransactionParams = ValidTransactionParams::new("a", 1, 1);
	const B: ValidTransactionParams = ValidTransactionParams::new("b", 2, 2);
	// Same prefix, different pricing: only the prefix matters.
	const A_AGAIN: ValidTransactionParams = ValidTransactionParams::new("a", 3, 3);

	#[test]
	fn distinct_tag_prefixes_pass() {
		assert_distinct_tag_prefixes(&[("A", A), ("B", B)]);
	}

	#[test]
	#[should_panic(expected = "A and A_AGAIN must not share the tag prefix `a`")]
	fn shared_tag_prefix_panics() {
		assert_distinct_tag_prefixes(&[("A", A), ("B", B), ("A_AGAIN", A_AGAIN)]);
	}

	/// The containing `TransactionInfo` layout is pinned by the renewal pallet's
	/// `transaction_info_encoding_matches_pre_split_layout`.
	#[test]
	fn entry_kind_encoding_is_frozen() {
		assert_eq!(EntryKind::Store.encode(), vec![0u8]);
		assert_eq!(EntryKind::Renew.encode(), vec![1u8]);
		assert_eq!(EntryKind::default(), EntryKind::Store);
	}
}
