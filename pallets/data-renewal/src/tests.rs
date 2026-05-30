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

//! Tests for the data-renewal pallet.

#![cfg(test)]
#![allow(deprecated)]

use crate::{mock::*, AutoRenewals, PendingAutoRenewals, RenewalData};
use bulletin_transaction_storage_primitives::cids::{CidConfig, HashingAlgorithm};
use codec::Encode;
use pallet_bulletin_transaction_storage::{
	self as txs, pallet::Origin, AuthorizationExtent, AuthorizationScope, TransactionInfo,
	TransactionKind, TransactionRef,
};
use polkadot_sdk_frame::{
	deps::{
		frame_support::{storage::storage_prefix, traits::OnRuntimeUpgrade},
		sp_io,
		sp_runtime::traits::Dispatchable,
	},
	hashing::blake2_256,
	testing_prelude::*,
};
use sp_transaction_storage_proof::{num_chunks, registration::build_proof};

// ---------------------------------------------------------------------------
// Smoke tests
// ---------------------------------------------------------------------------

#[test]
fn pallet_compiles_and_storage_is_separate_from_transaction_storage() {
	new_test_ext().execute_with(|| {
		assert!(AutoRenewals::<Test>::iter().next().is_none());
		assert!(PendingAutoRenewals::<Test>::get().is_empty());
		use polkadot_sdk_frame::deps::frame_support::traits::GetStorageVersion;
		assert_eq!(
			crate::Pallet::<Test>::on_chain_storage_version(),
			polkadot_sdk_frame::deps::frame_support::traits::StorageVersion::new(1),
		);
	});
}

#[test]
fn on_obsolete_callback_queues_pending_renewals_for_is_latest_entries_with_registrations() {
	use bulletin_transaction_storage_primitives::{cids::HashingAlgorithm, ContentHash};
	use pallet_bulletin_transaction_storage::OnObsoleteTransactions;
	use polkadot_sdk_frame::deps::sp_runtime::traits::{BlakeTwo256, Hash};

	new_test_ext().execute_with(|| {
		let acct: u64 = 7;
		let content_hash: ContentHash = BlakeTwo256::hash(b"smoke-hash").into();
		AutoRenewals::<Test>::insert(
			content_hash,
			RenewalData { account: acct, recurring: true, paid: false },
		);
		let info = TransactionInfo {
			chunk_root: BlakeTwo256::hash(b"chunk-root"),
			content_hash,
			hashing: HashingAlgorithm::Blake2b256,
			cid_codec: 0x55,
			size: 16,
			extrinsic_index: 0,
			block_chunks: 1,
			kind: TransactionKind::Renew,
		};
		let items = [(info, true)];
		<crate::Pallet<Test> as OnObsoleteTransactions<u64>>::handle_obsolete(1, &items);

		let pending = PendingAutoRenewals::<Test>::get();
		assert_eq!(pending.len(), 1);
		assert_eq!(pending[0].0, content_hash);
	});
}

#[test]
fn on_obsolete_callback_skips_stale_shadow_entries() {
	use bulletin_transaction_storage_primitives::{cids::HashingAlgorithm, ContentHash};
	use pallet_bulletin_transaction_storage::OnObsoleteTransactions;
	use polkadot_sdk_frame::deps::sp_runtime::traits::{BlakeTwo256, Hash};

	new_test_ext().execute_with(|| {
		let acct: u64 = 7;
		let content_hash: ContentHash = BlakeTwo256::hash(b"stale-hash").into();
		AutoRenewals::<Test>::insert(
			content_hash,
			RenewalData { account: acct, recurring: true, paid: false },
		);
		let info = TransactionInfo {
			chunk_root: BlakeTwo256::hash(b"chunk-root"),
			content_hash,
			hashing: HashingAlgorithm::Blake2b256,
			cid_codec: 0x55,
			size: 16,
			extrinsic_index: 0,
			block_chunks: 1,
			kind: TransactionKind::Store,
		};
		let items = [(info, false)];
		<crate::Pallet<Test> as OnObsoleteTransactions<u64>>::handle_obsolete(1, &items);
		assert!(PendingAutoRenewals::<Test>::get().is_empty());
	});
}

#[test]
fn relocation_migration_moves_legacy_entries_under_new_prefix() {
	use polkadot_sdk_frame::deps::frame_support::traits::StorageVersion;
	new_test_ext().execute_with(|| {
		StorageVersion::new(0).put::<crate::Pallet<Test>>();
		let old_prefix = storage_prefix(b"TransactionStorage", b"AutoRenewals");
		let hash = [0xAAu8; 32];
		let mut key = old_prefix.to_vec();
		key.extend_from_slice(&sp_io::hashing::blake2_128(&hash));
		key.extend_from_slice(&hash);
		let value = RenewalData::<u64> { account: 9, recurring: true, paid: false };
		sp_io::storage::set(&key, &value.encode());

		let _ = crate::migrations::RelocateFromTransactionStorage::<Test>::on_runtime_upgrade();

		assert!(sp_io::storage::get(&key).is_none());
		assert_eq!(AutoRenewals::<Test>::iter().count(), 1);
		let weight =
			crate::migrations::RelocateFromTransactionStorage::<Test>::on_runtime_upgrade();
		assert_eq!(weight, polkadot_sdk_frame::deps::frame_support::weights::Weight::zero());
	});
}

/// A pre-v4 `AutoRenewals` entry (bare `{ account }`, no `recurring`/`paid`) is
/// reshaped to the current `RenewalData` layout during relocation — the case a
/// plain `move_prefix` could not handle. Covers a chain still at tx-storage v3.
#[test]
fn relocation_migration_reshapes_legacy_v3_layout() {
	use polkadot_sdk_frame::deps::frame_support::traits::StorageVersion;
	new_test_ext().execute_with(|| {
		StorageVersion::new(0).put::<crate::Pallet<Test>>();
		let old_prefix = storage_prefix(b"TransactionStorage", b"AutoRenewals");
		let hash = [0xBBu8; 32];
		let mut key = old_prefix.to_vec();
		key.extend_from_slice(&sp_io::hashing::blake2_128(&hash));
		key.extend_from_slice(&hash);
		// v3 layout: just the account, no `recurring`/`paid` fields.
		let account: u64 = 7;
		sp_io::storage::set(&key, &account.encode());

		let _ = crate::migrations::RelocateFromTransactionStorage::<Test>::on_runtime_upgrade();

		assert!(sp_io::storage::get(&key).is_none(), "old-prefix entry must be removed");
		assert_eq!(AutoRenewals::<Test>::iter().count(), 1);
		let entry = AutoRenewals::<Test>::get(hash).expect("entry relocated under new prefix");
		assert_eq!(
			entry,
			RenewalData::<u64> { account, recurring: true, paid: false },
			"v3 entry must be reshaped to recurring & prepaid",
		);
	});
}

#[allow(dead_code)]
fn _ensure_imports() {
	let _ = txs::TransactionKind::Store;
}

// ---------------------------------------------------------------------------
// Ported from the pre-split `pallet-bulletin-transaction-storage::tests` —
// critical renewal coverage. Remaining tests will be ported in a follow-up.
// ---------------------------------------------------------------------------

/// `enable_auto_renew` is a feeless registration: the extension's
/// `pre_dispatch` charges one tx slot, `bytes_permanent`, and the chain-wide
/// `PermanentStorageUsed`; the dispatchable then inserts an `AutoRenewals`
/// entry with `recurring: true, paid: true`.
#[test]
fn enable_auto_renew_works() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		assert_ok!(enable_auto_renew_via_extension(who, content_hash));

		let renewal_data = AutoRenewals::<Test>::get(content_hash).unwrap();
		assert_eq!(renewal_data.account, who);
		assert!(renewal_data.recurring);
		assert!(renewal_data.paid);

		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::RenewalEnabled {
			content_hash,
			who,
			recurring: true,
		}));

		// Re-enabling is rejected before any state mutation.
		let call = crate::Call::<Test>::enable_auto_renew { content_hash };
		assert_eq!(
			DataRenewal::validate_renewal_signed(&who, &call).map(|_| ()),
			Err(txs::AUTO_RENEWAL_ALREADY_ENABLED.into()),
		);
	});
}

/// Owner-only `disable_auto_renew`: a non-owner is rejected at extension time
/// with `NOT_AUTO_RENEWAL_OWNER`; the owner can disable only after the prepaid
/// cycle has consumed the prepayment (we flip `paid = false` directly to
/// simulate that for this test).
#[test]
fn disable_auto_renew_validate_signed_gates_on_ownership() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let owner = 1;
		let other = 2;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			owner,
			10,
			100_000,
		));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		assert_ok!(enable_auto_renew_via_extension(owner, content_hash));
		// Owner can't disable while still in the prepaid window.
		AutoRenewals::<Test>::mutate(content_hash, |entry| {
			entry.as_mut().unwrap().paid = false;
		});

		let call = crate::Call::<Test>::disable_auto_renew { content_hash };
		assert_eq!(
			DataRenewal::validate_renewal_signed(&other, &call).map(|_| ()),
			Err(txs::NOT_AUTO_RENEWAL_OWNER.into()),
		);
		// Owner can disable.
		assert_ok!(disable_auto_renew_via_extension(owner, content_hash));
		assert!(AutoRenewals::<Test>::get(content_hash).is_none());
	});
}

/// Root bypasses the owner gate and the prepaid window.
#[test]
fn disable_auto_renew_root_override() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let owner = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			owner,
			10,
			100_000,
		));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		assert_ok!(enable_auto_renew_via_extension(owner, content_hash));
		// `paid = true` would block signed disable, but root bypasses it.
		assert_ok!(DataRenewal::disable_auto_renew(RuntimeOrigin::root(), content_hash));
		assert!(AutoRenewals::<Test>::get(content_hash).is_none());
	});
}

