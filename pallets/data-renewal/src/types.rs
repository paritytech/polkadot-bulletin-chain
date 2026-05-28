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

//! Type definitions for the data-renewal pallet.

use codec::{Decode, Encode, MaxEncodedLen};

/// Data associated with a renewal registration in [`crate::AutoRenewals`].
///
/// Holds the owner account, a `recurring` flag that decides whether the registration
/// is consumed after a single successful renewal (`false`, set by [`crate::Pallet::renew`])
/// or persists forever (`true`, set by [`crate::Pallet::enable_auto_renew`]), and a
/// `paid` flag indicating that the next cycle has already been charged against the
/// owner's authorization at registration time.
///
/// Both `renew` and `enable_auto_renew` insert with `paid: true`: the renewal pallet's
/// extension charges `bytes_permanent`, the chain-wide `PermanentStorageUsed`, and one
/// tx slot up front (same as `force_renew`). `do_process_auto_renewals` keys its
/// charge-skip off `paid`: when `paid` is true the cycle renews without re-charging and
/// then flips `paid` to `false` (for recurring entries) so subsequent cycles pay
/// per-cycle. One-shot entries (`recurring: false`) are removed after the single
/// renewal so the flag is inert after that point.
///
/// While `paid` is true, `disable_auto_renew` rejects signed callers — the owner must
/// wait for the first cycle to consume the prepayment. This is what makes
/// `enable_auto_renew` honestly cost a renewal even if the owner immediately disables.
/// Root can still disable for governance cleanup.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
pub struct RenewalData<AccountId> {
	/// Account whose authorization will be consumed each time data is auto-renewed.
	pub account: AccountId,
	/// `true` — auto-renew forever (set by `enable_auto_renew`).
	/// `false` — one-shot: removed from `AutoRenewals` after the first successful renewal
	/// cycle (set by `renew`).
	pub recurring: bool,
	/// `true` — the next renewal cycle has already been charged at registration time and
	/// will fire free. After the cycle delivers, the flag is flipped to `false` for
	/// recurring entries; for one-shot entries the registration is removed outright.
	pub paid: bool,
}
