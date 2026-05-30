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

//! Weights for `pallet_bulletin_data_renewal`.
//!
//! Hand-seeded from the pre-split `pallet_bulletin_transaction_storage` weights
//! for the renewal-related dispatchables, which exercise the same on-chain code
//! paths (per-account auth lookup, `bytes_permanent` consumption, chain-wide
//! `PermanentStorageUsed` bump, `AutoRenewals` write, optional in-block renew
//! mechanics). Re-bench in CI once `frame-omni-bencher` is wired to the renewal
//! pallet — schema is unchanged.

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use core::marker::PhantomData;
use frame_support::{traits::Get, weights::Weight};

/// Weight functions for `pallet_bulletin_data_renewal`.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> pallet_bulletin_data_renewal::WeightInfo for WeightInfo<T> {
	/// Storage: `TransactionStorage::TransactionByContentHash` (r:1 w:0)
	/// Storage: `TransactionStorage::Transactions` (r:1 w:0)
	/// Storage: `DataRenewal::AutoRenewals` (r:1 w:1)
	fn renew() -> Weight {
		Weight::from_parts(26_543_000, 0)
			.saturating_add(Weight::from_parts(0, 47519))
			.saturating_add(T::DbWeight::get().reads(3))
			.saturating_add(T::DbWeight::get().writes(1))
	}

	/// Storage: `TransactionStorage::TransactionByContentHash` (r:1 w:1)
	/// Storage: `TransactionStorage::Transactions` (r:1 w:0)
	/// Storage: `TransactionStorage::BlockTransactions` (r:1 w:1)
	fn force_renew() -> Weight {
		Weight::from_parts(28_066_000, 0)
			.saturating_add(Weight::from_parts(0, 47519))
			.saturating_add(T::DbWeight::get().reads(3))
			.saturating_add(T::DbWeight::get().writes(2))
	}

	/// Storage: `DataRenewal::AutoRenewals` (r:1 w:1)
	/// Storage: `TransactionStorage::TransactionByContentHash` (r:1 w:0)
	/// Storage: `TransactionStorage::Transactions` (r:1 w:0)
	fn enable_auto_renew() -> Weight {
		Weight::from_parts(27_275_000, 0)
			.saturating_add(Weight::from_parts(0, 47519))
			.saturating_add(T::DbWeight::get().reads(3))
			.saturating_add(T::DbWeight::get().writes(1))
	}

	/// Storage: `DataRenewal::AutoRenewals` (r:1 w:1)
	fn disable_auto_renew() -> Weight {
		Weight::from_parts(18_666_000, 0)
			.saturating_add(Weight::from_parts(0, 3547))
			.saturating_add(T::DbWeight::get().reads(1))
			.saturating_add(T::DbWeight::get().writes(1))
	}

	/// Storage: `TransactionStorage::Transactions` (r:1 w:0)
	/// Storage: `TransactionStorage::BlockTransactions` (r:1 w:0)
	/// Storage: `TransactionStorage::PermanentStorageUsed` (r:1 w:1)
	/// Storage: `TransactionStorage::Authorizations` (r:2 w:2)
	fn validate_renew() -> Weight {
		Weight::from_parts(44_737_000, 0)
			.saturating_add(Weight::from_parts(0, 47519))
			.saturating_add(T::DbWeight::get().reads(6))
			.saturating_add(T::DbWeight::get().writes(3))
	}

	/// Storage: `DataRenewal::PendingAutoRenewals` (r:1 w:1)
	/// Storage: `TransactionStorage::BlockTransactions` (r:1 w:1)
	/// Storage: `TransactionStorage::PermanentStorageUsed` (r:1 w:1)
	/// Storage: `TransactionStorage::Authorizations` (r:n w:n)
	/// Storage: `TransactionStorage::TransactionByContentHash` (r:0 w:n)
	/// Storage: `DataRenewal::AutoRenewals` (r:0 w:n)
	/// The range of component `n` is `[0, 512]`.
	fn process_pending_renewals(n: u32) -> Weight {
		Weight::from_parts(40_000_000, 0)
			.saturating_add(Weight::from_parts(0, 79311))
			.saturating_add(Weight::from_parts(15_295_543, 0).saturating_mul(n.into()))
			.saturating_add(T::DbWeight::get().reads(3))
			.saturating_add(T::DbWeight::get().reads((1_u64).saturating_mul(n.into())))
			.saturating_add(T::DbWeight::get().writes(3))
			.saturating_add(T::DbWeight::get().writes((2_u64).saturating_mul(n.into())))
			.saturating_add(Weight::from_parts(0, 2560).saturating_mul(n.into()))
	}
}
