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

//! One-shot relocation: moves `AutoRenewals` and `PendingAutoRenewals` from
//! the legacy `TransactionStorage::*` storage prefix to `DataRenewal::*`.
//! `PermanentStorageUsed` continues to live in the storage pallet and needs no
//! relocation.

extern crate alloc;

use crate::{Config, RenewalData};
use codec::{Decode, Encode};
use polkadot_sdk_frame::deps::{
	frame_support::{
		pallet_prelude::PhantomData,
		storage::storage_prefix,
		traits::{Get, GetStorageVersion, OnRuntimeUpgrade, StorageVersion},
		weights::Weight,
	},
	sp_io,
};

const LOG_TARGET: &str = "runtime::data-renewal::migrations";

const OLD_PALLET: &[u8] = b"TransactionStorage";
const NEW_PALLET: &[u8] = b"DataRenewal";

/// One-shot migration relocating `AutoRenewals` and `PendingAutoRenewals` from the
/// `TransactionStorage` pallet prefix to the `DataRenewal` pallet prefix. Bumps the
/// renewal pallet's storage version from 0 to 1.
///
/// Idempotent: re-running after success is a no-op (storage version gate).
pub struct RelocateFromTransactionStorage<T: Config>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for RelocateFromTransactionStorage<T> {
	fn on_runtime_upgrade() -> Weight {
		let current = <crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
		if current >= StorageVersion::new(1) {
			tracing::info!(target: LOG_TARGET, "already migrated; skipping");
			return Weight::zero();
		}

		// `AutoRenewals` (StorageMap): re-key every entry from the old
		// `TransactionStorage` prefix to the new `DataRenewal` prefix, reshaping any
		// pre-v4 `{ account }` value into the current `RenewalData { account,
		// recurring, paid }` layout. A plain `move_prefix` would carry pre-v4 bytes
		// over verbatim and leave them undecodable under this pallet's type — see the
		// retired `transaction_storage::migrations::v4::MigrateV3ToV4` for the original
		// reshape. The Blake2_128Concat key suffix (the content hash) is identical
		// across prefixes, so only the prefix is rewritten.
		let old_auto_prefix = storage_prefix(OLD_PALLET, b"AutoRenewals");
		let new_auto_prefix = storage_prefix(NEW_PALLET, b"AutoRenewals");
		let mut moved: u64 = 0;
		let mut previous = old_auto_prefix.to_vec();
		while let Some(key) =
			sp_io::storage::next_key(&previous).filter(|k| k.starts_with(&old_auto_prefix))
		{
			previous = key.clone();
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

		// `PendingAutoRenewals` (StorageValue): transient per-block scratch, normally
		// empty across an upgrade. Move verbatim if present.
		let old_pending_key = storage_prefix(OLD_PALLET, b"PendingAutoRenewals");
		let new_pending_key = storage_prefix(NEW_PALLET, b"PendingAutoRenewals");
		if let Some(raw) = sp_io::storage::get(&old_pending_key) {
			sp_io::storage::set(&new_pending_key, &raw);
			sp_io::storage::clear(&old_pending_key);
		}

		StorageVersion::new(1).put::<crate::pallet::Pallet<T>>();

		tracing::info!(target: LOG_TARGET, moved, "relocation complete");

		// One read + one write per moved `AutoRenewals` entry, plus the
		// `PendingAutoRenewals` move and the storage-version write.
		T::DbWeight::get()
			.reads_writes(moved.saturating_add(1), moved.saturating_mul(2).saturating_add(2))
	}

	#[cfg(feature = "try-runtime")]
	fn pre_upgrade(
	) -> Result<alloc::vec::Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
		let old_auto_prefix = storage_prefix(OLD_PALLET, b"AutoRenewals");
		let mut previous = old_auto_prefix.to_vec();
		let mut count: u64 = 0;
		while let Some(key) =
			sp_io::storage::next_key(&previous).filter(|k| k.starts_with(&old_auto_prefix))
		{
			previous = key;
			count = count.saturating_add(1);
		}
		Ok(count.encode())
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(
		state: alloc::vec::Vec<u8>,
	) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
		use polkadot_sdk_frame::prelude::ensure;
		let pre = u64::decode(&mut &state[..]).map_err(|_| "pre_upgrade state decode failed")?;

		// Every relocated entry must live under the new prefix and decode as the
		// current `RenewalData` layout (catches a pre-v4 entry that wasn't reshaped).
		let new_auto_prefix = storage_prefix(NEW_PALLET, b"AutoRenewals");
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
		ensure!(post == pre, "AutoRenewals entry count changed across migration");

		// No `AutoRenewals` must remain under the old `TransactionStorage` prefix.
		let old_auto_prefix = storage_prefix(OLD_PALLET, b"AutoRenewals");
		ensure!(
			sp_io::storage::next_key(&old_auto_prefix)
				.filter(|k| k.starts_with(&old_auto_prefix))
				.is_none(),
			"AutoRenewals entries remain under the old prefix after migration"
		);

		let current = <crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
		ensure!(current >= StorageVersion::new(1), "storage version must be >= 1 after migration");
		Ok(())
	}
}
