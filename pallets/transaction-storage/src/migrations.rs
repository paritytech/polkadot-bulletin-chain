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

use crate::{AllowedAuthorizers, AuthorizerBudgetFor, Config, RetentionPeriod, LOG_TARGET};
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

/// Runtime migration that seeds `AllowedAuthorizers` with the given accounts
/// **only if the storage is currently empty**.
///
/// Idempotent: safe to run multiple times. Skips if any authorizers already
/// exist (e.g., set via genesis on a fresh chain, or by a previous run).
pub struct PopulateAllowedAuthorizersIfEmpty<T, Accounts, Budget>(
	PhantomData<(T, Accounts, Budget)>,
);
impl<T: Config, Accounts: Get<Vec<T::AccountId>>, Budget: Get<AuthorizerBudgetFor<T>>>
	OnRuntimeUpgrade for PopulateAllowedAuthorizersIfEmpty<T, Accounts, Budget>
{
	fn on_runtime_upgrade() -> Weight {
		let weight = T::DbWeight::get().reads(1);

		if AllowedAuthorizers::<T>::iter_keys().next().is_some() {
			tracing::info!(
				target: LOG_TARGET,
				"[PopulateAllowedAuthorizersIfEmpty] AllowedAuthorizers non-empty, skipping",
			);
			return weight;
		}

		let accounts = Accounts::get();
		let count = accounts.len() as u64;
		for who in accounts {
			AllowedAuthorizers::<T>::insert(&who, Budget::get());
		}

		tracing::warn!(
			target: LOG_TARGET,
			count,
			"[PopulateAllowedAuthorizersIfEmpty] seeded AllowedAuthorizers",
		);

		weight.saturating_add(T::DbWeight::get().writes(count))
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(
		_state: alloc::vec::Vec<u8>,
	) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::DispatchError> {
		for who in Accounts::get() {
			polkadot_sdk_frame::prelude::ensure!(
				AllowedAuthorizers::<T>::contains_key(&who),
				"expected authorizer missing from AllowedAuthorizers after migration",
			);
		}
		tracing::info!(target: LOG_TARGET, "PopulateAllowedAuthorizersIfEmpty is OK!");
		Ok(())
	}
}

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
/// into the slot-based [`crate::pallet::Authorizations`] map **in place** — v3
/// and v4 share the `"Authorizations"` storage prefix, so [`MigrateV3ToV4`]
/// rewrites each key from a `LegacyAuthorization` into an [`Authorization<T>`].
///
/// Stepped: each step processes one entry. Still-active legacy entries become
/// a single fresh slot inheriting the allowances (used counters reset);
/// expired or zero-allowance entries are removed. Storage version flips to
/// `4` once the legacy iterator is exhausted.
///
/// The legacy `expiration` was a parachain block; the slot model uses relay
/// blocks. The clocks aren't aligned, so every still-valid entry gets a fresh
/// `T::DefaultAuthorizationWindow`-block window starting at `relay_now`.
/// Pre-existing renewed bytes from the old window remain in
/// `PermanentStorageUsed` and age out via `on_initialize`, so the per-account
/// `bytes_permanent` reset does not double-count chain-wide footprint.
pub mod v4 {
	use super::*;
	use crate::{
		pallet::{Authorizations, Pallet},
		Authorization, AuthorizationExtent, AuthorizationScopeFor, TimedAuthorization, WeightInfo,
	};
	use polkadot_sdk_frame::{
		deps::{
			frame_support::{
				migrations::{MigrationId, SteppedMigration, SteppedMigrationError},
				storage::types::StorageMap,
				traits::{ConstU32, StorageInstance},
				weights::WeightMeter,
				Blake2_128Concat, BoundedVec,
			},
			sp_runtime::traits::BlockNumberProvider,
		},
		prelude::{frame_system, BlockNumberFor, StorageVersion},
	};

	/// Legacy `Authorization` shape (v3): single-window per scope. Production
	/// code only decodes existing entries; the benchmark and tests write it
	/// via the [`LegacyAuthorizations`] storage alias to seed scenarios.
	#[derive(Encode, Decode, MaxEncodedLen, scale_info::TypeInfo)]
	pub struct LegacyAuthorization<BlockNumber> {
		pub extent: AuthorizationExtent,
		pub expiration: BlockNumber,
	}

	/// Hand-rolled prefix struct so the legacy alias decouples its Rust type
	/// name (`LegacyAuthorizations`) from its on-chain prefix (`"Authorizations"`,
	/// shared with the live `pallet::Authorizations` v4 storage). This is what
	/// makes the v3→v4 rewrite in-place: the legacy alias and the new pallet
	/// storage are two decoders for the same on-chain keys.
	pub struct LegacyAuthorizationsInstance<T>(PhantomData<T>);
	impl<T: Config> StorageInstance for LegacyAuthorizationsInstance<T> {
		fn pallet_prefix() -> &'static str {
			<Pallet<T> as polkadot_sdk_frame::deps::frame_support::traits::PalletInfoAccess>::name()
		}
		const STORAGE_PREFIX: &'static str = "Authorizations";
	}

	/// View over the v3 `"Authorizations"` prefix that decodes values as
	/// `LegacyAuthorization`. Used by [`MigrateV3ToV4`] to read entries before
	/// overwriting them with v4 [`Authorization<T>`] values at the same key.
	pub type LegacyAuthorizations<T> = StorageMap<
		LegacyAuthorizationsInstance<T>,
		Blake2_128Concat,
		AuthorizationScopeFor<T>,
		LegacyAuthorization<BlockNumberFor<T>>,
		polkadot_sdk_frame::deps::frame_support::pallet_prelude::OptionQuery,
	>;

	/// Stepped migration from storage version 3 to 4.
	pub struct MigrateV3ToV4<T: Config>(PhantomData<T>);

	/// Upper bound on the migration cursor's hashed-key length.
	/// `Blake2_128Concat` produces `16 + encoded_key` bytes; the encoded
	/// `AuthorizationScope` (Account 32B / Preimage 32B plus 1B discriminator)
	/// fits well inside this.
	const CURSOR_MAX_LEN: u32 = 128;

	impl<T: Config> SteppedMigration for MigrateV3ToV4<T> {
		type Cursor = BoundedVec<u8, ConstU32<CURSOR_MAX_LEN>>;
		type Identifier = MigrationId<24>;

		fn id() -> Self::Identifier {
			MigrationId { pallet_id: *super::MIGRATIONS_ID, version_from: 3, version_to: 4 }
		}

		fn step(
			cursor: Option<Self::Cursor>,
			meter: &mut WeightMeter,
		) -> Result<Option<Self::Cursor>, SteppedMigrationError> {
			use polkadot_sdk_frame::deps::frame_support::storage::IterableStorageMap;

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

			let mut prev_key: Option<Vec<u8>> = cursor.map(|c| c.into_inner());

			loop {
				if meter.try_consume(required).is_err() {
					break;
				}

				// In-place rewrite: `translate_next` reads each entry under the
				// v3 decoder (`LegacyAuthorization`) and writes v4 bytes
				// (`Authorization<T>`) at the same key.
				let next = Authorizations::<T>::translate_next::<
					LegacyAuthorization<BlockNumberFor<T>>,
					_,
				>(prev_key.take(), |scope, legacy| {
					// Drop already-expired entries (under the v3 parachain
					// clock) or zero-allowance entries.
					if legacy.expiration <= parachain_now || legacy.extent.bytes_allowance == 0 {
						return None;
					}
					let slot = TimedAuthorization {
						extent: AuthorizationExtent {
							bytes: 0,
							bytes_permanent: 0,
							transactions: 0,
							bytes_allowance: legacy.extent.bytes_allowance,
							transactions_allowance: legacy.extent.transactions_allowance,
						},
						starts_at: relay_now,
						expiration: new_expiration,
					};
					let mut slots =
						BoundedVec::<TimedAuthorization, T::MaxAuthorizationSlots>::new();
					slots
						.try_push(slot)
						.expect("MaxAuthorizationSlots > 0 by integrity_test; one slot fits; qed");
					let auth = Authorization::<T> { slots };
					// `inc_providers` lives on the System pallet so it does
					// not touch the key being rewritten and is safe to call
					// from inside the `translate_next` closure.
					Pallet::<T>::authorization_added(&scope);
					Some(auth)
				});

				match next {
					Some(key) => prev_key = Some(key),
					None => {
						// Iteration exhausted. Pin to v4 explicitly;
						// `in_code_storage_version` may be higher if later
						// versions chain on top.
						StorageVersion::new(4).put::<Pallet<T>>();
						return Ok(None);
					},
				}
			}

			let cursor = prev_key
				.map(BoundedVec::try_from)
				.transpose()
				.map_err(|_| SteppedMigrationError::Failed)?;
			Ok(cursor)
		}

		#[cfg(feature = "try-runtime")]
		fn pre_upgrade() -> Result<Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			let mut total: u32 = 0;
			let mut zero_allowance: u32 = 0;
			for (_, legacy) in LegacyAuthorizations::<T>::iter() {
				total = total.saturating_add(1);
				if legacy.extent.bytes_allowance == 0 {
					zero_allowance = zero_allowance.saturating_add(1);
				}
			}
			tracing::info!(
				target: LOG_TARGET,
				total,
				zero_allowance,
				"v3->v4 pre_upgrade: legacy entries",
			);
			Ok((total, zero_allowance).encode())
		}

		#[cfg(feature = "try-runtime")]
		fn post_upgrade(
			state: Vec<u8>,
		) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			use polkadot_sdk_frame::prelude::GetStorageVersion;

			let (total, zero_allowance) = <(u32, u32)>::decode(&mut &state[..])
				.map_err(|_| "Failed to decode pre_upgrade state")?;

			let mut new_count: u32 = 0;
			for (_, auth) in Authorizations::<T>::iter() {
				new_count = new_count.saturating_add(1);
				polkadot_sdk_frame::prelude::ensure!(
					!auth.slots.is_empty(),
					"v3->v4 post_upgrade: Authorization has empty slots",
				);
				for slot in auth.slots.iter() {
					polkadot_sdk_frame::prelude::ensure!(
						slot.extent.bytes == 0 &&
							slot.extent.bytes_permanent == 0 &&
							slot.extent.transactions == 0,
						"v3->v4 post_upgrade: translated slot has non-zero used counters",
					);
				}
			}
			polkadot_sdk_frame::prelude::ensure!(
				new_count <= total,
				"v3->v4 post_upgrade: more v4 entries than legacy entries",
			);
			let drops = total.saturating_sub(new_count);
			polkadot_sdk_frame::prelude::ensure!(
				drops >= zero_allowance,
				"v3->v4 post_upgrade: drops < known zero-allowance drops",
			);
			polkadot_sdk_frame::prelude::ensure!(
				Pallet::<T>::on_chain_storage_version() >= StorageVersion::new(4),
				"v3->v4 post_upgrade: storage version not bumped",
			);
			tracing::info!(
				target: LOG_TARGET,
				total,
				new_count,
				drops,
				zero_allowance,
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

/// V4 → V5 migration: re-encode each [`AutoRenewals`] entry from
/// `{ account }` (pre-paid-flag) to `{ account, recurring: true, paid: false }`.
///
/// All existing entries were written by the old fee-paying `enable_auto_renew`,
/// which:
///
/// - is the forever-renewal path, so the entries map to `recurring: true`;
/// - did **not** pre-pay the next cycle against the owner's authorization, so they map to `paid:
///   false` — `do_process_auto_renewals` will charge them per-cycle, preserving their on-chain
///   behaviour across the upgrade.
///
/// The new one-shot path (`recurring: false`) and the new prepaid path
/// (`paid: true`, set by both `renew` and the new `enable_auto_renew`) are only
/// reachable through the v5 extrinsics, which can't have written any entries
/// before this migration runs.
pub mod v5 {
	use super::*;
	use crate::{
		pallet::{AutoRenewals, Pallet},
		RenewalData, WeightInfo,
	};
	use bulletin_transaction_storage_primitives::ContentHash;
	use polkadot_sdk_frame::deps::{
		frame_support::{
			migrations::{MigrationId, SteppedMigration, SteppedMigrationError},
			weights::WeightMeter,
		},
		sp_io,
	};

	const MIGRATIONS_ID: &[u8; 24] = b"bulletin-tx-storage-vmig";

	/// `AutoRenewalData` layout pre-v5 (no `recurring` / `paid` fields). Used
	/// only for decoding pre-migration entries; never written.
	#[derive(Encode, Decode, Clone, Debug, MaxEncodedLen)]
	pub(crate) struct PreV5AutoRenewalData<AccountId> {
		pub account: AccountId,
	}

	/// Stepped migration from storage version 4 to 5.
	pub struct MigrateV4ToV5<T: Config>(PhantomData<T>);

	impl<T: Config> SteppedMigration for MigrateV4ToV5<T> {
		type Cursor = ContentHash;
		type Identifier = MigrationId<24>;

		fn id() -> Self::Identifier {
			MigrationId { pallet_id: *MIGRATIONS_ID, version_from: 4, version_to: 5 }
		}

		fn step(
			mut cursor: Option<Self::Cursor>,
			meter: &mut WeightMeter,
		) -> Result<Option<Self::Cursor>, SteppedMigrationError> {
			let required = T::WeightInfo::migrate_v4_to_v5_step();
			if meter.remaining().any_lt(required) {
				return Err(SteppedMigrationError::InsufficientWeight { required });
			}

			loop {
				if meter.try_consume(required).is_err() {
					break;
				}

				let mut iter = match cursor.as_ref() {
					None => AutoRenewals::<T>::iter_keys(),
					Some(last) =>
						AutoRenewals::<T>::iter_keys_from(AutoRenewals::<T>::hashed_key_for(last)),
				};

				let Some(content_hash) = iter.next() else {
					polkadot_sdk_frame::deps::frame_support::traits::StorageVersion::new(5)
						.put::<Pallet<T>>();
					cursor = None;
					break;
				};

				let raw_key = AutoRenewals::<T>::hashed_key_for(content_hash);

				let Some(raw) = sp_io::storage::get(&raw_key) else {
					cursor = Some(content_hash);
					continue;
				};

				// Idempotent: if it's already v5, skip.
				if RenewalData::<T::AccountId>::decode(&mut &raw[..]).is_ok() {
					cursor = Some(content_hash);
					continue;
				}

				let legacy = PreV5AutoRenewalData::<T::AccountId>::decode(&mut &raw[..])
					.map_err(|_| SteppedMigrationError::Failed)?;

				AutoRenewals::<T>::insert(
					content_hash,
					RenewalData { account: legacy.account, recurring: true, paid: false },
				);
				cursor = Some(content_hash);
			}

			Ok(cursor)
		}

		#[cfg(feature = "try-runtime")]
		fn pre_upgrade() -> Result<Vec<u8>, polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			use polkadot_sdk_frame::deps::frame_support::storage::StoragePrefixedMap;
			let prefix = AutoRenewals::<T>::final_prefix();
			let mut previous_key = prefix.to_vec();
			let mut count: u64 = 0;
			while let Some(key) =
				sp_io::storage::next_key(&previous_key).filter(|k| k.starts_with(&prefix))
			{
				previous_key = key;
				count += 1;
			}
			tracing::info!(target: LOG_TARGET, count, "v4->v5 pre_upgrade: AutoRenewals entries");
			Ok(count.encode())
		}

		#[cfg(feature = "try-runtime")]
		fn post_upgrade(
			state: Vec<u8>,
		) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::TryRuntimeError> {
			use polkadot_sdk_frame::deps::frame_support::storage::StoragePrefixedMap;

			let old_count =
				u64::decode(&mut &state[..]).map_err(|_| "Failed to decode pre_upgrade state")?;

			let prefix = AutoRenewals::<T>::final_prefix();
			let mut previous_key = prefix.to_vec();
			let mut new_count: u64 = 0;
			while let Some(key) =
				sp_io::storage::next_key(&previous_key).filter(|k| k.starts_with(&prefix))
			{
				previous_key = key.clone();
				let raw = sp_io::storage::get(&key)
					.ok_or("v4->v5 post_upgrade: missing AutoRenewals entry")?;
				let decoded = RenewalData::<T::AccountId>::decode(&mut &raw[..])
					.map_err(|_| "v4->v5 post_upgrade: remaining entry is not v5")?;
				polkadot_sdk_frame::prelude::ensure!(
					decoded.recurring,
					"v4->v5 post_upgrade: migrated entry must have recurring=true",
				);
				polkadot_sdk_frame::prelude::ensure!(
					!decoded.paid,
					"v4->v5 post_upgrade: migrated entry must have paid=false",
				);
				new_count += 1;
			}

			polkadot_sdk_frame::prelude::ensure!(
				new_count == old_count,
				"v4->v5 post_upgrade: entry count changed",
			);
			tracing::info!(
				target: LOG_TARGET,
				old_count,
				new_count,
				"v4->v5 post_upgrade: valid"
			);
			Ok(())
		}
	}
}
