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

//! One-shot split migration: moves `AutoRenewals`, `PendingAutoRenewals`, and
//! `PermanentStorageUsed` from the legacy `TransactionStorage::*` storage prefix to
//! `DataRenewal::*`, and reshapes `TransactionStorage::Authorizations` to the
//! `AuthorizationExtra` layout.

extern crate alloc;

use crate::{Config, PermanentExtent, RenewalData};
use codec::{Decode, Encode};
use pallet_bulletin_transaction_storage as txs;
use polkadot_sdk_frame::prelude::BlockNumberFor;

use polkadot_sdk_frame::deps::{
	frame_support::{
		pallet_prelude::PhantomData,
		storage::{storage_prefix, StoragePrefixedMap},
		traits::{Get, GetStorageVersion, OnRuntimeUpgrade, PalletInfoAccess, StorageVersion},
		weights::Weight,
	},
	sp_io,
};

const LOG_TARGET: &str = "runtime::data-renewal::migrations";

const OLD_PALLET: &[u8] = b"TransactionStorage";

/// One-shot migration relocating `AutoRenewals`, `PendingAutoRenewals`, and the
/// `PermanentStorageUsed` counter from the `TransactionStorage` pallet prefix to the
/// `DataRenewal` pallet prefix, and reshaping every `TransactionStorage::Authorizations`
/// value to the `AuthorizationExtra` layout — **moving** `bytes_permanent` into
/// [`PermanentExtent`]. Bumps the renewal pallet's storage version from 0 to 1.
///
/// Must run single-block: the old and new `Authorization` encodings are the same byte
/// length (all fixed-width fields), so a stale value read through the new type decodes
/// *successfully* with shifted fields. Idempotent via the storage-version gate.
pub struct RelocateFromTransactionStorage<T: Config>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for RelocateFromTransactionStorage<T> {
	fn on_runtime_upgrade() -> Weight {
		let current = <crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
		if current >= StorageVersion::new(1) {
			tracing::info!(target: LOG_TARGET, "already migrated; skipping");
			return Weight::zero();
		}

		// `AutoRenewals`: re-key from the old prefix, reshaping pre-v4 `{ account }`
		// values into the current `RenewalData` layout (a plain `move_prefix` would
		// leave them undecodable). The Blake2_128Concat key suffix is identical
		// across prefixes, so only the prefix is rewritten.
		let old_pallet = <txs::Pallet<T> as PalletInfoAccess>::name().as_bytes();
		let old_auto_prefix = storage_prefix(old_pallet, b"AutoRenewals");
		let new_auto_prefix = crate::Renewals::<T>::final_prefix();
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
		let new_pending_key = crate::PendingAutoRenewals::<T>::hashed_key();
		if let Some(raw) = sp_io::storage::get(&old_pending_key) {
			sp_io::storage::set(&new_pending_key, &raw);
			sp_io::storage::clear(&old_pending_key);
		}

		// `PermanentStorageUsed` (StorageValue<u64>): move verbatim if present.
		let old_used_key = storage_prefix(OLD_PALLET, b"PermanentStorageUsed");
		let new_used_key = storage_prefix(NEW_PALLET, b"PermanentStorageUsed");
		if let Some(raw) = sp_io::storage::get(&old_used_key) {
			sp_io::storage::set(&new_used_key, &raw);
			sp_io::storage::clear(&old_used_key);
		}

		// `Authorizations` reshape: `bytes_permanent` moves into the opaque `extra`.
		let mut reshaped: u64 = 0;
		txs::Authorizations::<T>::translate::<OldAuthorization<BlockNumberFor<T>>, _>(
			|_scope, old| {
				reshaped = reshaped.saturating_add(1);
				Some(txs::Authorization {
					extent: txs::AuthorizationExtent {
						transactions: old.extent.transactions,
						transactions_allowance: old.extent.transactions_allowance,
						bytes: old.extent.bytes,
						bytes_allowance: old.extent.bytes_allowance,
						extra: PermanentExtent { bytes_permanent: old.extent.bytes_permanent },
					},
					expiration: old.expiration,
				})
			},
		);

		StorageVersion::new(1).put::<crate::pallet::Pallet<T>>();

		tracing::info!(target: LOG_TARGET, moved, reshaped, "split migration complete");

		// One read + one write per moved `AutoRenewals` entry and per reshaped
		// `Authorizations` entry, plus the `PendingAutoRenewals` / counter moves and
		// the storage-version write.
		T::DbWeight::get().reads_writes(
			moved.saturating_add(reshaped).saturating_add(1),
			moved.saturating_mul(2).saturating_add(reshaped).saturating_add(2),
		)
	}

