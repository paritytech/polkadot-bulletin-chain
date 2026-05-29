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

//! Storage relocation migration for the data-renewal pallet.
//!
//! `AutoRenewals` and `PendingAutoRenewals` previously lived under the
//! `TransactionStorage` pallet prefix. After the renewal-pallet split they live
//! under the `DataRenewal` prefix. This migration relocates existing entries on
//! a single runtime upgrade.
//!
//! `PermanentStorageUsed` stayed in the storage pallet for E1, so only the two
//! items above need relocation.

extern crate alloc;

use crate::Config;
use polkadot_sdk_frame::deps::{
	frame_support::{
		migration::move_prefix,
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

		// `AutoRenewals` (StorageMap): move all keys under the old `(pallet, item)`
		// prefix to the new prefix. `move_prefix` walks the trie under the source
		// prefix and re-keys every entry without decoding the value, so the layout
		// of `RenewalData` is irrelevant here.
		let old_auto_prefix = storage_prefix(OLD_PALLET, b"AutoRenewals");
		let new_auto_prefix = storage_prefix(NEW_PALLET, b"AutoRenewals");
		move_prefix(&old_auto_prefix, &new_auto_prefix);

		// `PendingAutoRenewals` (StorageValue): exactly one key under each prefix.
		let old_pending_key = storage_prefix(OLD_PALLET, b"PendingAutoRenewals");
		let new_pending_key = storage_prefix(NEW_PALLET, b"PendingAutoRenewals");
		if let Some(raw) = sp_io::storage::get(&old_pending_key) {
			sp_io::storage::set(&new_pending_key, &raw);
			sp_io::storage::clear(&old_pending_key);
		}

		StorageVersion::new(1).put::<crate::pallet::Pallet<T>>();

		tracing::info!(target: LOG_TARGET, "relocation complete");

		// Conservative: assume a small bounded number of entries moved.
		T::DbWeight::get().reads_writes(2, 2)
	}

	#[cfg(feature = "try-runtime")]
	fn pre_upgrade(
	) -> Result<alloc::vec::Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
		use codec::Encode;
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
		use codec::Decode;
		use polkadot_sdk_frame::prelude::ensure;
		let pre = u64::decode(&mut &state[..]).map_err(|_| "pre_upgrade state decode failed")?;

		let new_auto_prefix = storage_prefix(NEW_PALLET, b"AutoRenewals");
		let mut previous = new_auto_prefix.to_vec();
		let mut post: u64 = 0;
		while let Some(key) =
			sp_io::storage::next_key(&previous).filter(|k| k.starts_with(&new_auto_prefix))
		{
			previous = key;
			post = post.saturating_add(1);
		}
		ensure!(post == pre, "AutoRenewals entry count changed across migration");

		let current = <crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
		ensure!(current >= StorageVersion::new(1), "storage version must be >= 1 after migration");
		Ok(())
	}
}
