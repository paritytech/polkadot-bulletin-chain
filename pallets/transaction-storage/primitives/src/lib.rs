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

/// A [`ValidTransaction`] minus its `provides` payload.
///
/// Transactions sharing a `tag_prefix` *and* a `provides` tag conflict, so families that
/// must not evict each other need distinct prefixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

	/// Same pricing under a different dedup prefix.
	pub const fn with_tag_prefix(self, tag_prefix: &'static str) -> Self {
		Self { tag_prefix, ..self }
	}

	pub const fn with_priority(self, priority: TransactionPriority) -> Self {
		Self { priority, ..self }
	}

	pub fn provides(&self, provides: impl Encode) -> ValidTransaction {
		ValidTransaction::with_tag_prefix(self.tag_prefix)
			.and_provides(provides)
			.priority(self.priority)
			.longevity(self.longevity)
			.into()
	}

	/// Pricing only, for calls that need no dedup tag.
	pub fn untagged(&self) -> ValidTransaction {
		ValidTransaction {
			priority: self.priority,
			longevity: self.longevity,
			..Default::default()
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
