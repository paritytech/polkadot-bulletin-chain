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

/// Identifier prefix for every stepped migration in this pallet. The
/// MBM scheduler keys on `(pallet_id, version_from, version_to)`, so all of
/// our `SteppedMigration` impls must share this constant — only the version
/// pair distinguishes one from the next.
const MIGRATIONS_ID: &[u8; 24] = b"bulletin-tx-storage-vmig";

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

/// Migration v0→v1: Adds `hashing` and `cid_codec` fields to `TransactionInfo`.
///
/// Handles mixed-format storage safely: the chain was upgraded without migration,
/// so `Transactions` contains both old-format (pre-CID) and new-format (post-CID)
/// entries. Uses raw storage iteration with try-new-then-old decoding to avoid
/// corrupting post-upgrade entries.
///
/// Old entries get defaults: `hashing = Blake2b256`, `cid_codec = RAW_CODEC`.
pub mod v1 {
	use super::*;
	use crate::{
		pallet::{Pallet, Transactions},
		TransactionInfo,
	};
	use bulletin_transaction_storage_primitives::{
		cids::{CidCodec, HashingAlgorithm, RAW_CODEC},
		ContentHash,
	};
	use polkadot_sdk_frame::deps::{
		frame_support::{
			migrations::VersionedMigration,
			storage::{unhashed, StoragePrefixedMap},
			traits::UncheckedOnRuntimeUpgrade,
			BoundedVec,
		},
		sp_io,
		sp_runtime::traits::{BlakeTwo256, Hash},
	};
	use sp_transaction_storage_proof::ChunkIndex;

	/// `TransactionInfo` layout before v1 (no CID fields).
	#[derive(Encode, Decode, Clone, Debug, MaxEncodedLen)]
	pub(crate) struct OldTransactionInfo {
		pub chunk_root: <BlakeTwo256 as Hash>::Output,
		pub content_hash: <BlakeTwo256 as Hash>::Output,
		pub size: u32,
		pub block_chunks: ChunkIndex,
	}

	/// `TransactionInfo` layout at v1 (mandatory `hashing` and `cid_codec`).
	#[derive(Encode, Decode, Clone, Debug, MaxEncodedLen)]
	pub(crate) struct V1TransactionInfo {
		pub chunk_root: <BlakeTwo256 as Hash>::Output,
		pub content_hash: ContentHash,
		pub hashing: HashingAlgorithm,
		pub cid_codec: CidCodec,
		pub size: u32,
		pub block_chunks: ChunkIndex,
	}

	/// Version-unchecked migration logic. Wrapped by [`MigrateV0ToV1`] for version gating.
	pub struct VersionUncheckedMigrateV0ToV1<T>(PhantomData<T>);

	impl<T: Config> UncheckedOnRuntimeUpgrade for VersionUncheckedMigrateV0ToV1<T> {
		/// NOTE: This iterates all `Transactions` entries without an upper bound.
		/// The entry count is bounded by `RetentionPeriod` (one per block number).
		/// At the time of deployment the live chain has 126 entries, well within
		/// a single block's weight and PoV limits. If the entry count were ever
		/// close to `RetentionPeriod` (100,800), this would need to be converted
		/// to a multi-block migration.
		fn on_runtime_upgrade() -> Weight {
			let prefix = Transactions::<T>::final_prefix();
			let mut previous_key = prefix.to_vec();
			let mut migrated: u64 = 0;
			let mut skipped: u64 = 0;
			let mut corrupted: u64 = 0;

			while let Some(key) =
				sp_io::storage::next_key(&previous_key).filter(|k| k.starts_with(&prefix))
			{
				previous_key = key.clone();
				let Some(raw) = unhashed::get_raw(&key) else { continue };

				// Try decode as current type first — if it works, the entry is
				// already post-upgrade. Old format (72 bytes/entry) always fails
				// here because the decoder runs out of bytes.
				if BoundedVec::<TransactionInfo, T::MaxBlockTransactions>::decode(&mut &raw[..])
					.is_ok()
				{
					skipped += 1;
					continue;
				}

				// Fall back to old type and transform.
				match BoundedVec::<OldTransactionInfo, T::MaxBlockTransactions>::decode(
					&mut &raw[..],
				) {
					Ok(old_txs) => {
						let new_txs: Vec<V1TransactionInfo> = old_txs
							.into_iter()
							.map(|old| V1TransactionInfo {
								chunk_root: old.chunk_root,
								content_hash: old.content_hash.into(),
								hashing: HashingAlgorithm::Blake2b256,
								cid_codec: RAW_CODEC,
								size: old.size,
								block_chunks: old.block_chunks,
							})
							.collect();
						let Ok(bounded) =
							BoundedVec::<V1TransactionInfo, T::MaxBlockTransactions>::try_from(
								new_txs,
							)
						else {
							// Unreachable: decoded N items from a BoundedVec with the same
							// bound, mapped 1:1. Log defensively and skip.
							polkadot_sdk_frame::deps::frame_support::defensive!(
								"v0->v1: BoundedVec conversion failed"
							);
							continue;
						};
						unhashed::put_raw(&key, &bounded.encode());
						migrated += 1;
					},
					Err(_) => {
						// Corrupted entry — remove to prevent on_finalize panic.
						unhashed::kill(&key);
						corrupted += 1;
						tracing::warn!(
							target: LOG_TARGET,
							"Removed corrupted Transactions entry during v0->v1 migration",
						);
					},
				}
			}

			tracing::info!(
				target: LOG_TARGET,
				migrated,
				skipped,
				corrupted,
				"v0->v1 TransactionInfo migration complete",
			);

			let entries = migrated + skipped + corrupted;
			// 2 reads per entry (next_key + get_raw), 1 for the final next_key returning None.
			// 1 write per migrated (put_raw) or corrupted (kill) entry; skipped = 0 writes.
			T::DbWeight::get()
				.reads(entries.saturating_mul(2).saturating_add(1))
				.saturating_add(T::DbWeight::get().writes(migrated + corrupted))
		}