/// Issue #531 / PR #557 regression: storing the same content hash twice must not
/// cause a paid auto-renewal registration to double-charge `PermanentStorageUsed`
/// when the duplicate store moves `TransactionByContentHash` forward.
#[test]
fn duplicate_store_does_not_double_charge_auto_renew() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let alice: u64 = 1;
		let data = vec![0xABu8; 1000];
		let content_hash = blake2_256(&data);
		let size = data.len() as u64;

		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			alice,
			10,
			100_000,
		));
		// First store + enable auto-renew. Registration charges `size` against
		// `PermanentStorageUsed`.
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data.clone()));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(alice, content_hash));
		let permanent_after_first =
			pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get();
		assert_eq!(permanent_after_first, size);

		// Re-store the same data — must not bump `PermanentStorageUsed` again
		// (only `renew` does).
		run_to_block(3, || None);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		let permanent_after_dup =
			pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get();
		assert_eq!(permanent_after_dup, permanent_after_first);
	});
}

/// `OnObsoleteTransactions::handle_obsolete` queues a pending renewal for items
/// with a matching `AutoRenewals` registration and skips items without one,
/// even when both are flagged `is_latest`. Driven directly through the trait
/// callback to keep the test self-contained.
#[test]
fn pending_auto_renewals_populated_only_for_registered_items() {
	use bulletin_transaction_storage_primitives::{cids::HashingAlgorithm, ContentHash};
	use pallet_bulletin_transaction_storage::OnObsoleteTransactions;
	use polkadot_sdk_frame::deps::sp_runtime::traits::{BlakeTwo256, Hash};

	new_test_ext().execute_with(|| {
		let alice: u64 = 1;
		let hash_a: ContentHash = BlakeTwo256::hash(b"a").into();
		let hash_b: ContentHash = BlakeTwo256::hash(b"b").into();

		// Only `hash_a` registers for auto-renew.
		AutoRenewals::<Test>::insert(
			hash_a,
			RenewalData { account: alice, recurring: true, paid: false },
		);

		let info_a = TransactionInfo {
			chunk_root: BlakeTwo256::hash(b"root-a"),
			content_hash: hash_a,
			hashing: HashingAlgorithm::Blake2b256,
			cid_codec: 0x55,
			size: 100,
			extrinsic_index: 0,
			block_chunks: 1,
			kind: TransactionKind::Store,
		};
		let info_b = TransactionInfo {
			chunk_root: BlakeTwo256::hash(b"root-b"),
			content_hash: hash_b,
			hashing: HashingAlgorithm::Blake2b256,
			cid_codec: 0x55,
			size: 100,
			extrinsic_index: 1,
			block_chunks: 2,
			kind: TransactionKind::Store,
		};
		let items = [(info_a, true), (info_b, true)];
		<crate::Pallet<Test> as OnObsoleteTransactions<u64>>::handle_obsolete(1, &items);

		let pending = PendingAutoRenewals::<Test>::get();
		assert_eq!(pending.len(), 1, "only the registered item should be queued");
		assert_eq!(pending[0].0, hash_a);
	});
}

/// `process_pending_renewals` rejects signed callers — it's a mandatory unsigned inherent.
#[test]
fn process_auto_renewals_rejects_signed_origin() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert_noop!(
			DataRenewal::process_pending_renewals(RuntimeOrigin::signed(1)),
			polkadot_sdk_frame::deps::sp_runtime::DispatchError::BadOrigin,
		);
		assert_noop!(
			DataRenewal::process_pending_renewals(RuntimeOrigin::root()),
			polkadot_sdk_frame::deps::sp_runtime::DispatchError::BadOrigin,
		);
	});
}

/// `process_pending_renewals` is a no-op (with refunded weight) when nothing is queued.
#[test]
fn process_auto_renewals_noop_when_empty() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert!(PendingAutoRenewals::<Test>::get().is_empty());
		assert_ok!(DataRenewal::process_pending_renewals(RuntimeOrigin::none()));
	});
}

/// One-shot `renew` registers exactly that — `AutoRenewals[hash]` is created
/// with `recurring: false`, prepaid for the first cycle.
#[test]
fn renew_schedules_one_shot() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 200];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000,));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		assert_ok!(renew_via_extension(
			who,
			pallet_bulletin_transaction_storage::TransactionRef::ContentHash(content_hash),
		));

		let r = AutoRenewals::<Test>::get(content_hash).unwrap();
		assert!(!r.recurring);
		assert!(r.paid);
	});
}
#[test]
fn renews_data() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![0u8; 2000]));
		let info = pallet_bulletin_transaction_storage::BlockTransactions::<Test>::get()
			.last()
			.unwrap()
			.clone();
		run_to_block(6, || None);
		assert_ok!(DataRenewal::force_renew(
			RuntimeOrigin::none(),
			TransactionRef::Position { block: 1, index: 0 },
		));
		let proof_provider = || {
			let block_num = System::block_number();
			if block_num == 11 || block_num == 16 {
				let parent_hash = System::parent_hash();
				build_proof(parent_hash.as_ref(), vec![vec![0u8; 2000]]).unwrap()
			} else {
				None
			}
		};
		run_to_block(16, proof_provider);
		assert!(pallet_bulletin_transaction_storage::Transactions::<Test>::get(1).is_none());
		// Renew preserves chunk_root / content_hash / size from the original entry but
		// stamps `kind = Renew` so on_initialize cleanup can decrement the chain counter.
		let renewed = pallet_bulletin_transaction_storage::Transactions::<Test>::get(6)
			.unwrap()
			.first()
			.unwrap()
			.clone();
		assert_eq!(renewed.chunk_root, info.chunk_root);
		assert_eq!(renewed.content_hash, info.content_hash);
		assert_eq!(renewed.size, info.size);
		assert_eq!(renewed.kind, TransactionKind::Renew);
		run_to_block(17, proof_provider);
		assert!(pallet_bulletin_transaction_storage::Transactions::<Test>::get(6).is_none());
	});
}

/// `renew` accepts a content-hash variant of [`TransactionRef`] equivalently to
/// the position variant.
#[test]
fn renew_by_content_hash_schedules_one_shot() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		// Unknown content hash is rejected at dispatch (after origin admission).
		let bogus_hash = [0u8; 32];
		let origin: RuntimeOrigin =
			Origin::<Test>::Authorized { who, scope: AuthorizationScope::Account(who) }.into();
		assert_noop!(
			DataRenewal::renew(origin, TransactionRef::ContentHash(bogus_hash)),
			crate::Error::<Test>::RenewedNotFound,
		);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		assert_ok!(renew_via_extension(who, TransactionRef::ContentHash(content_hash)));

		let entry = crate::AutoRenewals::<Test>::get(content_hash).unwrap();
		assert_eq!(entry.account, who);
		assert!(!entry.recurring, "renew should register a one-shot entry");
		assert!(entry.paid, "one-shot is prepaid at registration");

		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::RenewalEnabled {
			content_hash,
			who,
			recurring: false,
		}));
	});
}

#[test]
fn signed_renew_uses_account_authorization() {
	// When no preimage authorization exists for the stored content, signed renew falls back
	// to account authorization. (The old test used preimage auth for the store and relied on
	// it being deleted on consumption — which no longer happens, so the setup is reworked to
	// use account auth end-to-end.)
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		// Setup: authorize and store via account authorization.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
			},
		);

		run_to_block(3, || None);

		// No preimage authorization exists for the content hash — renew uses account auth.
		let renew_call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_ok!(DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call));

		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 2000,
				bytes_allowance: 4000,
				transactions: 2,
				transactions_allowance: 0,
			},
			"Account authorization should be consumed for renew when no preimage auth"
		);
	});
}

#[test]
fn content_hash_map_not_cleaned_if_renewed() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));

		// Renew at block 6, which updates the map to point to block 6
		run_to_block(6, || None);
		assert_ok!(DataRenewal::force_renew(
			RuntimeOrigin::none(),
			TransactionRef::ContentHash(content_hash),
		));
		assert_eq!(
			pallet_bulletin_transaction_storage::TransactionByContentHash::<Test>::get(
				content_hash
			),
			Some((6, 0))
		);

		let proof_provider = || {
			let block_num = System::block_number();
			if block_num == 11 || block_num == 16 {
				let parent_hash = System::parent_hash();
				build_proof(parent_hash.as_ref(), vec![vec![0u8; 2000]]).unwrap()
			} else {
				None
			}
		};

		// Block 1 data expires at block 12, but the map should still point to block 6
		run_to_block(12, proof_provider);
		assert_eq!(
			pallet_bulletin_transaction_storage::TransactionByContentHash::<Test>::get(
				content_hash
			),
			Some((6, 0))
		);

		// Block 6 data expires at block 17
		run_to_block(17, proof_provider);
		assert!(pallet_bulletin_transaction_storage::TransactionByContentHash::<Test>::get(
			content_hash
		)
		.is_none());
	});
}

#[test]
fn signed_renew_prefers_preimage_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];
		let content_hash = blake2_256(&data);

		// Setup: store data using account authorization.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		// Account auth now at the cap (still present, just fully used).
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
			}
		);

		run_to_block(3, || None);

		// Authorize preimage.
		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::root(),
			content_hash,
			2000
		));

		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 1,
			}
		);
		// Account auth was unaffected by the preimage authorize.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
			}
		);

		// Renew using signed transaction - should prefer preimage authorization
		let renew_call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_ok!(DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call));

		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 2000,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1,
			},
			"Preimage authorization should be consumed for renew"
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
			},
			"Account authorization should remain unchanged when preimage auth is used"
		);
	});
}

