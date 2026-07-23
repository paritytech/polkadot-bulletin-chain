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

/// Per-entry `EntryMeta` wired into the storage pallet: marks entries created by
/// `renew`/auto-renewal as [`EntryKind::Renew`] so `handle_obsolete` can decrement
/// the chain-wide renewed-byte counter when they age out.
///
/// INVARIANT: must stay identical (variant names AND 1-byte encoding, `Store = 0`,
/// `Renew = 1`) to the storage pallet's retired `TransactionKind` — live `Transactions`
/// entries written before the split decode through this type without migration, and
/// metadata consumers see an unchanged enum. Locked by the
/// `entry_kind_encoding_is_frozen` test.
#[derive(
	Copy, Clone, Debug, PartialEq, Eq, Default, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen,
)]
pub enum EntryKind {
	/// Created by `store` (temporary storage); ages out silently.
	#[default]
	#[codec(index = 0)]
	Store,
	/// Created by `renew`/auto-renewal (permanent storage); counted in the chain-wide
	/// renewed-byte counter.
	#[codec(index = 1)]
	Renew,
}

/// Per-authorization `AuthorizationExtra` wired into the storage pallet: this pallet's
/// per-window renew quota, gated against the shared `bytes_allowance`
/// (`bytes_permanent + size <= bytes_allowance`). Reset to `0` with the other extent
/// counters when an expired authorization is re-granted; never decremented — the
/// chain-wide `PermanentStorageUsed` counter is the source of truth for renewed
/// on-chain bytes.
#[derive(
	Copy, Clone, Debug, PartialEq, Eq, Default, Encode, Decode, scale_info::TypeInfo, MaxEncodedLen,
)]
pub struct PermanentExtent {
	/// Bytes consumed by `renew` calls (permanent storage) within the current window.
	pub bytes_permanent: u64,
}

/// Auto-renewal registration value stored in [`crate::Renewals`].
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
