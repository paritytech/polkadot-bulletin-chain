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

//! Tests for `pallet-storage-auto-renewal`.

use super::{
	mock::{
		new_test_ext, AutoRenewal, RuntimeEvent, RuntimeOrigin, System, Test, TransactionStorage,
	},
	Event, TransactionInfo,
};
use bulletin_transaction_storage_primitives::cids::HashingAlgorithm;
use pallet_bulletin_transaction_storage::{
	AuthorizationExtent, BlockTransactions, TransactionKind, Transactions,
};
use polkadot_sdk_frame::{
	deps::frame_support::storage::unhashed, hashing::blake2_256, prelude::*, testing_prelude::*,
};

type Error = super::Error<Test>;
type AutoRenewals = super::AutoRenewals<Test>;
type PendingAutoRenewals = super::PendingAutoRenewals<Test>;
type TransactionByContentHash = pallet_bulletin_transaction_storage::TransactionByContentHash<Test>;

/// Run System + TransactionStorage + AutoRenewal hooks up to block `n`.
fn run_to_block(n: u64) {
	while System::block_number() < n {
		<AutoRenewal as polkadot_sdk_frame::traits::Hooks<u64>>::on_finalize(System::block_number());
		<TransactionStorage as polkadot_sdk_frame::traits::Hooks<u64>>::on_finalize(
			System::block_number(),
		);
		System::set_block_number(System::block_number() + 1);
		System::reset_events();
		unhashed::put::<u32>(b":extrinsic_index", &0);
		<TransactionStorage as polkadot_sdk_frame::traits::Hooks<u64>>::on_initialize(
			System::block_number(),
		);
		<AutoRenewal as polkadot_sdk_frame::traits::Hooks<u64>>::on_initialize(
			System::block_number(),
		);
	}
}

/// Initialize block N for tests that manually exercise on_initialize without running
/// `on_finalize` (which would panic on a non-empty PendingAutoRenewals queue).
fn init_block(n: u64) {
	System::set_block_number(n);
	System::reset_events();
	unhashed::put::<u32>(b":extrinsic_index", &0);
	<TransactionStorage as polkadot_sdk_frame::traits::Hooks<u64>>::on_initialize(n);
	<AutoRenewal as polkadot_sdk_frame::traits::Hooks<u64>>::on_initialize(n);
}

#[test]
fn enable_auto_renew_works() {
	new_test_ext().execute_with(|| {
		run_to_block(1);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2);

		assert_ok!(AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(who), content_hash));

		let renewal_data = AutoRenewals::get(content_hash).unwrap();
		assert_eq!(renewal_data.account, who);

		System::assert_has_event(RuntimeEvent::AutoRenewal(Event::AutoRenewalEnabled {
			content_hash,
			who,
		}));

		assert_noop!(
			AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(who), content_hash),
			Error::AutoRenewalAlreadyEnabled,
		);
	});
}

#[test]
fn enable_auto_renew_rejects_invalid() {
	new_test_ext().execute_with(|| {
		run_to_block(1);
		let who = 1;

		let bogus_hash = blake2_256(&[99u8; 100]);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_noop!(
			AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(who), bogus_hash),
			Error::ContentNotFound,
		);

		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2);

		let unauthorized_user = 99;
		assert_noop!(
			AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(unauthorized_user), content_hash,),
			Error::InsufficientAuthorization,
		);
	});
}

#[test]
fn disable_auto_renew_works() {
	new_test_ext().execute_with(|| {
		run_to_block(1);
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
		run_to_block(2);
		assert_ok!(AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(owner), content_hash));

		assert_noop!(
			AutoRenewal::disable_auto_renew(RuntimeOrigin::signed(other), content_hash),
			Error::NotAutoRenewalOwner,
		);

		assert_ok!(AutoRenewal::disable_auto_renew(RuntimeOrigin::signed(owner), content_hash));
		assert!(AutoRenewals::get(content_hash).is_none());
		System::assert_has_event(RuntimeEvent::AutoRenewal(Event::AutoRenewalDisabled {
			content_hash,
			who: owner,
		}));
	});
}