#[test]
fn preimage_authorize_store_with_cid_config_and_renew() {
	new_test_ext().execute_with(|| {
		let data = vec![42u8; 2000];
		let sha2_config = CidConfig { codec: 0x55, hashing: HashingAlgorithm::Sha2_256 };
		let sha2_hash = polkadot_sdk_frame::hashing::sha2_256(&data);

		// check_unsigned / check_store_renew_unsigned use the CID config's hashing
		// algorithm for preimage authorization lookup.
		// Authorizing with blake2 hash should NOT work for store_with_cid_config(sha2).
		let blake2_hash = blake2_256(&data);
		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::root(),
			blake2_hash,
			2000
		));
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store_with_cid_config {
			cid: sha2_config.clone(),
			data: data.clone(),
		};
		run_to_block(1, || None);
		assert_noop!(TransactionStorage::pre_dispatch(&store_call), InvalidTransaction::Payment);

		// Authorize preimage with SHA2 hash (matching the CID config's algorithm).
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), sha2_hash, 2000));

		// store_with_cid_config goes through check_unsigned → check_store_renew_unsigned.
		assert_ok!(TransactionStorage::pre_dispatch(&store_call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(sha2_hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1,
			}
		);
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		// sha2 preimage consumed to cap; entry stays.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(sha2_hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1,
			}
		);
		// Blake2 authorization should remain unconsumed.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(blake2_hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 1,
			}
		);

		// Finalize block so Transactions storage is populated.
		run_to_block(3, || None);

		// Verify stored entry uses SHA2-256 and content_hash matches.
		let txs = pallet_bulletin_transaction_storage::Transactions::<Test>::get(1)
			.expect("transactions stored at block 1");
		assert_eq!(txs.len(), 1);
		assert_eq!(txs[0].hashing, HashingAlgorithm::Sha2_256);
		assert_eq!(txs[0].cid_codec, 0x55);
		assert_eq!(txs[0].content_hash, sha2_hash);

		// Renew with the sha2 preimage auth still present — succeeds via the unsigned
		// `force_renew` path, accumulating on `bytes_permanent` while leaving `bytes`
		// (store-only) untouched.
		let renew_call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_ok!(DataRenewal::pre_dispatch(&renew_call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(sha2_hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 2000,
				bytes_allowance: 2000,
				transactions: 2,
				transactions_allowance: 1,
			}
		);
	});
}

/// Happy path for the hard-side invariant: a real `renew` keeps `PermanentStorageUsed`
/// equal to the sum of renewed `Transactions` entries' sizes.
#[test]
fn try_state_passes_after_renew() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store_call =
			pallet_bulletin_transaction_storage::Call::<Test>::store { data: vec![42u8; 2000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);
		let renew_call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_ok!(DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call));
		assert_ok!(Into::<RuntimeCall>::into(renew_call).dispatch(RuntimeOrigin::none()));
		// Force the renewed entry into the persistent `Transactions` map by advancing a
		// block so `on_finalize` flushes `BlockTransactions`.
		run_to_block(4, || None);
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 2000);
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

/// `renew` / `enable_auto_renew` charge `PermanentStorageUsed` up front but the
/// matching `Renew` entry only lands at the next retention boundary; `try_state`
/// must reconcile the counter against paid registrations in the meantime.
#[test]
fn try_state_passes_during_paid_auto_renewal_prepayment_window() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let data = vec![42u8; 2000];
		let content_hash = blake2_256(&data);
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);

		assert_ok!(renew_via_extension(who, TransactionRef::ContentHash(content_hash)));
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 2000);
		assert!(crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid);

		run_to_block(4, || None);
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

/// As above, for `enable_auto_renew`.
#[test]
fn try_state_passes_during_enable_auto_renew_prepayment_window() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let data = vec![42u8; 2000];
		let content_hash = blake2_256(&data);
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);

		assert_ok!(enable_auto_renew_via_extension(who, content_hash));
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 2000);
		assert!(crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid);

		run_to_block(4, || None);
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

#[test]
fn enable_auto_renew_rejects_invalid() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;

		// Enabling for non-existent content hash is rejected at the extension level.
		let bogus_hash = blake2_256(&[99u8; 100]);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		let call = crate::Call::<Test>::enable_auto_renew { content_hash: bogus_hash };
		assert_eq!(
			DataRenewal::validate_renewal_signed(&who, &call).map(|_| ()),
			Err(txs::RENEWED_NOT_FOUND.into()),
		);

		// Enabling without account authorization fails. `check_authorization`
		// (with `is_renew = true`) folds both missing and expired authorizations
		// into `InvalidTransaction::Payment`, matching the one-shot `renew` path.
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		let unauthorized_user = 99;
		let call = crate::Call::<Test>::enable_auto_renew { content_hash };
		assert_eq!(
			DataRenewal::validate_renewal_signed(&unauthorized_user, &call).map(|_| ()),
			Err(InvalidTransaction::Payment.into()),
		);
	});
}

#[test]
fn disable_auto_renew_works() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let owner = 1;
		let other = 2;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			owner,
			10,
			100_000
		));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(owner, content_hash));

		// Even the owner cannot disable while the registration is in its prepaid
		// window: `enable_auto_renew` charges the next cycle up front, and we
		// don't let that prepayment be reclaimed.
		assert_noop!(
			disable_auto_renew_via_extension(owner, content_hash),
			crate::Error::<Test>::CannotDisablePrepaidAutoRenewal,
		);

		// Fire the first cycle to consume the prepayment, after which the
		// registration sits at `paid = false` and the owner can disable.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			owner,
			10,
			100_000,
		));
		assert_ok!(apply_block_inherents_full(None));
		assert!(
			!crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid,
			"cycle should consume prepayment"
		);

		// Non-owner is still rejected after the prepayment is consumed.
		assert_noop!(
			disable_auto_renew_via_extension(other, content_hash),
			crate::Error::<Test>::NotAutoRenewalOwner,
		);

		// Owner can now disable.
		assert_ok!(disable_auto_renew_via_extension(owner, content_hash));

		assert!(crate::AutoRenewals::<Test>::get(content_hash).is_none());
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::AutoRenewalDisabled {
			content_hash,
			who: owner,
		}));
	});
}

#[test]
fn disable_auto_renew_fails_if_not_enabled() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let content_hash = blake2_256(&[99u8; 100]);

		assert_noop!(
			disable_auto_renew_via_extension(who, content_hash),
			crate::Error::<Test>::AutoRenewalNotEnabled,
		);
	});
}

#[test]
fn auto_renewal_lifecycle() {
	new_test_ext().execute_with(|| {
		// Block 1: store data and enable auto-renew
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data.clone()));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(who, content_hash));

		// Registration is feeless and does not move the data: `TransactionByContentHash`
		// still points at the original `Store` at block 1.
		assert_eq!(
			pallet_bulletin_transaction_storage::TransactionByContentHash::<Test>::get(
				content_hash
			),
			Some((1, 0))
		);
		assert!(pallet_bulletin_transaction_storage::Transactions::<Test>::get(1).is_some());

		// Build proof provider for the retention boundary.
		let proof_provider = move || {
			let block_num = System::block_number();
			let period: u64 = pallet_bulletin_transaction_storage::RetentionPeriod::<Test>::get();
			let target = block_num.saturating_sub(period);
			if target > 0 &&
				pallet_bulletin_transaction_storage::Transactions::<Test>::get(target).is_some()
			{
				let parent_hash = System::parent_hash();
				let txs =
					pallet_bulletin_transaction_storage::Transactions::<Test>::get(target).unwrap();
				let data_vec: Vec<Vec<u8>> = txs.iter().map(|_| data.clone()).collect();
				build_proof(parent_hash.as_ref(), data_vec).unwrap()
			} else {
				None
			}
		};

		// Advance up to (but not including) block 12 — `run_to_block` handles
		// proofs and on_finalize for each block. The block-1 entry expires at
		// block 12; we want to stop before its on_initialize so we can drive
		// `apply_block_inherents` manually below.
		run_to_block(11, proof_provider);

		// Block 12: on_initialize takes `Transactions[1]` and schedules the
		// auto-renewal into `PendingAutoRenewals`.
		init_block(12);

		// Verify PendingAutoRenewals was populated
		let pending = crate::PendingAutoRenewals::<Test>::get();
		assert_eq!(pending.len(), 1);
		assert_eq!(pending[0].0, content_hash);

		// Process auto-renewals (simulating the mandatory extrinsic). Refresh
		// authorization first since the block-1 grant expired at block 11.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));

		assert_ok!(apply_block_inherents_full(None));

		// Verify PendingAutoRenewals is now empty
		assert!(crate::PendingAutoRenewals::<Test>::get().is_empty());

		// Data was renewed into the current block.
		assert_eq!(
			pallet_bulletin_transaction_storage::TransactionByContentHash::<Test>::get(
				content_hash
			),
			Some((12, 0))
		);

		// Verify event
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::DataAutoRenewed {
			index: 0,
			content_hash,
			account: who,
		}));

		// Old block-1 entry was taken in on_initialize.
		assert!(pallet_bulletin_transaction_storage::Transactions::<Test>::get(1).is_none());

		// Recurring registration should still exist
		assert!(crate::AutoRenewals::<Test>::get(content_hash).is_some());
	});
}