		#[cfg(feature = "try-runtime")]
		fn pre_upgrade() -> Result<Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			let prefix = Transactions::<T>::final_prefix();
			let mut previous_key = prefix.to_vec();
			let mut count: u64 = 0;
			while let Some(key) =
				sp_io::storage::next_key(&previous_key).filter(|k| k.starts_with(&prefix))
			{
				previous_key = key;
				count += 1;
			}
			tracing::info!(target: LOG_TARGET, count, "pre_upgrade: Transactions entries");
			Ok(count.encode())
		}

		#[cfg(feature = "try-runtime")]
		fn post_upgrade(
			state: Vec<u8>,
		) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			let old_count =
				u64::decode(&mut &state[..]).map_err(|_| "Failed to decode pre_upgrade state")?;
			// iter() decodes every entry — if any fail, they are skipped and the
			// count drops, which the check below will catch.
			let new_count = Transactions::<T>::iter().count() as u64;
			polkadot_sdk_frame::prelude::ensure!(
				new_count <= old_count,
				"post_upgrade: more entries than before migration"
			);
			tracing::info!(target: LOG_TARGET, old_count, new_count, "post_upgrade: valid");
			Ok(())
		}
	}

	/// Versioned migration v0→v1: adds `hashing` and `cid_codec` to `TransactionInfo`.
	/// Safe for mixed old/new format storage.
	pub type MigrateV0ToV1<T> = VersionedMigration<
		0,
		1,
		VersionUncheckedMigrateV0ToV1<T>,
		Pallet<T>,
		<T as polkadot_sdk_frame::deps::frame_system::Config>::DbWeight,
	>;

	/// Run the v0→v1 `TransactionInfo` migration if the on-chain storage version
	/// is still 0. This covers the `codeSubstitutes` recovery path where the fix
	/// runtime is loaded without triggering `on_runtime_upgrade`.
	///
	/// Returns the weight consumed. On subsequent blocks (version already ≥ 1)
	/// this is a single storage read.
	pub fn maybe_migrate_v0_to_v1<T: Config>() -> Weight {
		use polkadot_sdk_frame::prelude::{GetStorageVersion, StorageVersion};

		let on_chain = Pallet::<T>::on_chain_storage_version();
		if on_chain >= 1 {
			return T::DbWeight::get().reads(1);
		}

		tracing::info!(
			target: LOG_TARGET,
			?on_chain,
			"Running v0→v1 TransactionInfo migration from on_initialize",
		);

		let migration_weight = VersionUncheckedMigrateV0ToV1::<T>::on_runtime_upgrade();

		StorageVersion::new(1).put::<Pallet<T>>();

		// 1 read (version check) + migration weight + 1 write (version bump)
		T::DbWeight::get()
			.reads(1)
			.saturating_add(migration_weight)
			.saturating_add(T::DbWeight::get().writes(1))
	}
}

