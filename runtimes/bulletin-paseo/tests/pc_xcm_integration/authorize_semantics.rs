// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Cumulus.
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

//! `authorize_account` semantics: happy path, additivity, replacement on
//! expiry, and per-account scoping.

use super::*;

#[test]
fn happy_path_from_sibling() {
	new_test_ext().execute_with(|| {
		let who: AccountId = Sr25519Keyring::Alice.to_account_id();

		assert_ok!(pc_authorize(who.clone(), 10, 1_000_000));

		assert_eq!(extent_of(&who), extent(0, 1_000_000, 0, 10));
		assert!(TransactionStorage::account_has_active_authorization(&who));
	});
}

// Authorizations are additive within an unexpired window.
// Each people chain claim adds to the existing allowance
// and does NOT push expiry forward.
// Consumed counters are preserved.
#[test]
fn additive_within_window() {
	new_test_ext().execute_with(|| {
		let account = Sr25519Keyring::Alice;
		let who: AccountId = account.to_account_id();

		assert_ok!(pc_authorize(who.clone(), 5, 1_000));
		assert_eq!(extent_of(&who), extent(0, 1_000, 0, 5));

		advance_block();
		assert_extrinsic_ok(construct_and_apply_extrinsic(
			Some(account.pair()),
			RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
				data: vec![0u8; 200],
			}),
		));
		assert_eq!(extent_of(&who), extent(200, 1_000, 1, 5));

		assert_ok!(pc_authorize(who.clone(), 3, 500));
		assert_eq!(extent_of(&who), extent(200, 1_500, 1, 8));

		assert_ok!(pc_authorize(who.clone(), 2, 250));
		assert_eq!(extent_of(&who), extent(200, 1_750, 1, 10));

		// Expiry must not have been pushed forward by additive claims.
		let now = System::block_number();
		System::set_block_number(now + auth_period());
		assert_eq!(extent_of(&who), empty(), "additive claims must not have extended expiry",);
	});
}

// Replace after expiry.
#[test]
fn replaces_after_expiry() {
	new_test_ext().execute_with(|| {
		let account = Sr25519Keyring::Alice;
		let who: AccountId = account.to_account_id();

		assert_ok!(pc_authorize(who.clone(), 5, 1_000));

		advance_block();
		assert_extrinsic_ok(construct_and_apply_extrinsic(
			Some(account.pair()),
			RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
				data: vec![0u8; 400],
			}),
		));
		assert_eq!(extent_of(&who), extent(400, 1_000, 1, 5));

		let now = System::block_number();
		System::set_block_number(now + auth_period() + 1);
		assert_eq!(extent_of(&who), empty(), "extent must read empty once expired");

		assert_ok!(pc_authorize(who.clone(), 1, 100));
		assert_eq!(extent_of(&who), extent(0, 100, 0, 1));
	});
}

#[test]
fn account_scopes_are_independent() {
	new_test_ext().execute_with(|| {
		let alice: AccountId = Sr25519Keyring::Alice.to_account_id();
		let bob: AccountId = Sr25519Keyring::Bob.to_account_id();

		assert_ok!(pc_authorize(alice.clone(), 5, 1_000));
		assert_ok!(pc_authorize(bob.clone(), 10, 2_000));

		assert_eq!(extent_of(&alice), extent(0, 1_000, 0, 5));
		assert_eq!(extent_of(&bob), extent(0, 2_000, 0, 10));

		let now = System::block_number();
		System::set_block_number(now + auth_period() + 1);
		assert_ok!(TransactionStorage::remove_expired_account_authorization(
			RuntimeOrigin::none(),
			alice.clone(),
		));
		assert_eq!(extent_of(&alice), empty());
		// Bob's entry is still in storage — re-authorize lands as a
		// fresh entry rather than failing.
		assert_ok!(pc_authorize(bob.clone(), 1, 50));
		assert_eq!(extent_of(&bob), extent(0, 50, 0, 1));
	});
}