#[test]
fn disable_auto_renew_fails_if_not_enabled() {
	new_test_ext().execute_with(|| {
		run_to_block(1);
		let who = 1;
		let content_hash = blake2_256(&[99u8; 100]);

		assert_noop!(
			AutoRenewal::disable_auto_renew(RuntimeOrigin::signed(who), content_hash),
			Error::AutoRenewalNotEnabled,
		);
	});
}

#[test]
fn auto_renewal_lifecycle() {
	new_test_ext().execute_with(|| {
		run_to_block(1);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2);
		assert_ok!(AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(who), content_hash));

		assert_eq!(TransactionByContentHash::get(content_hash), Some((1, 0)));
		assert!(Transactions::<Test>::get(1).is_some());

		// Block 12: on_initialize takes Transactions(1) and emits OnTransactionExpiring,
		// which the AutoRenewal pallet uses to populate PendingAutoRenewals.
		init_block(12);

		let pending = PendingAutoRenewals::get();
		assert_eq!(pending.len(), 1);
		assert_eq!(pending[0].0, content_hash);

		// Refresh authorization (it expired at block 11) so the renewal succeeds.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(AutoRenewal::process_auto_renewals(RuntimeOrigin::none()));

		assert!(PendingAutoRenewals::get().is_empty());
		assert_eq!(TransactionByContentHash::get(content_hash), Some((12, 0)));
		System::assert_has_event(RuntimeEvent::AutoRenewal(Event::DataAutoRenewed {
			index: 0,
			content_hash,
			account: who,
		}));
		assert!(Transactions::<Test>::get(1).is_none());
		assert!(AutoRenewals::get(content_hash).is_some());
	});
}

#[test]
fn auto_renewal_consumes_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 3, 6000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2);
		assert_ok!(AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(who), content_hash));

		let initial = TransactionStorage::account_authorization_extent(who);
		assert_eq!(
			initial,
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 6000,
				transactions: 0,
				transactions_allowance: 3,
			},
		);

		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 3, 6000));
		assert_ok!(AutoRenewal::process_auto_renewals(RuntimeOrigin::none()));

		// Renewals consume `bytes_permanent` (not `bytes`) on success.
		let after = TransactionStorage::account_authorization_extent(who);
		assert_eq!(
			after,
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
		run_to_block(1);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 5, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2);
		assert_ok!(AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(who), content_hash));

		// First renewal at block 12 — refresh with exactly 1 op worth of authorization.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 2000));
		assert_ok!(AutoRenewal::process_auto_renewals(RuntimeOrigin::none()));

		// Permanent axis fully consumed.
		let extent = TransactionStorage::account_authorization_extent(who);
		assert_eq!(
			extent,
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 2000,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1,
			},
		);
		assert_eq!(TransactionByContentHash::get(content_hash), Some((12, 0)));

		// Move BlockTransactions → Transactions(12) so on_initialize finds it later.
		let block_txs = BlockTransactions::<Test>::take();
		if !block_txs.is_empty() {
			Transactions::<Test>::insert(12u64, &block_txs);
		}

		// Second renewal at block 23 (12 + 10 + 1) — should fail (no auth).
		init_block(23);
		let pending = PendingAutoRenewals::get();
		assert_eq!(pending.len(), 1, "Should have pending renewal");

		assert_ok!(AutoRenewal::process_auto_renewals(RuntimeOrigin::none()));

		System::assert_has_event(RuntimeEvent::AutoRenewal(Event::AutoRenewalFailed {
			content_hash,
			account: who,
		}));
		assert!(AutoRenewals::get(content_hash).is_none(), "Auto-renewal should be removed");
	});
}

#[test]
fn process_auto_renewals_rejects_signed_origin() {
	new_test_ext().execute_with(|| {
		run_to_block(1);
		assert_noop!(
			AutoRenewal::process_auto_renewals(RuntimeOrigin::signed(1)),
			DispatchError::BadOrigin,
		);
	});
}

#[test]
fn process_auto_renewals_noop_when_empty() {
	new_test_ext().execute_with(|| {
		run_to_block(1);
		assert_ok!(AutoRenewal::process_auto_renewals(RuntimeOrigin::none()));
		assert!(PendingAutoRenewals::get().is_empty());
	});
}