/// Migration v1→v2: rewrites `AuthorizationExtent` from `{ transactions, bytes }` to
/// `{ transactions, transactions_allowance, bytes, bytes_permanent, bytes_allowance }`.
/// The old remaining quota becomes the new allowance; consumed counters reset to `0`.
/// Entries with `bytes == 0` are dropped (already unusable; `bytes_allowance > 0` is a
/// v2 invariant).
///
/// `Transactions` is intentionally left at the v1 shape — the stepped `v2→v3` migration
/// decodes it via `V2TransactionInfo` (byte-identical) and converts entries to the
/// current layout in bounded per-block steps. `BlockTransactions` is still translated
/// here defensively (single transient `StorageValue`).
pub mod v2 {
	use super::*;
	use crate::{
		pallet::{BlockTransactions, Pallet},
		TransactionInfo, TransactionKind,
	};
	use bulletin_transaction_storage_primitives::{
		cids::{CidCodec, HashingAlgorithm},
		ContentHash,
	};
	use polkadot_sdk_frame::deps::{
		frame_support::{
			migrations::VersionedMigration, traits::UncheckedOnRuntimeUpgrade, BoundedVec,
		},
		sp_runtime::traits::{BlakeTwo256, Hash},
	};
	use sp_transaction_storage_proof::ChunkIndex;

	/// `TransactionInfo` layout at v1 — same shape as v2 minus the trailing `kind`
	/// field. Used here to decode existing entries during translation.
	#[derive(Encode, Decode, Clone, Debug, MaxEncodedLen)]
	pub(crate) struct V1TransactionInfo {
		pub chunk_root: <BlakeTwo256 as Hash>::Output,
		pub content_hash: ContentHash,
		pub hashing: HashingAlgorithm,
		pub cid_codec: CidCodec,
		pub size: u32,
		pub block_chunks: ChunkIndex,
	}

	pub struct VersionUncheckedMigrateV1ToV2<T>(PhantomData<T>);

	impl<T: Config> UncheckedOnRuntimeUpgrade for VersionUncheckedMigrateV1ToV2<T> {
		fn on_runtime_upgrade() -> Weight {
			// The historical body of this migration translated entries in the old
			// single-window `Authorizations` map. The slot redesign (v3→v4) drops
			// that storage entirely, so translating it here would be wasted work
			// — we leave the entries in the old layout for v3→v4 to clear by
			// raw prefix.
			//
			// `BlockTransactions` is transient (cleared every `on_finalize`),
			// almost always empty between blocks, but we still translate
			// defensively in case the upgrade lands mid-block.
			let mut block_tx_present = 0u64;
			let _ = BlockTransactions::<T>::translate::<
				BoundedVec<V1TransactionInfo, T::MaxBlockTransactions>,
				_,
			>(|maybe_old| {
				let old_vec = maybe_old?;
				block_tx_present = 1;
				let new_vec: alloc::vec::Vec<TransactionInfo> = old_vec
					.into_iter()
					.map(|old| TransactionInfo {
						chunk_root: old.chunk_root,
						content_hash: old.content_hash,
						hashing: old.hashing,
						cid_codec: old.cid_codec,
						size: old.size,
						extrinsic_index: u32::MAX,
						block_chunks: old.block_chunks,
						kind: TransactionKind::Store,
					})
					.collect();
				Some(
					BoundedVec::<TransactionInfo, T::MaxBlockTransactions>::try_from(new_vec)
						.expect("v1->v2: vec re-bounded with same size; qed"),
				)
			});

			tracing::info!(
				target: LOG_TARGET,
				block_transactions_present = block_tx_present,
				"v1->v2 BlockTransactions migration complete",
			);

			T::DbWeight::get().reads_writes(block_tx_present, block_tx_present)
		}
	}

	/// Versioned migration v1→v2: translates `BlockTransactions` to the v2
	/// `TransactionInfo` layout. The (legacy) per-window `Authorizations` map is
	/// cleared by v3→v4.
	pub type MigrateV1ToV2<T> = VersionedMigration<
		1,
		2,
		VersionUncheckedMigrateV1ToV2<T>,
		Pallet<T>,
		<T as polkadot_sdk_frame::deps::frame_system::Config>::DbWeight,
	>;
}