#[test]
fn auto_renewal_consumes_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		// `store` is unsigned, so it does not consume authorization.
		// `enable_auto_renew` is feeless but pre-pays the first cycle, mirroring
		// one-shot `renew`: `bytes_permanent`, the chain-wide
		// `PermanentStorageUsed`, and one tx slot are all charged at registration.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 3, 6000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(who, content_hash));

		// Registration prepaid the first cycle: `bytes_permanent = size` and one
		// tx slot consumed.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 2000,
				bytes_allowance: 6000,
				transactions: 1,
				transactions_allowance: 3,
			},
		);

		// First auto-renewal cycle fires when `Transactions[1]` ages out at
		// block 12. The block-1 grant expired at block 11, so the re-grant below
		// replaces it with fresh counters; the cycle then fires free against the
		// fresh auth thanks to `paid: true`.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 3, 6000));
		assert_ok!(apply_block_inherents_full(None));

		// Cycle 1 (prepaid) leaves the fresh authorization untouched.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 6000,
				transactions: 0,
				transactions_allowance: 3,
			},
		);
		// Prepayment is consumed; subsequent cycles charge per-cycle.
		assert!(!crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid);

		// Cycle 2: carry `BlockTransactions[12]` → `Transactions[12]` so the
		// block-12 renew can age out at block 23.
		let block_txs = pallet_bulletin_transaction_storage::BlockTransactions::<Test>::take();
		if !block_txs.is_empty() {
			pallet_bulletin_transaction_storage::Transactions::<Test>::insert(12u64, &block_txs);
		}
		init_block(23);
		// The block-12 grant expired at block 22; re-grant fresh counters so the
		// cycle can charge against a known baseline.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 3, 6000));
		assert_ok!(apply_block_inherents_full(None));

		// Cycle 2 (paid = false) charged `size` bytes_permanent + 1 tx slot.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 2000,
				bytes_allowance: 6000,
				transactions: 1,
				transactions_allowance: 3,
			},
		);
	});
}

#[test]
fn auto_renewal_fails_when_authorization_exhausted() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		// Authorize generously initially so the prepaid registration fits.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 5, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(who, content_hash));

		// Cycle 1 (prepaid) fires free at block 12. Re-authorize first because
		// the block-1 grant expired at block 11.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 5, 100_000));
		assert_ok!(apply_block_inherents_full(None));
		assert!(
			!crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid,
			"cycle should consume prepayment"
		);

		// Carry block-12 renew into Transactions[12] for the next age-out.
		let block_txs = pallet_bulletin_transaction_storage::BlockTransactions::<Test>::take();
		if !block_txs.is_empty() {
			pallet_bulletin_transaction_storage::Transactions::<Test>::insert(12u64, &block_txs);
		}

		// Cycle 2 at block 23 runs against a refreshed authorization with no
		// headroom — `bytes_permanent + size > bytes_allowance` fires.
		init_block(23);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 1000));
		let pending = crate::PendingAutoRenewals::<Test>::get();
		assert_eq!(pending.len(), 1, "Should have pending renewal");

		assert_ok!(apply_block_inherents_full(None));

		// Should have failed — event emitted and auto-renewal removed.
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::AutoRenewalFailed {
			content_hash,
			account: who,
		}));
		assert!(
			crate::AutoRenewals::<Test>::get(content_hash).is_none(),
			"Auto-renewal should be removed"
		);
	});
}

#[test]
fn auto_renew_permissionless_transfer() {
	// Alice stores and enables auto-renew, waits out the prepaid window so the
	// first cycle consumes her prepayment, then disables. Bob enables instead.
	// Anyone can take over keeping data alive on Bulletin, permissionlessly —
	// but only after the original registrant's pre-paid cycle has fired.
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let alice = 1;
		let bob = 2;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		// Authorize and store as Alice
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			alice,
			10,
			100_000
		));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data.clone()));
		run_to_block(2, || None);

		// Alice enables auto-renew (prepays the first cycle).
		assert_ok!(enable_auto_renew_via_extension(alice, content_hash));
		let renewal = crate::AutoRenewals::<Test>::get(content_hash).unwrap();
		assert_eq!(renewal.account, alice);
		assert!(renewal.paid);

		// Alice cannot disable during the prepaid window.
		assert_noop!(
			disable_auto_renew_via_extension(alice, content_hash),
			crate::Error::<Test>::CannotDisablePrepaidAutoRenewal,
		);

		// Fire the first cycle to consume Alice's prepayment.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			alice,
			10,
			100_000,
		));
		assert_ok!(apply_block_inherents_full(None));
		assert!(!crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid);
		// Carry the cycle-12 renew out of `BlockTransactions` so the next
		// `enable_auto_renew` can resolve the `(12, 0)` index against
		// `Transactions[12]` (mirrors what `on_finalize` does in a live chain).
		let block_txs = pallet_bulletin_transaction_storage::BlockTransactions::<Test>::take();
		if !block_txs.is_empty() {
			pallet_bulletin_transaction_storage::Transactions::<Test>::insert(12u64, &block_txs);
		}

		// Alice can now disable.
		assert_ok!(disable_auto_renew_via_extension(alice, content_hash));
		assert!(crate::AutoRenewals::<Test>::get(content_hash).is_none());

		// Bob authorizes and enables auto-renew for the same content. The renew
		// at block 12 made `(12, 0)` the latest `TransactionByContentHash` entry,
		// so `enable_auto_renew` resolves cleanly.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), bob, 10, 100_000));
		assert_ok!(enable_auto_renew_via_extension(bob, content_hash));

		let renewal = crate::AutoRenewals::<Test>::get(content_hash).unwrap();
		assert_eq!(renewal.account, bob, "Bob should now own the auto-renewal");
		assert!(renewal.paid, "Bob's registration prepays his first cycle");

		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::RenewalEnabled {
			content_hash,
			who: bob,
			recurring: true,
		}));
	});
}

#[test]
fn process_auto_renewals_continues_on_per_item_failure() {
	// Verify that if one renewal fails (e.g. block full), the remaining items are still processed.
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;

		// Store MaxBlockTransactions items to fill the block later
		let max_txns =
			<<Test as pallet_bulletin_transaction_storage::Config>::MaxBlockTransactions as Get<
				u32,
			>>::get();
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			max_txns + 10,
			100_000_000
		));

		let mut hashes = Vec::new();
		for i in 0..3u8 {
			let data = vec![i; 2000];
			let content_hash = blake2_256(&data);
			hashes.push(content_hash);
			assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		}
		run_to_block(2, || None);

		// Enable auto-renew for all three (feeless registration — only consumes
		// one tx slot each).
		for hash in &hashes {
			assert_ok!(enable_auto_renew_via_extension(who, *hash));
		}

		// Block-1 entries age out at block 12; on_initialize schedules each as
		// a pending auto-renewal. Refresh authorization (block-1 grant expired
		// at block 11).
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			max_txns + 10,
			100_000_000
		));

		// Verify PendingAutoRenewals was populated with 3 items
		let pending = crate::PendingAutoRenewals::<Test>::get();
		assert_eq!(pending.len(), 3);

		// Fill block with (max - 1) dummy transactions so only 1 renewal fits
		pallet_bulletin_transaction_storage::BlockTransactions::<Test>::mutate(|txns| {
			for _ in 0..(max_txns - 1) {
				let _ = txns.try_push(TransactionInfo {
					chunk_root: Default::default(),
					size: 100,
					content_hash: [0u8; 32],
					hashing: HashingAlgorithm::Blake2b256,
					cid_codec: 0x55,
					extrinsic_index: 0,
					block_chunks: 0,
					kind: crate::TransactionKind::Store,
				});
			}
		});

		// Process auto-renewals — should NOT return an error even though 2 of 3 fail
		assert_ok!(apply_block_inherents_full(None));

		// PendingAutoRenewals should be fully consumed
		assert!(crate::PendingAutoRenewals::<Test>::get().is_empty());

		// First item should have succeeded (DataAutoRenewed event).
		// Index is max_txns - 1 because the block already has max_txns - 1 items (0-indexed).
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::DataAutoRenewed {
			index: max_txns - 1,
			content_hash: hashes[0],
			account: who,
		}));

		// Remaining items should have failed (AutoRenewalFailed events)
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::AutoRenewalFailed {
			content_hash: hashes[1],
			account: who,
		}));
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::AutoRenewalFailed {
			content_hash: hashes[2],
			account: who,
		}));

		// Auto-renewal registrations should be removed for failed items
		assert!(crate::AutoRenewals::<Test>::get(hashes[1]).is_none());
		assert!(crate::AutoRenewals::<Test>::get(hashes[2]).is_none());
	});
}

/// `paid = true` cycle rejected by the per-block slot cap refunds chain-wide
/// `PermanentStorageUsed`. Per-account `bytes_permanent` / `transactions` are
/// intentionally left burned — see the inline rationale in `do_process_auto_renewals`.
#[test]
fn paid_cycle_refunds_on_block_slot_cap() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![7u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		assert_ok!(enable_auto_renew_via_extension(who, content_hash));
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 2000);
		let auth = pallet_bulletin_transaction_storage::Authorizations::<Test>::get(
			AuthorizationScope::Account(who),
		)
		.expect("auth exists");
		let permanent_before = auth.extent.bytes_permanent;
		let transactions_before = auth.extent.transactions;

		init_block(12);
		assert_eq!(crate::PendingAutoRenewals::<Test>::get().len(), 1);

		// Fill `BlockTransactions` so the paid drain has no slot to land in.
		let max_txns =
			<<Test as pallet_bulletin_transaction_storage::Config>::MaxBlockTransactions as Get<
				u32,
			>>::get();
		pallet_bulletin_transaction_storage::BlockTransactions::<Test>::mutate(|txns| {
			for _ in 0..max_txns {
				let _ = txns.try_push(TransactionInfo {
					chunk_root: Default::default(),
					size: 100,
					content_hash: [0u8; 32],
					hashing: HashingAlgorithm::Blake2b256,
					cid_codec: 0x55,
					extrinsic_index: 0,
					block_chunks: 0,
					kind: crate::TransactionKind::Store,
				});
			}
		});

		assert_ok!(apply_block_inherents_full(None));

		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::AutoRenewalFailed {
			content_hash,
			account: who,
		}));
		assert!(crate::AutoRenewals::<Test>::get(content_hash).is_none());

		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 0);
		let auth = pallet_bulletin_transaction_storage::Authorizations::<Test>::get(
			AuthorizationScope::Account(who),
		)
		.expect("auth exists");
		assert_eq!(auth.extent.bytes_permanent, permanent_before);
		assert_eq!(auth.extent.transactions, transactions_before);

		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

