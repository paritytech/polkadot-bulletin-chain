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

use crate::{Config, RetentionPeriod, LOG_TARGET};
use alloc::vec::Vec;
use codec::{Decode, Encode, MaxEncodedLen};
use core::marker::PhantomData;
use polkadot_sdk_frame::{
	prelude::{BlockNumberFor, Weight},
	traits::{Get, OnRuntimeUpgrade, Zero},
};

/// Runtime migration that sets the `RetentionPeriod` storage item to a
/// non-zero `NewValue` value **only if it is currently zero**.
///
/// Idempotent migration: safe to run multiple times
pub struct SetRetentionPeriodIfZero<T, NewValue>(PhantomData<(T, NewValue)>);
impl<T: Config, NewValue: Get<BlockNumberFor<T>>> OnRuntimeUpgrade
	for SetRetentionPeriodIfZero<T, NewValue>
{
	fn on_runtime_upgrade() -> Weight {
		let mut weight = T::DbWeight::get().reads(1);

		// If zero, let's reset.
		if RetentionPeriod::<T>::get().is_zero() {
			RetentionPeriod::<T>::set(NewValue::get());
			weight.saturating_accrue(T::DbWeight::get().writes(1));

			tracing::warn!(
				target: LOG_TARGET,
				new_value = ?NewValue::get(),
				"[SetRetentionPeriodIfZero] RetentionPeriod was zero, resetting to:",
			);
		}

		weight
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(
		_state: alloc::vec::Vec<u8>,
	) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::DispatchError> {
		polkadot_sdk_frame::prelude::ensure!(
			!RetentionPeriod::<T>::get().is_zero(),
			"must be migrate to the `NewValue`."
		);

		tracing::info!(target: LOG_TARGET, "SetRetentionPeriodIfZero is OK!");
		Ok(())
	}
}

/// Migration v2→v3: Adds `extrinsic_index` to `TransactionInfo`.
///
/// Also opportunistically prunes any `Transactions[block]` entries with
/// `block < current_block - RetentionPeriod` — stale leftovers from chains where the
/// retention window was previously longer than it is now (`on_initialize`'s aging-out
/// hook only drops one entry per block going forward; it does not catch up on
/// historical entries that became stale across a retention-period change).
pub mod v3 {
	use super::*;
	use crate::{
		pallet::{Pallet, Transactions},
		RetentionPeriod, TransactionInfo, TransactionInfoFor, WeightInfo,
	};
	use bulletin_transaction_storage_primitives::{
		cids::{CidCodec, HashingAlgorithm},
		ContentHash,
	};
	use polkadot_sdk_frame::deps::{
		frame_support::{
			migrations::{MigrationId, SteppedMigration, SteppedMigrationError},
			weights::WeightMeter,
			BoundedVec,
		},
		sp_io,
		sp_runtime::traits::{BlakeTwo256, Hash},
	};
	use sp_transaction_storage_proof::ChunkIndex;

	const MIGRATIONS_ID: &[u8; 24] = b"bulletin-tx-storage-vmig";

	/// `TransactionInfo` layout at v2 (no `extrinsic_index`). Used only for
	/// decoding pre-migration entries; never written.
	#[derive(Encode, Decode, Clone, Debug, MaxEncodedLen)]
	pub(crate) struct V2TransactionInfo {
		pub chunk_root: <BlakeTwo256 as Hash>::Output,
		pub content_hash: ContentHash,
		pub hashing: HashingAlgorithm,
		pub cid_codec: CidCodec,
		pub size: u32,
		pub block_chunks: ChunkIndex,
	}

	/// Stepped migration from storage version 2 to 3.
	pub struct MigrateV2ToV3<T: Config>(PhantomData<T>);

	impl<T: Config> SteppedMigration for MigrateV2ToV3<T> {
		type Cursor = polkadot_sdk_frame::prelude::BlockNumberFor<T>;
		type Identifier = MigrationId<24>;

		fn id() -> Self::Identifier {
			MigrationId { pallet_id: *MIGRATIONS_ID, version_from: 2, version_to: 3 }
		}

		fn step(
			mut cursor: Option<Self::Cursor>,
			meter: &mut WeightMeter,
		) -> Result<Option<Self::Cursor>, SteppedMigrationError> {
			use polkadot_sdk_frame::prelude::Saturating;

			let required = T::WeightInfo::migrate_v2_to_v3_step();
			if meter.remaining().any_lt(required) {
				return Err(SteppedMigrationError::InsufficientWeight { required });
			}

			let oldest_valid = Pallet::<T>::now().saturating_sub(RetentionPeriod::<T>::get());

			loop {
				if meter.try_consume(required).is_err() {
					break;
				}

				let mut iter = match cursor.as_ref() {
					None => Transactions::<T>::iter_keys(),
					Some(last) =>
						Transactions::<T>::iter_keys_from(Transactions::<T>::hashed_key_for(last)),
				};

				let Some(block_number) = iter.next() else {
					// Never downgrade — this MBM can be re-run against state already
					// at/beyond v3 (e.g. by try-runtime, whose id isn't in `Historic`).
					use polkadot_sdk_frame::prelude::{GetStorageVersion, StorageVersion};
					if Pallet::<T>::on_chain_storage_version() < 3 {
						StorageVersion::new(3).put::<Pallet<T>>();
					}
					cursor = None;
					break;
				};

				let raw_key = Transactions::<T>::hashed_key_for(block_number);

				// Stale leftovers from a previously-longer retention window: drop
				// instead of converting. `on_initialize`'s aging-out only catches
				// up one block at a time, so historical stale entries linger
				// forever otherwise.
				if block_number < oldest_valid {
					sp_io::storage::clear(&raw_key);
					cursor = Some(block_number);
					continue;
				}

				let Some(raw) = sp_io::storage::get(&raw_key) else {
					cursor = Some(block_number);
					continue;
				};

				if BoundedVec::<TransactionInfoFor<T>, T::MaxBlockTransactions>::decode(
					&mut &raw[..],
				)
				.is_ok()
				{
					cursor = Some(block_number);
					continue;
				}

				let v2 =
					BoundedVec::<V2TransactionInfo, T::MaxBlockTransactions>::decode(&mut &raw[..])
						.map_err(|_| SteppedMigrationError::Failed)?;

				let v3: BoundedVec<TransactionInfoFor<T>, T::MaxBlockTransactions> = v2
					.into_iter()
					.map(|old| TransactionInfo {
						chunk_root: old.chunk_root,
						content_hash: old.content_hash,
						hashing: old.hashing,
						cid_codec: old.cid_codec,
						size: old.size,
						extrinsic_index: u32::MAX,
						block_chunks: old.block_chunks,
						meta: Default::default(),
					})
					.collect::<Vec<_>>()
					.try_into()
					.map_err(|_| SteppedMigrationError::Failed)?;

				Transactions::<T>::insert(block_number, v3);
				cursor = Some(block_number);
			}

			Ok(cursor)
		}

		#[cfg(feature = "try-runtime")]
		fn pre_upgrade() -> Result<Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			use polkadot_sdk_frame::deps::frame_support::storage::StoragePrefixedMap;
			let prefix = Transactions::<T>::final_prefix();
			let mut previous_key = prefix.to_vec();
			let mut count: u64 = 0;
			while let Some(key) =
				sp_io::storage::next_key(&previous_key).filter(|k| k.starts_with(&prefix))
			{
				previous_key = key;
				count += 1;
			}
			tracing::info!(target: LOG_TARGET, count, "v2->v3 pre_upgrade: Transactions entries");
			Ok(count.encode())
		}

		#[cfg(feature = "try-runtime")]
		fn post_upgrade(
			state: Vec<u8>,
		) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			use polkadot_sdk_frame::deps::frame_support::storage::StoragePrefixedMap;

			let old_count =
				u64::decode(&mut &state[..]).map_err(|_| "Failed to decode pre_upgrade state")?;

			let prefix = Transactions::<T>::final_prefix();
			let mut previous_key = prefix.to_vec();
			let mut new_count: u64 = 0;
			while let Some(key) =
				sp_io::storage::next_key(&previous_key).filter(|k| k.starts_with(&prefix))
			{
				previous_key = key.clone();
				let raw = sp_io::storage::get(&key)
					.ok_or("v2->v3 post_upgrade: missing Transactions entry")?;
				BoundedVec::<TransactionInfoFor<T>, T::MaxBlockTransactions>::decode(&mut &raw[..])
					.map_err(|_| "v2->v3 post_upgrade: remaining entry is not v3")?;
				new_count += 1;
			}

			polkadot_sdk_frame::prelude::ensure!(
				new_count <= old_count,
				"v2->v3 post_upgrade: entry count increased"
			);
			tracing::info!(
				target: LOG_TARGET,
				old_count,
				new_count,
				pruned = old_count.saturating_sub(new_count),
				"v2->v3 post_upgrade: valid"
			);
			Ok(())
		}
	}
}