/// Migration v3→v4: translate the legacy single-window `Authorizations` map
/// into the slot-based `AuthorizationSlots` map.
///
/// Stepped (multi-block) migration. Each step drains a few legacy entries; an
/// active entry becomes a single fresh slot inheriting the original allowances
/// (used counters reset to `0`); an already-expired entry is dropped.
/// Storage version flips to `4` once the legacy map is exhausted.
///
/// The legacy `Authorization::expiration` was a parachain block number; the
/// slot model uses relay block numbers. The two clocks aren't aligned, so we
/// don't try to translate the original deadline — every still-valid entry gets
/// a fresh `T::DefaultAuthorizationWindow`-block window starting at
/// `relay_now`. Pre-existing renewed bytes from the old window remain in
/// `PermanentStorageUsed` and age out via `on_initialize`, so the per-account
/// `bytes_permanent` reset does not double-count chain-wide footprint.
pub mod v4 {
	use super::*;
	use crate::{
		pallet::{AuthorizationSlots, Pallet},
		AuthorizationExtent, AuthorizationScopeFor, TimedAuthorization, WeightInfo,
	};
	use polkadot_sdk_frame::{
		deps::{
			frame_support::{
				migrations::{MigrationId, SteppedMigration, SteppedMigrationError},
				storage_alias,
				weights::WeightMeter,
				Blake2_128Concat, BoundedVec,
			},
			sp_runtime::traits::BlockNumberProvider,
		},
		prelude::{frame_system, StorageVersion},
	};

	/// Legacy `Authorization` shape (v3): single-window per scope. Production
	/// code only decodes existing entries; the benchmark and tests write it
	/// via the `Authorizations` storage alias to seed scenarios.
	#[derive(Encode, Decode, MaxEncodedLen, scale_info::TypeInfo)]
	pub(crate) struct LegacyAuthorization<BlockNumber> {
		pub extent: AuthorizationExtent,
		pub expiration: BlockNumber,
	}

	/// Storage alias matching the v3 `Authorizations` storage prefix. Reads
	/// the live legacy entries during the migration; cleared as we go.
	#[storage_alias]
	pub(crate) type Authorizations<T: Config> = StorageMap<
		Pallet<T>,
		Blake2_128Concat,
		AuthorizationScopeFor<T>,
		LegacyAuthorization<polkadot_sdk_frame::prelude::BlockNumberFor<T>>,
		polkadot_sdk_frame::deps::frame_support::pallet_prelude::OptionQuery,
	>;

	/// Stepped migration from storage version 3 to 4.
	pub struct MigrateV3ToV4<T: Config>(PhantomData<T>);

	impl<T: Config> SteppedMigration for MigrateV3ToV4<T> {
		type Cursor = AuthorizationScopeFor<T>;
		type Identifier = MigrationId<24>;

		fn id() -> Self::Identifier {
			MigrationId { pallet_id: *super::MIGRATIONS_ID, version_from: 3, version_to: 4 }
		}

		fn step(
			mut cursor: Option<Self::Cursor>,
			meter: &mut WeightMeter,
		) -> Result<Option<Self::Cursor>, SteppedMigrationError> {
			let relay_now = T::RelayChainBlockNumberProvider::current_block_number();
			if relay_now == 0 {
				// The genesis sentinel — only seen on misconfigured try-runtime
				// snapshots. Live parachains fall back to
				// `last_relay_block_number()` via `RelaychainDataProvider`. Fail
				// loudly rather than write slot windows from `0`.
				tracing::error!(
					target: LOG_TARGET,
					"v3->v4: relay block number unavailable; cannot run migration",
				);
				return Err(SteppedMigrationError::Failed);
			}

			let required = T::WeightInfo::migrate_v3_to_v4_step();
			if meter.remaining().any_lt(required) {
				return Err(SteppedMigrationError::InsufficientWeight { required });
			}

			let parachain_now = frame_system::Pallet::<T>::block_number();
			let default_window = T::DefaultAuthorizationWindow::get();
			let new_expiration = relay_now.saturating_add(default_window);

			loop {
				if meter.try_consume(required).is_err() {
					break;
				}

				let mut iter = match cursor.as_ref() {
					None => Authorizations::<T>::iter(),
					Some(last) =>
						Authorizations::<T>::iter_from(Authorizations::<T>::hashed_key_for(last)),
				};

				let Some((scope, old)) = iter.next() else {
					// Pin to v4 explicitly; `in_code_storage_version` may be
					// higher if later versions chain on top.
					StorageVersion::new(4).put::<Pallet<T>>();
					cursor = None;
					break;
				};

				// Move the cursor immediately so we don't get stuck if the
				// translate branch panics in debug.
				cursor = Some(scope.clone());

				if old.expiration <= parachain_now || old.extent.bytes_allowance == 0 {
					Authorizations::<T>::remove(&scope);
					continue;
				}

				let slot = TimedAuthorization {
					extent: AuthorizationExtent {
						bytes: 0,
						bytes_permanent: 0,
						transactions: 0,
						bytes_allowance: old.extent.bytes_allowance,
						transactions_allowance: old.extent.transactions_allowance,
					},
					starts_at: relay_now,
					expiration: new_expiration,
				};
				let bounded = BoundedVec::<TimedAuthorization, T::MaxAuthorizationSlots>::try_from(
					alloc::vec![slot],
				)
				.expect("MaxAuthorizationSlots > 0 by integrity_test; one slot fits; qed");

				AuthorizationSlots::<T>::insert(&scope, bounded);
				Authorizations::<T>::remove(&scope);
				Pallet::<T>::authorization_added(&scope);
			}

			Ok(cursor)
		}