	#[cfg(feature = "try-runtime")]
	fn pre_upgrade(
	) -> Result<alloc::vec::Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
		// Mirror the runtime gate: already migrated → no-op, post checks skipped.
		let current = <crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
		if current >= StorageVersion::new(1) {
			return Ok(None::<(u64, Option<u64>, u64, u64)>.encode());
		}

		let old_auto_prefix = storage_prefix(OLD_PALLET, b"AutoRenewals");
		let mut previous = old_auto_prefix.to_vec();
		let mut count: u64 = 0;
		while let Some(key) =
			sp_io::storage::next_key(&previous).filter(|k| k.starts_with(&old_auto_prefix))
		{
			previous = key;
			count = count.saturating_add(1);
		}
		let old_used = sp_io::storage::get(&storage_prefix(OLD_PALLET, b"PermanentStorageUsed"))
			.and_then(|raw| u64::decode(&mut &raw[..]).ok());

		// Old-layout count + Σ bytes_permanent: the reshape must move, never zero.
		let mut auth_count: u64 = 0;
		let mut auth_perm_sum: u64 = 0;
		for key in txs::Authorizations::<T>::iter_keys() {
			let raw_key = txs::Authorizations::<T>::hashed_key_for(&key);
			let raw = sp_io::storage::get(&raw_key).ok_or("authorization value missing")?;
			let decoded = OldAuthorization::<BlockNumberFor<T>>::decode(&mut &raw[..])
				.map_err(|_| "pre-migration authorization is not the old layout")?;
			auth_count = auth_count.saturating_add(1);
			auth_perm_sum = auth_perm_sum.saturating_add(decoded.extent.bytes_permanent);
		}

		Ok(Some((count, old_used, auth_count, auth_perm_sum)).encode())
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(
		state: alloc::vec::Vec<u8>,
	) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
		use polkadot_sdk_frame::prelude::ensure;
		let Some((pre, pre_used, pre_auth_count, pre_auth_perm_sum)) =
			<Option<(u64, Option<u64>, u64, u64)>>::decode(&mut &state[..])
				.map_err(|_| "pre_upgrade state decode failed")?
		else {
			// Already migrated before this run — the gate made the migration a no-op;
			// only the version invariant is checkable.
			let current =
				<crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
			ensure!(current >= StorageVersion::new(1), "storage version must be >= 1");
			return Ok(());
		};

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
		ensure!(post == pre, "AutoRenewals entry count changed across migration");

		// No `AutoRenewals` must remain under the old `TransactionStorage` prefix.
		let old_auto_prefix = storage_prefix(OLD_PALLET, b"AutoRenewals");
		ensure!(
			sp_io::storage::next_key(&old_auto_prefix)
				.filter(|k| k.starts_with(&old_auto_prefix))
				.is_none(),
			"AutoRenewals entries remain under the old prefix after migration"
		);

		// The counter value captured under the old prefix must now live under the new
		// prefix, and the old key must be gone.
		if let Some(pre_used) = pre_used {
			ensure!(
				crate::PermanentStorageUsed::<T>::get() == pre_used,
				"PermanentStorageUsed value not preserved across relocation"
			);
		}
		ensure!(
			sp_io::storage::get(&storage_prefix(OLD_PALLET, b"PermanentStorageUsed")).is_none(),
			"PermanentStorageUsed remains under the old prefix after relocation"
		);

		// Reshape: every entry decodes as the new layout, count unchanged, and
		// Σ bytes_permanent preserved into `extra`.
		let mut post_auth_count: u64 = 0;
		let mut post_auth_perm_sum: u64 = 0;
		for (_, authorization) in txs::Authorizations::<T>::iter() {
			post_auth_count = post_auth_count.saturating_add(1);
			post_auth_perm_sum =
				post_auth_perm_sum.saturating_add(authorization.extent.extra.bytes_permanent);
		}
		ensure!(post_auth_count == pre_auth_count, "Authorizations entry count changed");
		ensure!(
			post_auth_perm_sum == pre_auth_perm_sum,
			"Σ bytes_permanent not preserved across the Authorizations reshape"
		);

		let current = <crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
		ensure!(current >= StorageVersion::new(1), "storage version must be >= 1 after migration");
		Ok(())
	}
}

/// `AuthorizationExtent` layout before the split (`bytes_permanent` inline).
#[derive(Encode, Decode)]
struct OldAuthorizationExtent {
	transactions: u32,
	transactions_allowance: u32,
	bytes: u64,
	bytes_permanent: u64,
	bytes_allowance: u64,
}

/// `Authorization` layout before the split.
#[derive(Encode, Decode)]
struct OldAuthorization<BlockNumber> {
	extent: OldAuthorizationExtent,
	expiration: BlockNumber,
}
