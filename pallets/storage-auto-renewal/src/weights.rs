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

//! Weights for `pallet-storage-auto-renewal`.
//!
//! Placeholder weights identical to the pre-split values for the auto-renewal
//! extrinsics that previously lived in `pallet-transaction-storage`. Replace
//! with benchmarked values once the runtime benchmarks are run on this pallet.

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use core::marker::PhantomData;
use polkadot_sdk_frame::weights_prelude::*;

pub trait WeightInfo {
	fn enable_auto_renew() -> Weight;
	fn disable_auto_renew() -> Weight;
	fn process_auto_renewals(n: u32) -> Weight;
}

/// Weights for `pallet-storage-auto-renewal` using the Substrate node and
/// recommended hardware.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	// TODO: update weights once benchmarked.
	fn enable_auto_renew() -> Weight {
		Weight::from_parts(10_000_000, 1_000)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	// TODO: update weights once benchmarked.
	fn disable_auto_renew() -> Weight {
		Weight::from_parts(10_000_000, 1_000)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	// TODO: update weights once benchmarked.
	fn process_auto_renewals(n: u32) -> Weight {
		// Per-item: 1 read (Authorizations) + 1 write (Authorizations) +
		// 1 write (BlockTransactions) + 1 write (TransactionByContentHash).
		Weight::from_parts(100_000_000, 40351)
			.saturating_add(T::DbWeight::get().reads(2_u64).saturating_mul(n as u64))
			.saturating_add(T::DbWeight::get().writes(3_u64).saturating_mul(n as u64))
	}
}

// For backwards compatibility and tests.
impl WeightInfo for () {
	fn enable_auto_renew() -> Weight {
		Weight::from_parts(10_000_000, 1_000)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn disable_auto_renew() -> Weight {
		Weight::from_parts(10_000_000, 1_000)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn process_auto_renewals(n: u32) -> Weight {
		Weight::from_parts(100_000_000, 40351)
			.saturating_add(RocksDbWeight::get().reads(2_u64).saturating_mul(n as u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64).saturating_mul(n as u64))
	}
}
