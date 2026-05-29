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

/// Auto-renewal registration value stored in [`crate::AutoRenewals`].
///
/// `recurring` distinguishes a forever-renewing entry (`enable_auto_renew`)
/// from a one-shot (`renew`). `paid` marks the prepaid first cycle: both
/// constructors set it to `true` (the extension's `pre_dispatch` charges the
/// slot at registration); the next cycle delivers free and flips it to `false`
/// for recurring entries. Signed `disable_auto_renew` is rejected while `paid`
/// is `true` — without this, an owner could pocket the prepaid slot.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen)]
pub struct RenewalData<AccountId> {
	/// Account whose authorization is consumed on each (non-prepaid) cycle.
	pub account: AccountId,
	/// `true` for `enable_auto_renew` (forever), `false` for `renew` (one-shot,
	/// removed after first cycle).
	pub recurring: bool,
	/// `true` while the prepaid first cycle hasn't fired yet.
	pub paid: bool,
}