/// V3 → V4 migration: storage-version bump only.
///
/// On `main` this reshaped `AutoRenewals` (`{ account }` →
/// `{ account, recurring, paid }`). The renewal split moved `AutoRenewals` out of
/// this pallet into `pallet-bulletin-transaction-storage-renewal`, so that reshape now happens
/// there, during relocation (see
/// [`pallet_bulletin_transaction_storage_renewal::migrations::RelocateFromTransactionStorage`]).
/// This migration is kept as a no-op solely to keep the storage-version chain
/// continuous (3 → 4) so [`v5::MigrateV4ToV5`] — gated on on-chain version `== 4` —
/// still runs on a chain that is only at 3.
pub mod v4 {
	use super::*;
	use crate::pallet::Pallet;
	use polkadot_sdk_frame::deps::frame_support::{
		migrations::VersionedMigration, traits::UncheckedOnRuntimeUpgrade,
	};

	pub struct VersionUncheckedMigrateV3ToV4<T>(PhantomData<T>);

	impl<T: Config> UncheckedOnRuntimeUpgrade for VersionUncheckedMigrateV3ToV4<T> {
		fn on_runtime_upgrade() -> Weight {
			// No data change: `AutoRenewals` left this pallet in the renewal split.
			Weight::zero()
		}
	}