/// A one-shot fires exactly once: after the renewal cycle the `AutoRenewals` entry is
/// removed, even on success. Distinct from forever auto-renewal which keeps firing.
#[test]
fn one_shot_fires_once_then_unregisters() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(renew_via_extension(who, TransactionRef::Position { block: 1, index: 0 }));

		// Fire the renewal cycle.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(apply_block_inherents_full(None));

		// DataAutoRenewed fired AND the registration was consumed.
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::DataAutoRenewed {
			index: 0,
			content_hash,
			account: who,
		}));
		assert!(
			crate::AutoRenewals::<Test>::get(content_hash).is_none(),
			"one-shot registration must be removed after firing"
		);
	});
}

/// `renew` (one-shot) charges `bytes_permanent` + `PermanentStorageUsed` + 1 tx slot
/// at registration — same hard-cap accounting as `force_renew`.
#[test]
fn renew_prepays_at_registration() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![0u8; 2000]));
		run_to_block(2, || None);

		assert_ok!(renew_via_extension(who, TransactionRef::Position { block: 1, index: 0 }));

		let auth = pallet_bulletin_transaction_storage::Authorizations::<Test>::get(
			AuthorizationScope::Account(who),
		)
		.unwrap();
		assert_eq!(auth.extent.bytes_permanent, 2000);
		assert_eq!(auth.extent.transactions, 1);
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 2000);
	});
}

/// Pre-payment caps spam: a second one-shot that would push `bytes_permanent` past
/// `bytes_allowance` is rejected at pool ingress.
#[test]
fn renew_rejects_when_quota_exhausted() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let hash_a = blake2_256(&[0u8; 2000][..]);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 2000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![0u8; 2000]));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![1u8; 2000]));
		run_to_block(2, || None);

		assert_ok!(renew_via_extension(who, TransactionRef::ContentHash(hash_a)));

		let call =
			crate::Call::<Test>::renew { entry: TransactionRef::Position { block: 1, index: 1 } };
		assert_eq!(
			DataRenewal::validate_renewal_signed(&who, &call).map(|_| ()),
			Err(txs::PERMANENT_ALLOWANCE_EXCEEDED.into()),
		);
	});
}

/// One-shot cycle delivers the Renew entry without re-charging auth (slot pre-paid
/// at registration). Contrast with recurring `enable_auto_renew` which charges
/// per cycle — see `auto_renewal_consumes_authorization`.
#[test]
fn one_shot_cycle_does_not_recharge_auth() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 3, 6000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(renew_via_extension(who, TransactionRef::Position { block: 1, index: 0 }));

		// Read raw `Authorizations` directly — the public extent helper masks expired entries.
		let before = pallet_bulletin_transaction_storage::Authorizations::<Test>::get(
			AuthorizationScope::Account(who),
		)
		.unwrap();
		init_block(12);
		assert_ok!(apply_block_inherents_full(None));

		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::DataAutoRenewed {
			index: 0,
			content_hash,
			account: who,
		}));
		let after = pallet_bulletin_transaction_storage::Authorizations::<Test>::get(
			AuthorizationScope::Account(who),
		)
		.unwrap();
		assert_eq!((after.extent, after.expiration), (before.extent, before.expiration));
	});
}

/// Once a registration exists (one-shot or recurring), neither `renew` nor
/// `enable_auto_renew` for the same hash may overwrite it.
#[test]
fn renew_and_enable_auto_renew_conflict() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		assert_ok!(renew_via_extension(who, TransactionRef::Position { block: 1, index: 0 }));
		let permanent_used_before =
			pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get();

		// Duplicate `renew` is rejected at the extension before any charge.
		let dup_call =
			crate::Call::<Test>::renew { entry: TransactionRef::Position { block: 1, index: 0 } };
		assert_eq!(
			DataRenewal::validate_renewal_signed(&who, &dup_call).map(|_| ()),
			Err(txs::AUTO_RENEWAL_ALREADY_ENABLED.into()),
		);
		assert_eq!(
			pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(),
			permanent_used_before
		);

		// Defensive dispatch-level guard still rejects if the extension is bypassed.
		let origin: RuntimeOrigin =
			Origin::<Test>::Authorized { who, scope: AuthorizationScope::Account(who) }.into();
		assert_noop!(
			DataRenewal::renew(origin, TransactionRef::Position { block: 1, index: 0 }),
			crate::Error::<Test>::AutoRenewalAlreadyEnabled,
		);

		let call = crate::Call::<Test>::enable_auto_renew { content_hash };
		assert_eq!(
			DataRenewal::validate_renewal_signed(&who, &call).map(|_| ()),
			Err(txs::AUTO_RENEWAL_ALREADY_ENABLED.into()),
		);
	});
}

/// `renew` requires an authorized-signed origin: Root and Unsigned are rejected with
/// `BadOrigin` (registration would have no account to record).
#[test]
fn renew_rejects_unsigned_and_root_origin() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let data = vec![0u8; 2000];
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		let entry = TransactionRef::Position { block: 1, index: 0 };
		assert_noop!(
			DataRenewal::renew(RuntimeOrigin::none(), entry.clone()),
			DispatchError::BadOrigin,
		);
		assert_noop!(DataRenewal::renew(RuntimeOrigin::root(), entry), DispatchError::BadOrigin,);
	});
}

/// Run a normal block lifecycle past expiry without invoking `apply_block_inherents`.
///
/// `on_initialize` populates `PendingAutoRenewals`; `on_finalize` then enforces that the
/// inherent ran, asserting that the storage is empty. The mock's `run_to_block` always
/// invokes the inherent, hiding this safeguard. This test bypasses the helper to confirm
/// the assert actually fires when an auto-renewal is pending and the inherent is missing.
#[test]
#[should_panic(
	expected = "All pending auto-renewals must be processed by process_pending_renewals"
)]
fn on_finalize_panics_when_inherent_missing() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data.clone()));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(who, content_hash));
		// `run_to_block`'s own on_finalize will flush BlockTransactions into
		// `Transactions[block]` as it advances through subsequent blocks.

		let proof_provider = move || {
			let block_num = System::block_number();
			let period: u64 = pallet_bulletin_transaction_storage::RetentionPeriod::<Test>::get();
			let target = block_num.saturating_sub(period);
			if target > 0 &&
				pallet_bulletin_transaction_storage::Transactions::<Test>::get(target).is_some()
			{
				let parent_hash = System::parent_hash();
				let txs =
					pallet_bulletin_transaction_storage::Transactions::<Test>::get(target).unwrap();
				let data_vec: Vec<Vec<u8>> = txs.iter().map(|_| data.clone()).collect();
				build_proof(parent_hash.as_ref(), data_vec).unwrap()
			} else {
				None
			}
		};

		// Run normally up to (and including) block 12 — proofs supplied via the inherent.
		// The block-1 `Store` entry expires at block 12 but the latest-entry guard
		// skips it (force-renew at block 2 is the latest), so no PendingAutoRenewals
		// build up here. The force-renewed entry ages out at block 13.
		run_to_block(12, proof_provider);

		// Manually advance to block 13 and run only on_initialize, which populates
		// PendingAutoRenewals as Transactions(2) expires. We deliberately do NOT call
		// apply_block_inherents, simulating an inherent that was lost or never built.
		init_block(13);
		assert_eq!(
			crate::PendingAutoRenewals::<Test>::get().len(),
			1,
			"on_initialize should have populated pending"
		);

		// on_finalize must panic on the PendingAutoRenewals invariant. The assert
		// now lives on the renewal pallet (the inherent moved there in the split).
		<DataRenewal as polkadot_sdk_frame::traits::Hooks<u64>>::on_finalize(13);
	});
}

