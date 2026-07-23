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

//! One-shot relocation: moves `AutoRenewals`, `PendingAutoRenewals`, and
//! `PermanentStorageUsed` from the legacy `TransactionStorage::*` storage prefix to
//! `DataRenewal::*`, plus the v1→v2 `Authorizations` reshape.

extern crate alloc;

use crate::{Config, RenewalData};
use codec::{Decode, Encode};
use pallet_bulletin_transaction_storage as txs;
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
const NEW_PALLET: &[u8] = b"DataRenewal";

/// One-shot migration relocating `AutoRenewals`, `PendingAutoRenewals`, and the
/// `PermanentStorageUsed` counter from the `TransactionStorage` pallet prefix to the
/// `DataRenewal` pallet prefix. Bumps the renewal pallet's storage version from 0 to 1.
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
		let new_pending_key = storage_prefix(NEW_PALLET, b"PendingAutoRenewals");
		if let Some(raw) = sp_io::storage::get(&old_pending_key) {
			sp_io::storage::set(&new_pending_key, &raw);
			sp_io::storage::clear(&old_pending_key);
		}

		// `PermanentStorageUsed` (StorageValue<u64>): the chain-wide renewed-byte
		// counter moved into this pallet with the split. Move verbatim if present.
		let old_used_key = storage_prefix(OLD_PALLET, b"PermanentStorageUsed");
		let new_used_key = storage_prefix(NEW_PALLET, b"PermanentStorageUsed");
		if let Some(raw) = sp_io::storage::get(&old_used_key) {
			sp_io::storage::set(&new_used_key, &raw);
			sp_io::storage::clear(&old_used_key);
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
		let old_used = sp_io::storage::get(&storage_prefix(OLD_PALLET, b"PermanentStorageUsed"))
			.and_then(|raw| u64::decode(&mut &raw[..]).ok());
		Ok((count, old_used).encode())
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(
		state: alloc::vec::Vec<u8>,
	) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
		use polkadot_sdk_frame::prelude::ensure;
		let (pre, pre_used) = <(u64, Option<u64>)>::decode(&mut &state[..])
			.map_err(|_| "pre_upgrade state decode failed")?;

		// Every relocated entry must live under the new prefix and decode as the
		// current `RenewalData` layout (catches a pre-v4 entry that wasn't reshaped).
		let new_auto_prefix = storage_prefix(NEW_PALLET, b"Renewals");
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

		let current = <crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
		ensure!(current >= StorageVersion::new(1), "storage version must be >= 1 after migration");
		Ok(())
	}
}

/// V1 → V2 migration: reshapes every `TransactionStorage::Authorizations` value from
/// the pre-split layout (`bytes_permanent` inline in the extent) to the
/// `AuthorizationExtra` layout, **moving** `bytes_permanent` into this pallet's
/// [`PermanentExtent`]. Gated on this pallet's storage version — independent of the
/// storage pallet's version chain, so it cannot be skipped behind its v2→v3 MBM.
///
/// Must run single-block in the upgrade block: the old and new `Authorization`
/// encodings are the same byte length with all fixed-width fields, so a stale value
/// read through the new type decodes *successfully* with shifted fields. The
/// `Authorizations` map is admin-scale (authorizer-granted accounts + preimages).
pub mod v2 {
	use super::*;
	use crate::PermanentExtent;
	use pallet_bulletin_transaction_storage as txs;
	use polkadot_sdk_frame::prelude::BlockNumberFor;

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

	pub struct MigrateAuthorizationsExtra<T: Config>(PhantomData<T>);

	impl<T: Config> OnRuntimeUpgrade for MigrateAuthorizationsExtra<T> {
		fn on_runtime_upgrade() -> Weight {
			let current =
				<crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
			if current >= StorageVersion::new(2) {
				tracing::info!(target: LOG_TARGET, "authorizations already reshaped; skipping");
				return T::DbWeight::get().reads(1);
			}

			let mut migrated: u64 = 0;
			txs::Authorizations::<T>::translate::<OldAuthorization<BlockNumberFor<T>>, _>(
				|_scope, old| {
					migrated = migrated.saturating_add(1);
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
			StorageVersion::new(2).put::<crate::pallet::Pallet<T>>();

			tracing::info!(target: LOG_TARGET, migrated, "authorizations reshape complete");
			T::DbWeight::get().reads_writes(migrated.saturating_add(1), migrated.saturating_add(1))
		}

		#[cfg(feature = "try-runtime")]
		fn pre_upgrade(
		) -> Result<alloc::vec::Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			// Σ bytes_permanent + entry count under the OLD layout (0/0 when already
			// migrated — the gate skips and post_upgrade only checks decodability).
			let current =
				<crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
			if current >= StorageVersion::new(2) {
				return Ok((0u64, 0u64).encode());
			}
			let mut count: u64 = 0;
			let mut sum: u64 = 0;
			for key in txs::Authorizations::<T>::iter_keys() {
				let raw_key = txs::Authorizations::<T>::hashed_key_for(&key);
				let raw = sp_io::storage::get(&raw_key).ok_or("authorization value missing")?;
				let decoded = OldAuthorization::<BlockNumberFor<T>>::decode(&mut &raw[..])
					.map_err(|_| "pre-migration authorization is not the old layout")?;
				count = count.saturating_add(1);
				sum = sum.saturating_add(decoded.extent.bytes_permanent);
			}
			Ok((count, sum).encode())
		}

		#[cfg(feature = "try-runtime")]
		fn post_upgrade(
			state: alloc::vec::Vec<u8>,
		) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			use polkadot_sdk_frame::prelude::ensure;
			let (pre_count, pre_sum) = <(u64, u64)>::decode(&mut &state[..])
				.map_err(|_| "pre_upgrade state decode failed")?;

			let mut post_count: u64 = 0;
			let mut post_sum: u64 = 0;
			for (_, authorization) in txs::Authorizations::<T>::iter() {
				post_count = post_count.saturating_add(1);
				post_sum = post_sum.saturating_add(authorization.extent.extra.bytes_permanent);
			}
			if pre_count > 0 {
				ensure!(post_count == pre_count, "Authorizations entry count changed");
				ensure!(post_sum == pre_sum, "Σ bytes_permanent not preserved across reshape");
			}
			let current =
				<crate::pallet::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
			ensure!(current >= StorageVersion::new(2), "storage version must be >= 2");
			Ok(())
		}
	}
}
