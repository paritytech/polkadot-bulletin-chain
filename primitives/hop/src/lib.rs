// This file is part of Substrate.

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

//! HOP (Hand-Off Protocol) primitives.
//!
//! Contains the runtime API trait for HOP — authorization checks and promotion
//! of ephemeral pool data to on-chain storage.
//!
//! TODO: once the upstream version from
//! https://github.com/paritytech/polkadot-sdk/pull/11960 lands, drop this local
//! copy and depend on it directly.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

sp_api::decl_runtime_apis! {
	/// Runtime API for HOP.
	///
	/// Runtimes that support HOP implement this API so the node can check
	/// authorization and promote near-expiry pool entries to on-chain storage.
	pub trait HopRuntimeApi<AccountId> where AccountId: codec::Codec {
		/// Whether `who` may submit a HOP blob of `data_len` bytes for promotion.
		fn can_account_promote(who: AccountId, data_len: u32) -> bool;
		/// Construct a general transaction extrinsic for promoting HOP data.
		///
		/// The submitter's `(signer, signature)` over `signing_payload(data, submit_timestamp)`
		/// (defined in `pallet-hop-promotion` and mirrored byte-for-byte in `sc-hop`) is carried
		/// as call arguments so the runtime pallet can verify consent on-chain. `submit_timestamp`
		/// is the wall-clock time (ms since unix epoch) at which the user signed; the pallet
		/// rejects promotions whose timestamp is too far from the current block time.
		fn create_promotion_extrinsic(
			data: alloc::vec::Vec<u8>,
			signer: sp_runtime::MultiSigner,
			signature: sp_runtime::MultiSignature,
			submit_timestamp: u64,
		) -> Block::Extrinsic;
		/// Maximum data size per promotion extrinsic.
		fn max_promotion_size() -> u32;
	}
}
