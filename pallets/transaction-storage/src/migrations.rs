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
/// Old entries get defaults: `hashing = Blake2b256`, `cid_codec = 0x55` (raw).
pub mod v1 {
	use super::*;
	use crate::{
		cids::HashingAlgorithm,
		pallet::{Pallet, Transactions},
		TransactionInfo,
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

	/// Unchecked migration logic. Wrapped by [`MigrateV0ToV1`] for version gating.
	pub struct UncheckedMigrateV0ToV1<T>(PhantomData<T>);

	impl<T: Config> UncheckedOnRuntimeUpgrade for UncheckedMigrateV0ToV1<T> {
		fn on_runtime_upgrade() -> Weight {
			let prefix = Transactions::<T>::final_prefix();
			let mut previous_key = prefix.to_vec();
			let mut migrated: u64 = 0;
			let mut skipped: u64 = 0;

			while let Some(key) =
				sp_io::storage::next_key(&previous_key).filter(|k| k.starts_with(&prefix))
			{
				previous_key = key.clone();
				let Some(raw) = unhashed::get_raw(&key) else { continue };

				// Try decode as new type first — if it works, the entry is already
				// post-upgrade. Old format (72 bytes/entry) always fails here because
				// the decoder runs out of bytes (new format needs 81 bytes/entry).
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
						let new_txs: Vec<TransactionInfo> = old_txs
							.into_iter()
							.map(|old| TransactionInfo {
								chunk_root: old.chunk_root,
								content_hash: old.content_hash.into(),
								hashing: HashingAlgorithm::Blake2b256,
								cid_codec: 0x55, // raw codec — the only codec pre-CID
								size: old.size,
								block_chunks: old.block_chunks,
							})
							.collect();
						let bounded: BoundedVec<TransactionInfo, T::MaxBlockTransactions> =
							new_txs.try_into().expect("same length, same bound");
						unhashed::put_raw(&key, &bounded.encode());
						migrated += 1;
					},
					Err(_) => {
						// Corrupted entry — remove to prevent on_finalize panic.
						unhashed::kill(&key);
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
				"v0->v1 TransactionInfo migration complete",
			);

			T::DbWeight::get()
				.reads(migrated + skipped + 1)
				.saturating_add(T::DbWeight::get().writes(migrated))
		}

		#[cfg(feature = "try-runtime")]
		fn pre_upgrade()
		-> Result<Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
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
			let new_count = Transactions::<T>::iter().count() as u64;
			polkadot_sdk_frame::prelude::ensure!(
				new_count <= old_count,
				"post_upgrade: more entries than before migration"
			);
			for (_block, txs) in Transactions::<T>::iter() {
				for tx in txs.iter() {
					polkadot_sdk_frame::prelude::ensure!(
						matches!(
							tx.hashing,
							HashingAlgorithm::Blake2b256
								| HashingAlgorithm::Sha2_256
								| HashingAlgorithm::Keccak256
						),
						"Invalid hashing algorithm"
					);
				}
			}
			tracing::info!(target: LOG_TARGET, old_count, new_count, "post_upgrade: valid");
			Ok(())
		}
	}

	/// Versioned migration v0→v1: adds `hashing` and `cid_codec` to `TransactionInfo`.
	/// Safe for mixed old/new format storage.
	pub type MigrateV0ToV1<T> = VersionedMigration<
		0,
		1,
		UncheckedMigrateV0ToV1<T>,
		Pallet<T>,
		<T as polkadot_sdk_frame::deps::frame_system::Config>::DbWeight,
	>;
}