	/// Versioned no-op v3→v4: storage-version bump only (see module docs).
	pub type MigrateV3ToV4<T> = VersionedMigration<
		3,
		4,
		VersionUncheckedMigrateV3ToV4<T>,
		Pallet<T>,
		<T as polkadot_sdk_frame::deps::frame_system::Config>::DbWeight,
	>;
}

/// V4 → V5 migration for `AllowedAuthorizers`.
///
/// `AuthorizerBudget` went from `{ quota, authorization_period, valid_until }` to
/// `{ quota, valid_until, feeless }`. Without translating, an existing
/// `authorization_period: Some(p)` would silently SCALE-decode as `valid_until: p`,
/// corrupting both fields. Existing entries default to `feeless: true` to match
/// the new genesis default.
///
/// Single-block: `AllowedAuthorizers` is an admin allow-list (single-digit count).
pub mod v5 {
	use super::*;
	use crate::{
		pallet::{AllowedAuthorizers, Pallet},
		AuthorizerBudget, Quota,
	};
	use polkadot_sdk_frame::deps::frame_support::{
		migrations::VersionedMigration, traits::UncheckedOnRuntimeUpgrade,
	};

	/// `AuthorizerBudget` layout at v4 (before removing `authorization_period`).
	#[derive(Encode, Decode, Clone, Debug, MaxEncodedLen)]
	pub(crate) struct V4AuthorizerBudget<BlockNumber> {
		pub quota: Option<Quota>,
		pub authorization_period: Option<BlockNumber>,
		pub valid_until: Option<BlockNumber>,
	}

	pub struct VersionUncheckedMigrateV4ToV5<T>(PhantomData<T>);

	impl<T: Config> UncheckedOnRuntimeUpgrade for VersionUncheckedMigrateV4ToV5<T> {
		fn on_runtime_upgrade() -> Weight {
			let mut migrated: u64 = 0;
			AllowedAuthorizers::<T>::translate::<V4AuthorizerBudget<BlockNumberFor<T>>, _>(
				|who, old| {
					migrated = migrated.saturating_add(1);
					// Authorizers registered before v5 never had their System provider
					// reference bumped (the feature was added together with this storage
					// shape). Bring them in line with `add_authorizer` so a `feeless`
					// authorizer with no balance can't be reaped between dispatches.
					Pallet::<T>::inc_authorizer_providers(&who);
					Some(AuthorizerBudget {
						quota: old.quota,
						valid_until: old.valid_until,
						feeless: true,
					})
				},
			);
			tracing::info!(target: LOG_TARGET, migrated, "v4->v5 migration complete");
			// 1 read + 1 write per entry for `AllowedAuthorizers` (via `translate`),
			// plus 1 read + 1 write per entry for `frame_system::Account` (via
			// `inc_providers`).
			T::DbWeight::get().reads_writes(migrated.saturating_mul(2), migrated.saturating_mul(2))
		}

		#[cfg(feature = "try-runtime")]
		fn pre_upgrade() -> Result<Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			use polkadot_sdk_frame::deps::frame_support::storage::StoragePrefixedMap;
			let prefix = AllowedAuthorizers::<T>::final_prefix();
			let mut previous_key = prefix.to_vec();
			let mut count: u64 = 0;
			while let Some(key) = polkadot_sdk_frame::deps::sp_io::storage::next_key(&previous_key)
				.filter(|k| k.starts_with(&prefix))
			{
				previous_key = key;
				count += 1;
			}
			tracing::info!(
				target: LOG_TARGET,
				count,
				"v4->v5 pre_upgrade: AllowedAuthorizers entries",
			);
			Ok(count.encode())
		}

		#[cfg(feature = "try-runtime")]
		fn post_upgrade(
			state: Vec<u8>,
		) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			let old_count =
				u64::decode(&mut &state[..]).map_err(|_| "Failed to decode pre_upgrade state")?;
			let new_count = AllowedAuthorizers::<T>::iter().count() as u64;
			polkadot_sdk_frame::prelude::ensure!(
				new_count == old_count,
				"v4->v5 post_upgrade: entry count changed",
			);
			tracing::info!(
				target: LOG_TARGET,
				old_count,
				new_count,
				"v4->v5 post_upgrade: valid",
			);
			Ok(())
		}
	}

	/// Versioned migration v4→v5: drops `authorization_period` from `AuthorizerBudget`.
	pub type MigrateV4ToV5<T> = VersionedMigration<
		4,
		5,
		VersionUncheckedMigrateV4ToV5<T>,
		Pallet<T>,
		<T as polkadot_sdk_frame::deps::frame_system::Config>::DbWeight,
	>;
}
