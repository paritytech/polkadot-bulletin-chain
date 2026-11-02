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

//! Storage migrations for the data-renewal pallet.
//!
//! The one-shot split migration that relocated `AutoRenewals`, `PendingAutoRenewals` and
//! `PermanentStorageUsed` out of the `TransactionStorage` prefix has run on every deployed
//! chain and was removed; see git history.

extern crate alloc;

use crate::{Config, Pallet, RenewalData, Renewals};
use bulletin_transaction_storage_primitives::ContentHash;
use codec::{Decode, Encode, MaxEncodedLen};
use pallet_bulletin_transaction_storage as txs;

use polkadot_sdk_frame::deps::frame_support::{
	migrations::VersionedMigration,
	pallet_prelude::PhantomData,
	traits::{Get, UncheckedOnRuntimeUpgrade},
	weights::Weight,
};

const LOG_TARGET: &str = "runtime::data-renewal::migrations";

/// Entry budget for a single-block `Renewals` scan.
///
/// A tripwire, not a hard limit: `pre_upgrade` fails outright when the map exceeds it, so a
/// dry-run catches the problem before the runtime ships, and `on_runtime_upgrade` logs an
/// error.
pub const MAX_SINGLE_BLOCK_ENTRIES: u64 = 5_000;

/// V1 → V2: `RenewalData` gains `size`, backfilled from the registration's on-chain target.
///
/// Per entry: `Renewals` read + write (via `translate`), `TransactionByContentHash` read,
/// `Transactions[block]` read. A registration whose target is gone keeps `size: 0`; such an
/// entry can never fire (the obsolete sweep only queues entries it finds), and a zero refund
/// on disable only over-counts `PermanentStorageUsed`, which rejects early rather than
/// admitting too much.
pub mod v2 {
	use super::*;

	/// `RenewalData` layout at v1.
	#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq, MaxEncodedLen)]
	pub struct V1RenewalData<AccountId> {
		pub account: AccountId,
		pub recurring: bool,
		pub paid: bool,
	}

	/// Size of `hash`'s latest on-chain entry. Shape-tolerant to pre-v3 block layouts.
	pub(crate) fn target_size<T: Config>(hash: ContentHash) -> Option<u32> {
		let (block, index) = txs::Pallet::<T>::lookup_by_content_hash(hash)?;
		txs::Pallet::<T>::transactions_at(block)?
			.get(index as usize)
			.map(|info| info.size)
	}

	pub struct VersionUncheckedMigrateV1ToV2<T>(PhantomData<T>);

	impl<T: Config> UncheckedOnRuntimeUpgrade for VersionUncheckedMigrateV1ToV2<T> {
		fn on_runtime_upgrade() -> Weight {
			let mut migrated: u64 = 0;
			let mut missing: u64 = 0;
			Renewals::<T>::translate::<V1RenewalData<T::AccountId>, _>(|hash, old| {
				migrated = migrated.saturating_add(1);
				let size = target_size::<T>(hash).unwrap_or_else(|| {
					missing = missing.saturating_add(1);
					tracing::warn!(
						target: LOG_TARGET,
						?hash,
						"v1->v2: registration has no on-chain target; size = 0",
					);
					0
				});
				Some(RenewalData {
					account: old.account,
					recurring: old.recurring,
					paid: old.paid,
					size,
				})
			});

			if migrated > MAX_SINGLE_BLOCK_ENTRIES {
				tracing::error!(
					target: LOG_TARGET,
					migrated,
					budget = MAX_SINGLE_BLOCK_ENTRIES,
					"single-block entry budget exceeded; block may exceed its weight or PoV limit",
				);
			}
			tracing::info!(target: LOG_TARGET, migrated, missing, "v1->v2 migration complete");

			T::DbWeight::get().reads_writes(migrated.saturating_mul(3), migrated)
		}

		#[cfg(feature = "try-runtime")]
		fn pre_upgrade(
		) -> Result<alloc::vec::Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			use polkadot_sdk_frame::{
				deps::{frame_support::storage::StoragePrefixedMap, sp_io},
				prelude::ensure,
			};

			// `translate` silently skips values it cannot decode, so prove every raw value
			// is v1 before the upgrade rather than discovering a vanished entry after it.
			let prefix = Renewals::<T>::final_prefix();
			let mut previous = prefix.to_vec();
			let mut count: u64 = 0;
			let mut missing: u64 = 0;
			while let Some(key) =
				sp_io::storage::next_key(&previous).filter(|k| k.starts_with(&prefix))
			{
				previous = key.clone();
				let raw = sp_io::storage::get(&key).ok_or("Renewals key without value")?;
				V1RenewalData::<T::AccountId>::decode(&mut &raw[..])
					.map_err(|_| "Renewals entry is not in the v1 layout")?;
				// The content hash is the Blake2_128Concat key suffix.
				let hash: ContentHash = key[key.len() - 32..]
					.try_into()
					.map_err(|_| "Renewals key shorter than a content hash")?;
				if target_size::<T>(hash).is_none() {
					missing = missing.saturating_add(1);
				}
				count = count.saturating_add(1);
			}

			// Fail the dry-run rather than the block: the migration cannot be stepped.
			ensure!(count <= MAX_SINGLE_BLOCK_ENTRIES, "Renewals exceeds the single-block budget");
			tracing::info!(target: LOG_TARGET, count, missing, "v1->v2 pre_upgrade");
			Ok((count, missing).encode())
		}

		#[cfg(feature = "try-runtime")]
		fn post_upgrade(
			state: alloc::vec::Vec<u8>,
		) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			use polkadot_sdk_frame::{
				deps::frame_support::storage::StoragePrefixedMap, prelude::ensure,
			};

			let (pre_count, pre_missing) = <(u64, u64)>::decode(&mut &state[..])
				.map_err(|_| "pre_upgrade state decode failed")?;

			// Raw key count unchanged, and every entry decodes as v2.
			let raw_count = count_keys(&Renewals::<T>::final_prefix());
			ensure!(raw_count == pre_count, "Renewals raw key count changed across migration");
			let mut decoded: u64 = 0;
			let mut missing: u64 = 0;
			for (hash, entry) in Renewals::<T>::iter() {
				decoded = decoded.saturating_add(1);
				match target_size::<T>(hash) {
					Some(size) => ensure!(entry.size == size, "size != on-chain target size"),
					None => {
						missing = missing.saturating_add(1);
						ensure!(entry.size == 0, "entry without target must have size 0");
					},
				}
			}
			ensure!(decoded == pre_count, "Renewals entry failed to decode as v2");
			ensure!(missing == pre_missing, "missing-target count changed across migration");
			tracing::info!(target: LOG_TARGET, decoded, missing, "v1->v2 post_upgrade: valid");
			Ok(())
		}
	}

	/// Versioned migration v1→v2: adds `size` to `RenewalData`.
	pub type MigrateV1ToV2<T> = VersionedMigration<
		1,
		2,
		VersionUncheckedMigrateV1ToV2<T>,
		Pallet<T>,
		<T as polkadot_sdk_frame::deps::frame_system::Config>::DbWeight,
	>;
}

/// Number of keys under `prefix`.
#[cfg(feature = "try-runtime")]
fn count_keys(prefix: &[u8]) -> u64 {
	use polkadot_sdk_frame::deps::sp_io;
	let mut previous = prefix.to_vec();
	let mut count: u64 = 0;
	while let Some(key) = sp_io::storage::next_key(&previous).filter(|k| k.starts_with(prefix)) {
		previous = key;
		count = count.saturating_add(1);
	}
	count
}