		#[cfg(feature = "try-runtime")]
		fn pre_upgrade() -> Result<Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			let count = Authorizations::<T>::iter().count() as u64;
			tracing::info!(target: LOG_TARGET, count, "v3->v4 pre_upgrade: legacy entries");
			Ok(count.encode())
		}

		#[cfg(feature = "try-runtime")]
		fn post_upgrade(
			state: Vec<u8>,
		) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			use polkadot_sdk_frame::prelude::GetStorageVersion;

			let old_count =
				u64::decode(&mut &state[..]).map_err(|_| "Failed to decode pre_upgrade state")?;

			polkadot_sdk_frame::prelude::ensure!(
				Authorizations::<T>::iter().next().is_none(),
				"v3->v4 post_upgrade: legacy Authorizations not empty",
			);

			let mut new_count: u64 = 0;
			for (_, slots) in AuthorizationSlots::<T>::iter() {
				new_count = new_count.saturating_add(1);
				for slot in slots.iter() {
					polkadot_sdk_frame::prelude::ensure!(
						slot.extent.bytes == 0 &&
							slot.extent.bytes_permanent == 0 &&
							slot.extent.transactions == 0,
						"v3->v4 post_upgrade: translated slot has non-zero used counters",
					);
				}
			}
			polkadot_sdk_frame::prelude::ensure!(
				new_count <= old_count,
				"v3->v4 post_upgrade: more new slots than legacy entries",
			);
			polkadot_sdk_frame::prelude::ensure!(
				Pallet::<T>::on_chain_storage_version() >= StorageVersion::new(4),
				"v3->v4 post_upgrade: storage version not bumped",
			);
			tracing::info!(
				target: LOG_TARGET,
				old_count,
				new_count,
				dropped = old_count.saturating_sub(new_count),
				"v3->v4 post_upgrade: ok",
			);
			Ok(())
		}
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
		RetentionPeriod, TransactionInfo, TransactionKind, WeightInfo,
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
			MigrationId { pallet_id: *super::MIGRATIONS_ID, version_from: 2, version_to: 3 }
		}

		fn step(
			mut cursor: Option<Self::Cursor>,
			meter: &mut WeightMeter,
		) -> Result<Option<Self::Cursor>, SteppedMigrationError> {
			use polkadot_sdk_frame::prelude::{frame_system, Saturating};

			let required = T::WeightInfo::migrate_v2_to_v3_step();
			if meter.remaining().any_lt(required) {
				return Err(SteppedMigrationError::InsufficientWeight { required });
			}

			let oldest_valid = frame_system::Pallet::<T>::block_number()
				.saturating_sub(RetentionPeriod::<T>::get());

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
					// Pin to v3 explicitly. We can't use `in_code_storage_version`
					// here: this MBM migrates to v3, but the in-code version may
					// be higher (later versions chain on top). The follow-up
					// migrations bump from there.
					polkadot_sdk_frame::prelude::StorageVersion::new(3).put::<Pallet<T>>();
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

				if BoundedVec::<TransactionInfo, T::MaxBlockTransactions>::decode(&mut &raw[..])
					.is_ok()
				{
					cursor = Some(block_number);
					continue;
				}

				let v2 =
					BoundedVec::<V2TransactionInfo, T::MaxBlockTransactions>::decode(&mut &raw[..])
						.map_err(|_| SteppedMigrationError::Failed)?;

				let v3: BoundedVec<TransactionInfo, T::MaxBlockTransactions> = v2
					.into_iter()
					.map(|old| TransactionInfo {
						chunk_root: old.chunk_root,
						content_hash: old.content_hash,
						hashing: old.hashing,
						cid_codec: old.cid_codec,
						size: old.size,
						extrinsic_index: u32::MAX,
						block_chunks: old.block_chunks,
						kind: TransactionKind::Store,
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
				BoundedVec::<TransactionInfo, T::MaxBlockTransactions>::decode(&mut &raw[..])
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