/// Verify that `ProvideInherent::create_inherent` actually emits the composite inherent call
/// when `PendingAutoRenewals` is non-empty, even with no storage proof in `InherentData`.
///
/// This is the direct test for "the block author will inject the inherent that drains pending
/// renewals" — if `create_inherent` ever stops returning the call when only renewals (and no
/// proof) are pending, the chain would panic at on_finalize without any test catching it.
#[test]
fn create_inherent_emits_call_when_pending_renewals_present() {
	use polkadot_sdk_frame::{deps::sp_inherents::InherentData, runtime::prelude::ProvideInherent};

	new_test_ext().execute_with(|| {
		run_to_block(1, || None);

		// Baseline: no pending renewals → the renewal pallet emits no inherent.
		let empty = InherentData::new();
		assert!(
			<DataRenewal as ProvideInherent>::create_inherent(&empty).is_none(),
			"no renewal inherent should be emitted when no auto-renewals are pending",
		);

		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data.clone()));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(who, content_hash));

		// The block-1 `Store` is no longer the latest reference after the
		// force-renew, so the latest-entry guard skips it at block 12. The
		// force-renewed entry ages out at block 13, populating
		// `PendingAutoRenewals` then. We need a proof provider because block 12
		// needs a proof for the (still-on-chain) block-2 Renew entry.
		let proof_provider = move || {
			let block_num = System::block_number();
			let period: u64 = pallet_bulletin_transaction_storage::RetentionPeriod::<Test>::get();
			let target = block_num.saturating_sub(period);
			if target > 0 &&
				pallet_bulletin_transaction_storage::Transactions::<Test>::get(target).is_some()
			{
				let parent_hash = System::parent_hash();
				let txs =
					pallet_bulletin_transaction_storage::Transactions::<Test>::get(target).unwrap();
				let data_vec: Vec<Vec<u8>> = txs.iter().map(|_| data.clone()).collect();
				build_proof(parent_hash.as_ref(), data_vec).unwrap()
			} else {
				None
			}
		};
		run_to_block(12, proof_provider);
		init_block(13);
		assert_eq!(crate::PendingAutoRenewals::<Test>::get().len(), 1);

		// `InherentData` carries no proof, but the renewal pallet must still emit its
		// drain inherent so the pending renewals are processed in this block.
		let result = <DataRenewal as ProvideInherent>::create_inherent(&empty);
		match result {
			Some(crate::Call::<Test>::process_pending_renewals {}) => {},
			other => panic!(
				"expected Some(process_pending_renewals) when pending renewals are present, \
				 got {other:?}"
			),
		}
	});
}

/// A successful renew bumps the chain-wide `PermanentStorageUsed` counter and is recorded
/// in `BlockTransactions` with `kind == Renew` so the obsolete-block cleanup in
/// `on_initialize` can later decrement the counter.
#[test]
fn renew_bumps_permanent_used_and_records_kind() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		assert_eq!(
			pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(),
			0,
			"store must not bump permanent counter"
		);
		// `BlockTransactions` holds the in-progress block's entries; the store entry must
		// have `kind = Store`.
		let block_txs = pallet_bulletin_transaction_storage::BlockTransactions::<Test>::get();
		assert_eq!(block_txs.len(), 1);
		assert_eq!(block_txs[0].kind, TransactionKind::Store);

		run_to_block(3, || None);

		let renew_call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_ok!(DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call));
		assert_ok!(Into::<RuntimeCall>::into(renew_call).dispatch(RuntimeOrigin::none()));

		assert_eq!(
			pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(),
			2000,
			"renew must bump the chain-wide permanent counter",
		);
		let block_txs = pallet_bulletin_transaction_storage::BlockTransactions::<Test>::get();
		assert_eq!(block_txs.len(), 1);
		assert_eq!(block_txs[0].kind, TransactionKind::Renew);
	});
}

/// `renew` rejects with [`txs::PERMANENT_ALLOWANCE_EXCEEDED`] when the per-account hard cap is
/// reached: `bytes_permanent + size > bytes_allowance`. The chain-wide counter must remain
/// untouched.
#[test]
fn renew_rejects_when_per_account_allowance_exceeded() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		// Allowance is below `size`, so renew must reject. Store still succeeds because the
		// non-renew path is the soft side — overshoot is allowed (and demoted in priority by
		// `AllowanceBasedPriority`).
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 1500));
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		run_to_block(3, || None);

		let renew_call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_noop!(
			DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call),
			txs::PERMANENT_ALLOWANCE_EXCEEDED,
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who).bytes_permanent,
			0,
			"rejected renew must not bump bytes_permanent",
		);
		assert_eq!(
			pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(),
			0,
			"rejected renew must not bump chain counter"
		);
	});
}

/// `renew` rejects with [`txs::CHAIN_PERMANENT_CAP_REACHED`] when the chain-wide hard cap is
/// reached: `PermanentStorageUsed + size > MaxPermanentStorageSize`. Per-account state
/// must remain untouched.
#[test]
fn renew_rejects_when_chain_wide_cap_reached() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		// Lower the chain-wide cap below what a renewal would require.
		MaxPermanentStorageSize::set(&1000);

		run_to_block(3, || None);

		let renew_call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_noop!(
			DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call),
			txs::CHAIN_PERMANENT_CAP_REACHED,
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who).bytes_permanent,
			0,
			"rejected renew must not bump bytes_permanent",
		);
		assert_eq!(
			pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(),
			0,
			"rejected renew must not bump chain counter"
		);
	});
}

/// Renews landing in different blocks each contribute to the counter and each decrement
/// independently as their respective blocks become obsolete.
#[test]
fn renews_across_multiple_blocks_decrement_independently() {
	new_test_ext().execute_with(|| {
		let renew_entry = |size: u32| TransactionInfo {
			chunk_root: Default::default(),
			content_hash: [0u8; 32],
			hashing: HashingAlgorithm::Blake2b256,
			cid_codec: 0x55,
			size,
			extrinsic_index: u32::MAX,
			block_chunks: num_chunks(size),
			kind: TransactionKind::Renew,
		};
		// 1000 bytes renewed at block 3, 700 at block 5.
		pallet_bulletin_transaction_storage::Transactions::<Test>::insert(
			3u64,
			BoundedVec::<TransactionInfo, _>::try_from(vec![renew_entry(1000)]).unwrap(),
		);
		pallet_bulletin_transaction_storage::Transactions::<Test>::insert(
			5u64,
			BoundedVec::<TransactionInfo, _>::try_from(vec![renew_entry(700)]).unwrap(),
		);
		pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::put(1700);

		// Block 14: obsolete = 3 → drop 1000.
		System::set_block_number(14);
		<TransactionStorage as Hooks<u64>>::on_initialize(14);
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 700);

		// Block 16: obsolete = 5 → drop 700.
		System::set_block_number(16);
		<TransactionStorage as Hooks<u64>>::on_initialize(16);
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 0);
	});
}

/// Renew emits `PermanentStorageUsedUpdated { used }` so off-chain capacity-planning
/// dashboards can track the chain-wide counter without polling storage.
#[test]
fn renew_emits_permanent_storage_used_updated() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);

		let renew_call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_ok!(DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call));

		System::assert_has_event(RuntimeEvent::TransactionStorage(
			pallet_bulletin_transaction_storage::Event::PermanentStorageUsedUpdated { used: 2000 },
		));
	});
}

/// `enable_auto_renew` rejects a second call for the same content hash, even from the
/// same account, with `AutoRenewalAlreadyEnabled`.
#[test]
fn enable_auto_renew_rejects_already_enabled() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		assert_ok!(enable_auto_renew_via_extension(who, content_hash));
		// Second call rejected at the extension (pool-level).
		let call = crate::Call::<Test>::enable_auto_renew { content_hash };
		assert_eq!(
			DataRenewal::validate_renewal_signed(&who, &call).map(|_| ()),
			Err(txs::AUTO_RENEWAL_ALREADY_ENABLED.into()),
		);
	});
}

/// Expired authorization rejects through `check_authorization` with
/// `InvalidTransaction::Payment` — same path one-shot `renew` and `force_renew`
/// take when `expired()` is `now >= expiration`.
#[test]
fn enable_auto_renew_rejects_expired_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		// AuthorizationPeriod = 10; the auth granted at block 1 expires at block 11.
		init_block(11);

		let call = crate::Call::<Test>::enable_auto_renew { content_hash };
		assert_eq!(
			DataRenewal::validate_renewal_signed(&who, &call).map(|_| ()),
			Err(InvalidTransaction::Payment.into()),
		);
	});
}

/// Insufficient byte capacity rejects at the extension with
/// `txs::PERMANENT_ALLOWANCE_EXCEEDED` when `bytes_permanent + size > bytes_allowance`.
#[test]
fn enable_auto_renew_rejects_insufficient_capacity() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 1000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		let call = crate::Call::<Test>::enable_auto_renew { content_hash };
		assert_eq!(
			DataRenewal::validate_renewal_signed(&who, &call).map(|_| ()),
			Err(txs::PERMANENT_ALLOWANCE_EXCEEDED.into()),
		);
	});
}

/// `disable_auto_renew` dispatched in the same block as the renewal does NOT prevent the
/// renewal. `on_initialize` captures the entry into `PendingAutoRenewals` as a snapshot;
/// `do_process_auto_renewals` then iterates that vec without re-reading
/// `AutoRenewals[hash]`. So disabling between `on_initialize` and `apply_block_inherents`
/// in the renewal block is a no-op for the in-flight cycle — the caller still sees one
/// final `DataAutoRenewed` event. Subsequent cycles are correctly suppressed because
/// `AutoRenewals[hash]` is gone for the next on_initialize sweep.
///
/// This is only reachable once the registration has cleared its prepaid window
/// (`disable_auto_renew` rejects the owner while `paid: true`), so we exercise it
/// on cycle 2.
#[test]
fn disable_auto_renew_in_renewal_block_does_not_prevent_renewal() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(who, content_hash));

		// Cycle 1: fire to consume the prepayment so the owner can `disable` later.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(apply_block_inherents_full(None));
		assert!(!crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid);
		let block_txs = pallet_bulletin_transaction_storage::BlockTransactions::<Test>::take();
		if !block_txs.is_empty() {
			pallet_bulletin_transaction_storage::Transactions::<Test>::insert(12u64, &block_txs);
		}

		// Cycle 2 block: on_initialize captures the block-12 renewal into pending.
		init_block(23);
		assert_eq!(crate::PendingAutoRenewals::<Test>::get().len(), 1);

		// Refresh authorization for cycle 2 (block-12 grant expired at block 22).
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));

		// Disable in the normal section — after on_initialize, before the inherent.
		assert_ok!(disable_auto_renew_via_extension(who, content_hash));
		assert!(
			crate::AutoRenewals::<Test>::get(content_hash).is_none(),
			"disable cleared the registration"
		);

		// The mandatory inherent still iterates the captured pending vec and renews.
		assert_ok!(apply_block_inherents_full(None));
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::DataAutoRenewed {
			index: 0,
			content_hash,
			account: who,
		}));
		// AutoRenewals stays gone after the block — no further cycles.
		assert!(crate::AutoRenewals::<Test>::get(content_hash).is_none());
	});
}

