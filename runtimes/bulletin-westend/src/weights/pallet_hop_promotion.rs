// Copyright (C) Parity Technologies and the various Polkadot contributors, see Contributions.md
// for a list of specific contributors.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Hand-crafted placeholder weights for `pallet_hop_promotion`.
//!
//! Conservative upper bound for the `authorize_promote` validation path.
//! Regenerate with measured values via:
//!
//! ```text
//! python3 scripts/cmd/cmd.py bench --pallet pallet_hop_promotion
//! ```

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use frame_support::{traits::Get, weights::Weight};
use core::marker::PhantomData;

/// Weight functions for `pallet_hop_promotion`.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> pallet_hop_promotion::WeightInfo for WeightInfo<T> {
	/// Storage: `TransactionStorage::BlockTransactions` (r:1 w:0) (block-fullness check).
	/// Storage: `Timestamp::Now` (r:1 w:0).
	/// Storage: `TransactionStorage::Authorizations` (r:1 w:0).
	/// Constant component: 3 reads + sr25519 verify (~50µs) + small fixed hashing.
	/// Per-byte component: blake2_256 over `data` (~7_200 ps/byte; matches `store`).
	/// The range of component `d` is `[1, 2097152]`.
	fn authorize_promote(d: u32) -> Weight {
		// Conservative placeholder; replace via `cmd.py bench`.
		Weight::from_parts(70_000_000, 0)
			.saturating_add(Weight::from_parts(7_500, 0).saturating_mul(d.into()))
			.saturating_add(T::DbWeight::get().reads(3))
	}
}
