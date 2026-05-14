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

//! Benchmarks for `pallet-storage-auto-renewal`.
//!
//! Stub benchmarks that exercise the no-op happy path. Real benchmarks need a
//! `StorageRenewer` mock with on-chain transactions and authorizations; that work
//! is tracked separately. The placeholder weights in [`crate::weights`] are
//! intentionally conservative (constant-time, 1 read / 1 write) so the chain
//! mandatory-budget bound is honored until real numbers land.

#![cfg(feature = "runtime-benchmarks")]

use super::{Call, Config, Pallet, PendingAutoRenewals};
use polkadot_sdk_frame::{benchmarking::prelude::*, deps::frame_system::RawOrigin};

#[benchmarks]
mod benchmarks {
	use super::*;

	/// Process an empty queue. The block-author always emits the inherent when
	/// pending renewals are present, so this is the cheapest path.
	#[benchmark]
	fn process_auto_renewals(
		n: Linear<0, { <T as Config>::MaxBlockTransactions::get() }>,
	) -> Result<(), BenchmarkError> {
		let _ = n;
		assert!(PendingAutoRenewals::<T>::get().is_empty());

		#[extrinsic_call]
		_(RawOrigin::None);

		Ok(())
	}

	impl_benchmark_test_suite!(Pallet, crate::mock::new_test_ext(), crate::mock::Test);
}