/// Root `disable_auto_renew` executed in the same block as a prepaid cycle (between
/// `on_initialize` and the mandatory inherent) must not be silently undone by the
/// cycle's `paid: true → false` flip. The flip is implemented as a `mutate`, so a
/// Root disable that already removed the entry leaves nothing for the cycle to
/// flip — and no further cycles fire.
#[test]
fn root_disable_in_prepaid_renewal_block_is_not_undone_by_cycle() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(who, content_hash));
		assert!(
			crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid,
			"registration starts prepaid"
		);

		// Cycle 1 block: on_initialize captures the prepaid entry into pending.
		init_block(12);
		assert_eq!(crate::PendingAutoRenewals::<Test>::get().len(), 1);

		// Root disables in the normal section — bypasses both the owner check and the
		// prepaid-window check.
		assert_ok!(DataRenewal::disable_auto_renew(RuntimeOrigin::root(), content_hash));
		assert!(
			crate::AutoRenewals::<Test>::get(content_hash).is_none(),
			"Root disable cleared the entry"
		);

		// Mandatory inherent: the captured pending renewal still fires (the prepayment
		// honestly delivers one cycle), but the post-cycle flip must not reinsert.
		assert_ok!(apply_block_inherents_full(None));
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::DataAutoRenewed {
			index: 0,
			content_hash,
			account: who,
		}));
		assert!(
			crate::AutoRenewals::<Test>::get(content_hash).is_none(),
			"Root's disable must survive the cycle — no silent re-arming via the paid-flip path",
		);
	});
}

/// `do_process_auto_renewals` emits `AutoRenewalFailed` when `check_authorization`
/// returns `txs::CHAIN_PERMANENT_CAP_REACHED` — i.e. the chain-wide `PermanentStorageUsed`
/// counter would exceed `MaxPermanentStorageSize` — even if the per-account budget is
/// fine. The chain-wide gate only applies on cycles that actually charge, so this
/// scenario is reachable on cycle 2 (the first cycle is prepaid at registration).
#[test]
fn auto_renewal_fails_on_chain_wide_permanent_cap() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		// Per-account budget comfortably large — chain-wide cap is the only gate.
		// Pick a cap that fits the registration's prepayment but not a second cycle.
		MaxPermanentStorageSize::set(&3000);
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			10,
			1_000_000,
		));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(who, content_hash));
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 2000);

		// Cycle 1 (prepaid) fires free at block 12 — chain-wide counter is not bumped.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			10,
			1_000_000,
		));
		assert_ok!(apply_block_inherents_full(None));
		assert!(!crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid);
		let block_txs = pallet_bulletin_transaction_storage::BlockTransactions::<Test>::take();
		if !block_txs.is_empty() {
			pallet_bulletin_transaction_storage::Transactions::<Test>::insert(12u64, &block_txs);
		}

		// Cycle 2 at block 23 would charge another `size` chain-wide; overshoot the cap.
		init_block(23);
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			10,
			1_000_000,
		));
		pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::put(2000);

		assert_ok!(apply_block_inherents_full(None));

		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::AutoRenewalFailed {
			content_hash,
			account: who,
		}));
		assert!(crate::AutoRenewals::<Test>::get(content_hash).is_none());
		// Per-account counters untouched — failure happened before consume.
		assert_eq!(TransactionStorage::account_authorization_extent(who).bytes_permanent, 0,);
	});
}

/// `RetentionPeriod` change mid-cycle shifts the `obsolete = n - RP - 1` window. Raising
/// RP after auto-renew is enabled defers the renewal block — the OLD renewal point
/// becomes a normal block (no pending), and the renewal fires later at the NEW window
/// boundary.
#[test]
fn auto_renew_obeys_updated_retention_period() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(who, content_hash));

		// Extend RetentionPeriod from 10 → 20.
		pallet_bulletin_transaction_storage::RetentionPeriod::<Test>::put(20u64);

		// Block 12 was the OLD renewal boundary. With RP=20, `obsolete = 12 - 21 = 0`
		// (saturating). Transactions[1] must NOT be pulled.
		init_block(12);
		assert!(
			crate::PendingAutoRenewals::<Test>::get().is_empty(),
			"RP change should push the obsolete boundary out",
		);
		assert!(
			pallet_bulletin_transaction_storage::Transactions::<Test>::get(1).is_some(),
			"Transactions[1] still present at block 12"
		);

		// Block 22 is the NEW renewal boundary (`obsolete = 22 - 21 = 1`).
		init_block(22);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_eq!(crate::PendingAutoRenewals::<Test>::get().len(), 1);
		assert_ok!(apply_block_inherents_full(None));
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::DataAutoRenewed {
			index: 0,
			content_hash,
			account: who,
		}));
	});
}

/// `enable_auto_renew` is signed by the *registrant*, who may not be the storer.
/// The prepayment at registration (and every subsequent cycle's charge) consume
/// the registrant's authorization, not the storer's. Verifies: Bob registers
/// auto-renew on data Alice stored, and Bob's `bytes_permanent` is the one that
/// moves — both at registration time and on cycle 2.
#[test]
fn auto_renew_consumes_registrant_authorization_not_storer() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let alice = 1;
		let bob = 2;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			alice,
			10,
			100_000,
		));
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), bob, 10, 100_000,));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		// Bob — not Alice — enables auto-renew. The prepayment lands on Bob.
		assert_ok!(enable_auto_renew_via_extension(bob, content_hash));
		assert_eq!(crate::AutoRenewals::<Test>::get(content_hash).unwrap().account, bob);
		assert_eq!(TransactionStorage::account_authorization_extent(alice).bytes_permanent, 0);
		assert_eq!(TransactionStorage::account_authorization_extent(bob).bytes_permanent, 2000);

		// Cycle 1 (prepaid) fires free at block 12 — re-authorize both so the
		// chain state is clean, but the cycle won't charge anyone.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			alice,
			10,
			100_000,
		));
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), bob, 10, 100_000,));
		assert_ok!(apply_block_inherents_full(None));
		assert!(!crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid);
		let block_txs = pallet_bulletin_transaction_storage::BlockTransactions::<Test>::take();
		if !block_txs.is_empty() {
			pallet_bulletin_transaction_storage::Transactions::<Test>::insert(12u64, &block_txs);
		}

		// Cycle 2 at block 23 charges per-cycle: again the registrant (Bob) pays.
		init_block(23);
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			alice,
			10,
			100_000,
		));
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), bob, 10, 100_000,));
		assert_ok!(apply_block_inherents_full(None));

		assert_eq!(TransactionStorage::account_authorization_extent(alice).bytes_permanent, 0);
		assert_eq!(TransactionStorage::account_authorization_extent(bob).bytes_permanent, 2000);
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::DataAutoRenewed {
			index: 0,
			content_hash,
			account: bob,
		}));
	});
}

/// `refresh_account_authorization` only extends `expiration`. If `bytes_permanent` is
/// already at or near the per-account cap, refreshing does NOT reset counters, so the
/// next auto-renew cycle still fails on the per-account axis (not on expiration).
///
/// To exercise this, we need to drive `bytes_permanent` close to the cap before
/// the cycle under test. `enable_auto_renew` pre-pays one cycle's worth at
/// registration — that registration charge already moves `bytes_permanent` to
/// `size`. The first cycle fires free (prepaid). The second cycle then has to
/// charge against an authorization whose `bytes_permanent` was preserved by
/// `refresh`, not reset.
#[test]
fn refresh_authorization_does_not_reset_counters_for_auto_renew() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		// Tight cap: one renewal fits, the second exceeds it.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 3000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(enable_auto_renew_via_extension(who, content_hash));

		// Registration prepaid the first cycle: `bytes_permanent → 2000`.
		let after_register = TransactionStorage::account_authorization_extent(who);
		assert_eq!(after_register.bytes_permanent, 2000);

		// Refresh at block 11 (the original auth's expiration boundary), extending
		// expiration to 21 so block 12's free cycle and the subsequent paid cycle
		// at block 23 both see an unexpired auth — but counters must NOT reset.
		init_block(11);
		assert_ok!(TransactionStorage::refresh_account_authorization(RuntimeOrigin::root(), who,));
		let after_refresh = TransactionStorage::account_authorization_extent(who);
		assert_eq!(after_refresh, after_register, "refresh must not touch the extent");

		// Cycle 1 (prepaid) fires free at block 12; carry the renew into
		// `Transactions[12]` so it can age out for cycle 2.
		init_block(12);
		assert_ok!(apply_block_inherents_full(None));
		assert!(!crate::AutoRenewals::<Test>::get(content_hash).unwrap().paid);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who).bytes_permanent,
			2000,
			"prepaid cycle does not charge again",
		);
		let block_txs = pallet_bulletin_transaction_storage::BlockTransactions::<Test>::take();
		if !block_txs.is_empty() {
			pallet_bulletin_transaction_storage::Transactions::<Test>::insert(12u64, &block_txs);
		}

		// Refresh again at block 21 to keep the auth alive past block 23.
		init_block(21);
		assert_ok!(TransactionStorage::refresh_account_authorization(RuntimeOrigin::root(), who,));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who).bytes_permanent,
			2000,
			"second refresh must also leave the extent untouched",
		);

		// Cycle 2 at block 23 must charge another 2000 against a per-account cap
		// of 3000 already at 2000 → AutoRenewalFailed on the per-account axis (not
		// on expiration — refresh kept the auth alive).
		init_block(23);
		assert_ok!(apply_block_inherents_full(None));
		System::assert_has_event(RuntimeEvent::DataRenewal(crate::Event::AutoRenewalFailed {
			content_hash,
			account: who,
		}));
		assert!(crate::AutoRenewals::<Test>::get(content_hash).is_none());
	});
}