#[test]
fn pending_auto_renewals_populated_only_for_registered_items() {
	new_test_ext().execute_with(|| {
		run_to_block(1);
		let who = 1;
		let data1 = vec![0u8; 2000];
		let data2 = vec![1u8; 2000];
		let hash1 = blake2_256(&data1);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data1));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data2));
		run_to_block(2);

		// Only enable auto-renew for hash1.
		assert_ok!(AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(who), hash1));

		init_block(12);

		let pending = PendingAutoRenewals::get();
		assert_eq!(pending.len(), 1, "Only hash1 should be pending");
		assert_eq!(pending[0].0, hash1);
	});
}

#[test]
fn auto_renew_permissionless_transfer() {
	// Alice enables, then disables. Bob enables instead. Anyone can keep data alive.
	new_test_ext().execute_with(|| {
		run_to_block(1);
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
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2);

		assert_ok!(AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(alice), content_hash));
		assert_eq!(AutoRenewals::get(content_hash).unwrap().account, alice);

		assert_ok!(AutoRenewal::disable_auto_renew(RuntimeOrigin::signed(alice), content_hash));
		assert!(AutoRenewals::get(content_hash).is_none());

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), bob, 10, 100_000));
		assert_ok!(AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(bob), content_hash));

		assert_eq!(AutoRenewals::get(content_hash).unwrap().account, bob);

		System::assert_has_event(RuntimeEvent::AutoRenewal(Event::AutoRenewalEnabled {
			content_hash,
			who: bob,
		}));
	});
}

#[test]
fn process_auto_renewals_continues_on_per_item_failure() {
	// Verify that if one renewal fails (e.g. block full), remaining items still process.
	new_test_ext().execute_with(|| {
		run_to_block(1);
		let who = 1;

		let max_txns =
			<<Test as pallet_bulletin_transaction_storage::Config>::MaxBlockTransactions as Get<
				u32,
			>>::get();
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			max_txns + 10,
			100_000_000,
		));

		let mut hashes = Vec::new();
		for i in 0..3u8 {
			let data = vec![i; 2000];
			let content_hash = blake2_256(&data);
			hashes.push(content_hash);
			assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		}
		run_to_block(2);

		for hash in &hashes {
			assert_ok!(AutoRenewal::enable_auto_renew(RuntimeOrigin::signed(who), *hash));
		}

		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			max_txns + 10,
			100_000_000,
		));

		let pending = PendingAutoRenewals::get();
		assert_eq!(pending.len(), 3);

		// Fill BlockTransactions with (max - 1) dummy entries so only 1 renewal fits.
		let dummy = TransactionInfo {
			chunk_root: Default::default(),
			size: 100,
			content_hash: [0u8; 32],
			hashing: HashingAlgorithm::Blake2b256,
			cid_codec: 0x55,
			extrinsic_index: 0,
			block_chunks: 0,
			kind: TransactionKind::Store,
		};
		BlockTransactions::<Test>::mutate(|txns| {
			for _ in 0..(max_txns - 1) {
				let _ = txns.try_push(dummy.clone());
			}
		});

		assert_ok!(AutoRenewal::process_auto_renewals(RuntimeOrigin::none()));

		assert!(PendingAutoRenewals::get().is_empty());

		// First should have succeeded.
		System::assert_has_event(RuntimeEvent::AutoRenewal(Event::DataAutoRenewed {
			index: max_txns - 1,
			content_hash: hashes[0],
			account: who,
		}));
		// Remaining should have failed (block full → AutoRenewalFailed).
		System::assert_has_event(RuntimeEvent::AutoRenewal(Event::AutoRenewalFailed {
			content_hash: hashes[1],
			account: who,
		}));
		System::assert_has_event(RuntimeEvent::AutoRenewal(Event::AutoRenewalFailed {
			content_hash: hashes[2],
			account: who,
		}));
		assert!(AutoRenewals::get(hashes[1]).is_none());
		assert!(AutoRenewals::get(hashes[2]).is_none());
	});
}
