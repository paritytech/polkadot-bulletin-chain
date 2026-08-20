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

//! One-shot split migration: moves `AutoRenewals` and `PermanentStorageUsed` from the legacy
//! `TransactionStorage::*` storage prefix to `DataRenewal::*`.
//!
//! The legacy `PendingAutoRenewals` queue is not relocated: it is per-block scratch that the
//! pre-split `on_finalize` asserted was drained every block, so no value can survive into the
//! upgrade block. `pre_upgrade` asserts the key is absent.
//!
//! `TransactionStorage::Authorizations` is deliberately *not* touched:
//! `AuthorizationExtent::extra` occupies the slot the pre-split `bytes_permanent` field had,
//! so existing values already decode as the new layout.

extern crate alloc;

use crate::{Config, RenewalData};
use codec::{Decode, Encode};
use pallet_bulletin_transaction_storage as txs;

use polkadot_sdk_frame::deps::{
	frame_support::{
		pallet_prelude::PhantomData,
		storage::{storage_prefix, StoragePrefixedMap},
		traits::{Get, OnRuntimeUpgrade, PalletInfoAccess},
		weights::Weight,
	},
	sp_io,
};

const LOG_TARGET: &str = "runtime::data-renewal::migrations";

/// Entry budget for the single-block `AutoRenewals` scan.
///
/// Each entry costs 2 reads + 2 writes, so a few thousand is comfortably inside a parachain
/// block's ref-time and PoV budget while leaving room for the rest of the upgrade.
///
/// This is a tripwire, not a hard limit: `pre_upgrade` fails outright when the collection
/// exceeds it, so a dry-run catches the problem before the runtime ships, and
/// `on_runtime_upgrade` logs an error.
const MAX_SINGLE_BLOCK_ENTRIES: u64 = 5_000;

/// Source prefixes, derived from the storage pallet's registered name rather than a
/// literal: the runtime is free to register it under any name, and a mismatch here
/// would silently scan an empty prefix and migrate nothing — which would still bump the
/// storage version, permanently locking the migration out via the idempotency gate.
fn old_prefix<T: Config>(item: &[u8]) -> [u8; 32] {
	storage_prefix(<txs::Pallet<T> as PalletInfoAccess>::name().as_bytes(), item)
}

/// One-shot migration relocating `AutoRenewals` and the `PermanentStorageUsed` counter from
/// the `TransactionStorage` pallet prefix to the `DataRenewal` pallet prefix.
///
/// Idempotent by construction — every step is conditional on the *old* key still existing,
/// which is false on any later run. Deliberately not gated on a storage version: this pallet
/// is introduced by the same runtime upgrade that runs the migration, and FRAME initializes
/// a brand-new pallet's on-chain version to the in-code `STORAGE_VERSION` in
/// `before_all_runtime_migrations` — i.e. before any migration runs — so a version gate here
/// is always already satisfied and would skip the relocation entirely.
///
/// Runs single-block: `AutoRenewals` is bounded by `MAX_SINGLE_BLOCK_ENTRIES`, enforced by
/// `pre_upgrade`, and the counter is a single `StorageValue`.
pub struct RelocateFromTransactionStorage<T: Config>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for RelocateFromTransactionStorage<T> {
	fn on_runtime_upgrade() -> Weight {
		// `AutoRenewals`: re-key from the old prefix, reshaping pre-v4 `{ account }`
		// values into the current `RenewalData` layout (a plain `move_prefix` would
		// leave them undecodable). The Blake2_128Concat key suffix is identical
		// across prefixes, so only the prefix is rewritten.
		let old_auto_prefix = old_prefix::<T>(b"AutoRenewals");
		let new_auto_prefix = crate::Renewals::<T>::final_prefix();
		let mut moved: u64 = 0;
		// Every key the scan touches, moved or not — each cost a `next_key` + a `get`.
		let mut visited: u64 = 0;
		let mut previous = old_auto_prefix.to_vec();
		while let Some(key) =
			sp_io::storage::next_key(&previous).filter(|k| k.starts_with(&old_auto_prefix))
		{
			previous = key.clone();
			visited = visited.saturating_add(1);
			let Some(raw) = sp_io::storage::get(&key) else { continue };

			// Already current layout? carry the bytes over unchanged. Otherwise the
			// entry is the pre-v4 bare `AccountId` (`{ account }` is a single-field
			// struct, encoded identically) — rebuild it as recurring & prepaid.
			let value = if RenewalData::<T::AccountId>::decode(&mut &raw[..]).is_ok() {
				raw.to_vec()
			} else {
				match T::AccountId::decode(&mut &raw[..]) {
					Ok(account) => RenewalData { account, recurring: true, paid: false }.encode(),
					Err(_) => {
						tracing::error!(
							target: LOG_TARGET,
							"skipping undecodable AutoRenewals entry during relocation"
						);
						continue;
					},
				}
			};

			let mut new_key = new_auto_prefix.to_vec();
			new_key.extend_from_slice(&key[old_auto_prefix.len()..]);
			sp_io::storage::set(&new_key, &value);
			sp_io::storage::clear(&key);
			moved = moved.saturating_add(1);
		}

		// Number of `StorageValue`s actually relocated below; each costs a set + a clear.
		let mut values_moved: u64 = 0;

		// `PermanentStorageUsed` (StorageValue<u64>): move verbatim if present.
		let old_used_key = old_prefix::<T>(b"PermanentStorageUsed");
		let new_used_key = crate::PermanentStorageUsed::<T>::hashed_key();
		if let Some(raw) = sp_io::storage::get(&old_used_key) {
			sp_io::storage::set(&new_used_key, &raw);
			sp_io::storage::clear(&old_used_key);
			values_moved = values_moved.saturating_add(1);
		}

		if visited > MAX_SINGLE_BLOCK_ENTRIES {
			tracing::error!(
				target: LOG_TARGET,
				visited,
				budget = MAX_SINGLE_BLOCK_ENTRIES,
				"single-block entry budget exceeded; block may exceed its weight or PoV limit",
			);
		}

		tracing::info!(
			target: LOG_TARGET,
			moved,
			visited,
			values_moved,
			"split migration complete",
		);

		// Reads: a `next_key` + `get` per visited `AutoRenewals` key, the terminal `next_key`,
		// and the `PermanentStorageUsed` `get`.
		let reads = visited.saturating_mul(2).saturating_add(2);
		// Writes: a set + a clear per moved `AutoRenewals` entry and per relocated
		// `StorageValue`.
		let writes = moved.saturating_add(values_moved).saturating_mul(2);
		T::DbWeight::get().reads_writes(reads, writes)
	}