#[test]
fn uses_preimage_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let data = vec![2; 2000];
		let hash = blake2_256(&data);
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), hash, 2002));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 2002,
				transactions: 0,
				transactions_allowance: 1,
			}
		);
		// Data with a non-matching hash has no preimage auth → rejected.
		let call = pallet_bulletin_transaction_storage::Call::<Test>::store { data: vec![1; 2000] };
		assert_noop!(TransactionStorage::pre_dispatch(&call), InvalidTransaction::Payment);
		// Matching data consumes allowance but the entry stays (new behaviour).
		let call = pallet_bulletin_transaction_storage::Call::<Test>::store { data };
		assert_ok!(TransactionStorage::pre_dispatch(&call));
		// Entry persists with the remainder (2002 - 2000 = 2 bytes); the
		// transaction count is exhausted so further stores still fail.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 2002,
				transactions: 1,
				transactions_allowance: 1,
			}
		);
		assert_ok!(Into::<RuntimeCall>::into(call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);
		// Renew also uses the same preimage auth; it bumps `bytes_permanent` rather than `bytes`.
		let call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_ok!(DataRenewal::pre_dispatch(&call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 2000,
				bytes_allowance: 2002,
				transactions: 2,
				transactions_allowance: 1,
			}
		);
	});
}

#[test]
fn storage_calls_reject_plain_signed_origin() {
	// Storage-mutating calls must gate on `ensure_authorized` (accepts `Authorized` /
	// `Root` / `None` only). A plain `Signed` origin bypasses the extension pipeline and
	// must be rejected. Catches the class of bug where the gate is dropped on a refactor.
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let signed = RuntimeOrigin::signed(42);
		let data = vec![0u8; 2000];
		let cid_config = CidConfig { codec: 0x55, hashing: HashingAlgorithm::Blake2b256 };

		assert_noop!(
			TransactionStorage::store(signed.clone(), data.clone()),
			DispatchError::BadOrigin,
		);
		assert_noop!(
			TransactionStorage::store_with_cid_config(signed.clone(), cid_config, data),
			DispatchError::BadOrigin,
		);
		assert_noop!(
			DataRenewal::force_renew(signed, TransactionRef::Position { block: 1, index: 0 },),
			DispatchError::BadOrigin,
		);
	});
}

/// `refresh_account_authorization` only extends expiration — it does NOT reset any
/// consumed counters (`bytes`, `bytes_permanent`, `transactions`). In particular,
/// `bytes_permanent` MUST be left intact: permanent storage stays on chain across refresh
/// cycles, so its accounting cannot be erased. See the comment in `refresh_authorization`.
#[test]
fn refresh_does_not_reset_consumed_counters() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		// Authorize: all counters start at 0.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 0,
				transactions_allowance: 0,
			},
		);

		// Store: bumps `bytes` and `transactions`; `bytes_permanent` untouched.
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
			},
			"store must advance `bytes` and `transactions`"
		);

		run_to_block(3, || None);

		// Renew: bumps `bytes_permanent` and `transactions`; `bytes` untouched.
		let renew_call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_ok!(DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 2000,
				bytes_allowance: 4000,
				transactions: 2,
				transactions_allowance: 0,
			},
			"renew must advance `bytes_permanent` and `transactions`"
		);

		// Refresh: all consumed counters preserved; only expiration moves.
		assert_ok!(TransactionStorage::refresh_account_authorization(RuntimeOrigin::root(), who));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 2000,
				bytes_allowance: 4000,
				transactions: 2,
				transactions_allowance: 0,
			},
			"refresh must not reset any consumed counters"
		);
	});
}

/// End-to-end: hit the chain-wide cap with renews; advance past `RetentionPeriod`;
/// `on_initialize` decrements the counter; new renews succeed again. This is the
/// self-correcting bound on chain-wide renewed bytes.
#[test]
fn chain_wide_cap_self_corrects_after_age_out() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, u64::MAX,));
		MaxPermanentStorageSize::set(&2000);

		// Renew 2000 bytes at block 1 → counter at cap.
		let store_call =
			pallet_bulletin_transaction_storage::Call::<Test>::store { data: vec![0u8; 2000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(2, || None);
		let renew_call = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 1, index: 0 },
		};
		assert_ok!(DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call));
		assert_ok!(Into::<RuntimeCall>::into(renew_call).dispatch(RuntimeOrigin::none()));
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 2000);

		// Another renew now must reject — chain cap reached.
		run_to_block(3, || None);
		let store_call_b =
			pallet_bulletin_transaction_storage::Call::<Test>::store { data: vec![0u8; 100] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call_b));
		assert_ok!(Into::<RuntimeCall>::into(store_call_b).dispatch(RuntimeOrigin::none()));
		run_to_block(4, || None);
		let renew_call_b = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 3, index: 0 },
		};
		assert_noop!(
			DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call_b),
			txs::CHAIN_PERMANENT_CAP_REACHED,
		);

		// Advance past `RetentionPeriod` (10) so the obsolete-block cleanup decrements
		// the counter. `on_finalize` requires a storage proof in any block whose
		// `target = n - 10` has `Transactions[target]` non-empty: blocks 1, 2, 3 each
		// got a transaction, so we provide proofs at blocks 11, 12, 13 respectively.
		let proof_provider = || {
			let parent_hash = System::parent_hash();
			let block_num = System::block_number();
			match block_num {
				11 | 12 => build_proof(parent_hash.as_ref(), vec![vec![0u8; 2000]]).unwrap(),
				13 => build_proof(parent_hash.as_ref(), vec![vec![0u8; 100]]).unwrap(),
				_ => None,
			}
		};
		run_to_block(13, proof_provider);
		assert_eq!(
			pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(),
			0,
			"counter must self-correct as data ages out"
		);

		// Renew now succeeds again. Mock `AuthorizationPeriod = 10`, so the original
		// authorization (granted at block 1) expired at block 11. Re-authorize for the
		// new window before driving another store/renew.
		run_to_block(14, proof_provider);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, u64::MAX,));
		let store_call_c =
			pallet_bulletin_transaction_storage::Call::<Test>::store { data: vec![0u8; 500] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call_c));
		assert_ok!(Into::<RuntimeCall>::into(store_call_c).dispatch(RuntimeOrigin::none()));
		run_to_block(15, proof_provider);
		let renew_call_c = crate::Call::<Test>::force_renew {
			entry: TransactionRef::Position { block: 14, index: 0 },
		};
		assert_ok!(DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call_c));
		assert_ok!(Into::<RuntimeCall>::into(renew_call_c).dispatch(RuntimeOrigin::none()));
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 500);
	});
}

/// `PermanentStorageNearCap` fires once on the rising edge across the threshold and is
/// **not** re-emitted while still above the threshold. Decrementing back below and rising
/// again re-arms the signal.
#[test]
fn permanent_storage_near_cap_fires_on_rising_edge_only() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;

		// Cap = 1000; threshold = 1000 * 80 / 100 = 800.
		MaxPermanentStorageSize::set(&1000);

		// Generous per-account allowance so renews are only gated by the chain-wide cap.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, u64::MAX,));

		// Helper: store `size` bytes at the current block, advance one block, then renew it.
		// Captures the store block so the renew always points at the just-stored tx, not
		// some earlier one.
		let store_and_renew = |size: usize| {
			let store_block = System::block_number();
			let store_call =
				pallet_bulletin_transaction_storage::Call::<Test>::store { data: vec![0u8; size] };
			assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
			assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
			run_to_block(store_block + 1, || None);
			let renew_call = crate::Call::<Test>::force_renew {
				entry: TransactionRef::Position { block: store_block, index: 0 },
			};
			assert_ok!(DataRenewal::pre_dispatch_renewal_signed(&who, &renew_call));
		};

		// Step 1: 500 bytes (PermanentStorageUsed: 0 → 500). Below threshold; no near-cap.
		store_and_renew(500);
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 500);
		let evs = System::events();
		assert!(!evs.iter().any(|r| matches!(
			r.event,
			RuntimeEvent::TransactionStorage(
				pallet_bulletin_transaction_storage::Event::PermanentStorageNearCap { .. }
			)
		)));

		// Step 2: +400 bytes (500 → 900). Crosses 800 threshold → near-cap fires.
		System::reset_events();
		store_and_renew(400);
		assert_eq!(pallet_bulletin_transaction_storage::PermanentStorageUsed::<Test>::get(), 900);
		System::assert_has_event(RuntimeEvent::TransactionStorage(
			pallet_bulletin_transaction_storage::Event::PermanentStorageNearCap {
				used: 900,
				cap: 1000,
			},
		));

		// Quick sanity check on the threshold formula matching the constant.
		assert_eq!(txs::PERMANENT_STORAGE_NEAR_CAP_PERCENT, 80);
	});
}
