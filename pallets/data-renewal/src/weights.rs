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

//! Weights for the data-renewal pallet.
//!
//! Auto-generated weights are produced by the benchmarking pipeline and wired in
//! `runtimes/bulletin-westend/src/weights/pallet_bulletin_data_renewal.rs`. The
//! defaults here are placeholders that mirror the renewal-related entries from
//! `pallet-bulletin-transaction-storage`'s `WeightInfo` until benchmarks land.

use polkadot_sdk_frame::deps::frame_support::weights::Weight;

pub trait WeightInfo {
	fn renew() -> Weight;
	fn force_renew() -> Weight;
	fn enable_auto_renew() -> Weight;
	fn disable_auto_renew() -> Weight;
	/// Weight charged by the renewal extension during signed renew-call validation
	/// (authorization lookup + per-account `bytes_permanent` + chain-wide
	/// `PermanentStorageUsed` checks). Returned from
	/// [`crate::extension::ValidateRenewalCalls::weight`].
	fn validate_renew() -> Weight;
	/// Drain `n` pending auto-renewals (linear in `n`).
	fn process_pending_renewals(n: u32) -> Weight;
	/// One outer-loop iteration of the v3→v4 `AutoRenewals` layout migration
	/// (re-encoding one entry from the pre-`recurring` shape). Used by the
	/// multi-block migration; kept here as a placeholder for the eventual port
	/// from the pre-split storage pallet — currently unused because the
	/// migration has run on every live deployment.
	fn migrate_v3_to_v4_step() -> Weight;
}

impl WeightInfo for () {
	fn renew() -> Weight {
		Weight::zero()
	}
	fn force_renew() -> Weight {
		Weight::zero()
	}
	fn enable_auto_renew() -> Weight {
		Weight::zero()
	}
	fn disable_auto_renew() -> Weight {
		Weight::zero()
	}
	fn validate_renew() -> Weight {
		Weight::zero()
	}
	fn process_pending_renewals(_n: u32) -> Weight {
		Weight::zero()
	}
	fn migrate_v3_to_v4_step() -> Weight {
		Weight::zero()
	}
}