	#[cfg(feature = "try-runtime")]
	fn pre_upgrade(
	) -> Result<alloc::vec::Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
		use polkadot_sdk_frame::prelude::ensure;

		let renewals = count_keys(&old_prefix::<T>(b"AutoRenewals"));
		let already_relocated = count_keys(&crate::Renewals::<T>::final_prefix());
		let permanent_used = sp_io::storage::get(&old_prefix::<T>(b"PermanentStorageUsed"))
			.and_then(|raw| u64::decode(&mut &raw[..]).ok());

		// Tripwire for the assumption that lets the transient queue be dropped rather than
		// relocated. A leftover value means the drain invariant did not hold on chain and the
		// migration has to move it after all — fail the dry-run, not the block.
		ensure!(
			sp_io::storage::get(&old_prefix::<T>(b"PendingAutoRenewals")).is_none(),
			"legacy PendingAutoRenewals is non-empty; it must be relocated",
		);

		// Fail the dry-run rather than the block: the migration cannot be stepped, so an
		// oversized collection has to be caught before the runtime ships.
		ensure!(
			renewals <= MAX_SINGLE_BLOCK_ENTRIES,
			"AutoRenewals exceeds the single-block entry budget",
		);

		Ok(PreUpgradeState { renewals, already_relocated, permanent_used }.encode())
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(
		state: alloc::vec::Vec<u8>,
	) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
		use polkadot_sdk_frame::prelude::ensure;
		let pre = PreUpgradeState::decode(&mut &state[..])
			.map_err(|_| "pre_upgrade state decode failed")?;

		// Every relocated entry must live under the new prefix and decode as the
		// current `RenewalData` layout (catches a pre-v4 entry that wasn't reshaped).
		let new_auto_prefix = crate::Renewals::<T>::final_prefix();
		let mut previous = new_auto_prefix.to_vec();
		let mut post: u64 = 0;
		while let Some(key) =
			sp_io::storage::next_key(&previous).filter(|k| k.starts_with(&new_auto_prefix))
		{
			previous = key.clone();
			let raw =
				sp_io::storage::get(&key).ok_or("relocated AutoRenewals entry missing value")?;
			RenewalData::<T::AccountId>::decode(&mut &raw[..])
				.map_err(|_| "relocated AutoRenewals entry is not current RenewalData layout")?;
			post = post.saturating_add(1);
		}
		ensure!(
			post == pre.renewals.saturating_add(pre.already_relocated),
			"AutoRenewals entry count changed across migration"
		);

		// No `AutoRenewals` must remain under the old `TransactionStorage` prefix.
		let old_auto_prefix = old_prefix::<T>(b"AutoRenewals");
		ensure!(
			sp_io::storage::next_key(&old_auto_prefix)
				.filter(|k| k.starts_with(&old_auto_prefix))
				.is_none(),
			"AutoRenewals entries remain under the old prefix after migration"
		);

		// The counter value captured under the old prefix must now live under the new
		// prefix, and the old key must be gone.
		if let Some(pre_used) = pre.permanent_used {
			ensure!(
				crate::PermanentStorageUsed::<T>::get() == pre_used,
				"PermanentStorageUsed value not preserved across relocation"
			);
		}
		ensure!(
			sp_io::storage::get(&old_prefix::<T>(b"PermanentStorageUsed")).is_none(),
			"PermanentStorageUsed remains under the old prefix after relocation"
		);

		Ok(())
	}
}

/// State captured by `pre_upgrade` for `post_upgrade` to check against.
#[cfg(feature = "try-runtime")]
#[derive(Encode, Decode)]
struct PreUpgradeState {
	renewals: u64,
	/// Entries already under the new prefix. Non-zero on a chain that has run the
	/// relocation and taken renewals since — `post_upgrade` must not count those as
	/// missing.
	already_relocated: u64,
	permanent_used: Option<u64>,
}

/// Number of keys under `prefix`.
#[cfg(feature = "try-runtime")]
fn count_keys(prefix: &[u8]) -> u64 {
	let mut previous = prefix.to_vec();
	let mut count: u64 = 0;
	while let Some(key) = sp_io::storage::next_key(&previous).filter(|k| k.starts_with(prefix)) {
		previous = key;
		count = count.saturating_add(1);
	}
	count
}
