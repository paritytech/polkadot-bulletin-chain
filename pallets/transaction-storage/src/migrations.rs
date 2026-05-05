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

/// Migration v1→v2: rewrites the `AuthorizationExtent` schema and tail-extends
/// `TransactionInfo` with a `kind` discriminator.
///
/// Old `AuthorizationExtent`: `{ transactions: u32, bytes: u64 }` — transaction count
/// and remaining byte quota.
///
/// New `AuthorizationExtent`: `{ transactions, transactions_allowance, bytes,
/// bytes_permanent, bytes_allowance }` — bytes consumed so far (split into store-side
/// and renew-side), total bytes granted, plus a parallel boost-tier transaction
/// counter and budget.
///
/// Each authorization's remaining byte quota becomes the new total allowance
/// (`bytes_allowance = old.bytes`, `bytes = bytes_permanent = 0`); the remaining
/// transaction count becomes `transactions_allowance` (`transactions = 0`), so each
/// authorization keeps its previous remaining capacity on both axes. Entries whose
/// remaining byte quota is already zero are dropped — they can't be translated to a
/// valid v2 entry (`check_authorizations_integrity` requires `bytes_allowance > 0`)
/// and they were already unusable on the old chain.
///
/// `TransactionInfo` gains a new `kind: TransactionKind { Store, Renew }` field
/// appended at the end so existing entries can be tail-extended. Pre-migration entries
/// are translated with `kind = Store`: they age out without affecting the chain-wide
/// `PermanentStorageUsed` counter, which starts at 0 and converges to accurate within
/// one `RetentionPeriod`.
pub mod v2 {
	use super::*;
	use crate::{
		pallet::{Authorizations, BlockTransactions, Pallet, Transactions},
		Authorization, AuthorizationExtent, TransactionInfo, TransactionKind,
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

	#[derive(Encode, Decode)]
	pub(crate) struct V1AuthorizationExtent {
		pub transactions: u32,
		pub bytes: u64,
	}

	#[derive(Encode, Decode)]
	pub(crate) struct V1Authorization<BlockNumber> {
		pub extent: V1AuthorizationExtent,
		pub expiration: BlockNumber,
	}

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
			let mut auth_migrated: u64 = 0;
			let mut auth_dropped: u64 = 0;
			Authorizations::<T>::translate::<V1Authorization<BlockNumberFor<T>>, _>(
				|_scope, old| {
					if old.extent.bytes == 0 {
						auth_dropped = auth_dropped.saturating_add(1);
						return None;
					}
					auth_migrated = auth_migrated.saturating_add(1);
					Some(Authorization {
						extent: AuthorizationExtent {
							bytes: 0,
							bytes_permanent: 0,
							bytes_allowance: old.extent.bytes,
							transactions: 0,
							transactions_allowance: old.extent.transactions,
						},
						expiration: old.expiration,
					})
				},
			);
			tracing::info!(
				target: LOG_TARGET,
				migrated = auth_migrated,
				dropped = auth_dropped,
				"v1->v2 AuthorizationExtent migration complete",
			);

			// Tail-extend every TransactionInfo with `kind = Store`. Pre-migration
			// renewed bytes age out without bumping/decrementing the chain counter,
			// which starts at 0 and converges within one RetentionPeriod.
			let mut tx_blocks_migrated: u64 = 0;
			Transactions::<T>::translate::<BoundedVec<V1TransactionInfo, T::MaxBlockTransactions>, _>(
				|_block, old_vec| {
					tx_blocks_migrated = tx_blocks_migrated.saturating_add(1);
					let new_vec: alloc::vec::Vec<TransactionInfo> = old_vec
						.into_iter()
						.map(|old| TransactionInfo {
							chunk_root: old.chunk_root,
							content_hash: old.content_hash,
							hashing: old.hashing,
							cid_codec: old.cid_codec,
							size: old.size,
							block_chunks: old.block_chunks,
							kind: TransactionKind::Store,
						})
						.collect();
					let bounded =
						BoundedVec::<TransactionInfo, T::MaxBlockTransactions>::try_from(new_vec)
							.expect("v1->v2: vec re-bounded with same size; qed");
					Some(bounded)
				},
			);

			// `BlockTransactions` is transient (cleared in `on_finalize`) so it's
			// almost always empty between blocks, but translate defensively in case
			// the upgrade lands mid-block.
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
				blocks = tx_blocks_migrated,
				block_transactions_present = block_tx_present,
				"v1->v2 TransactionInfo migration complete",
			);

			let auth_touched = auth_migrated.saturating_add(auth_dropped);
			let tx_touched = tx_blocks_migrated.saturating_add(block_tx_present);
			T::DbWeight::get().reads_writes(
				auth_touched.saturating_add(tx_touched),
				auth_touched.saturating_add(tx_touched),
			)
		}
	}

	/// Versioned migration v1→v2: rewrites `AuthorizationExtent` and `TransactionInfo`.
	pub type MigrateV1ToV2<T> = VersionedMigration<
		1,
		2,
		VersionUncheckedMigrateV1ToV2<T>,
		Pallet<T>,
		<T as polkadot_sdk_frame::deps::frame_system::Config>::DbWeight,
	>;
}
