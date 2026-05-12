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

//! `SafeCallFilter`.
//!
//! Storage-mutating calls must not reach dispatch over XCM, even when the
//! origin is otherwise valid. The filter inspects through `Utility::batch*`.

use super::*;

#[test]
fn sibling_xcm_store_is_blocked() {
	new_test_ext().execute_with(|| {
		let who: AccountId = Sr25519Keyring::Alice.to_account_id();
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who.clone(),
			1,
			1_000
		));

		let store_call = RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
			data: vec![0u8; 100],
		});

		assert!(execute_from(pc_location(), xcm_transact(store_call, OriginKind::Xcm)).is_err());
		assert_eq!(extent_of(&who), extent(0, 1_000, 0, 1));
	});
}

#[test]
fn sibling_xcm_batch_with_store_is_entirely_blocked() {
	new_test_ext().execute_with(|| {
		let target: AccountId = Sr25519Keyring::Bob.to_account_id();
		let store_target: AccountId = Sr25519Keyring::Alice.to_account_id();

		let authorize_call =
			RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
				who: target.clone(),
				transactions: 5,
				bytes: 1_000,
			});
		let store_call = RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
			data: vec![0u8; 50],
		});
		let batch_call = RuntimeCall::Utility(pallet_utility::Call::batch {
			calls: vec![authorize_call, store_call],
		});

		assert!(execute_from(pc_location(), xcm_transact(batch_call, OriginKind::Xcm)).is_err());
		assert_eq!(
			extent_of(&target),
			empty(),
			"the inner authorize_account must NOT have executed alongside a filtered store",
		);
		assert_eq!(extent_of(&store_target), empty());
	});
}

#[test]
fn sibling_xcm_batch_of_only_authorize_calls_succeeds() {
	new_test_ext().execute_with(|| {
		let alice: AccountId = Sr25519Keyring::Alice.to_account_id();
		let bob: AccountId = Sr25519Keyring::Bob.to_account_id();

		let authorize_alice =
			RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
				who: alice.clone(),
				transactions: 5,
				bytes: 1_000,
			});
		let authorize_bob =
			RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
				who: bob.clone(),
				transactions: 10,
				bytes: 2_000,
			});
		let batch_call = RuntimeCall::Utility(pallet_utility::Call::batch {
			calls: vec![authorize_alice, authorize_bob],
		});

		assert_ok!(execute_from(pc_location(), xcm_transact(batch_call, OriginKind::Xcm)));
		assert_eq!(extent_of(&alice), extent(0, 1_000, 0, 5));
		assert_eq!(extent_of(&bob), extent(0, 2_000, 0, 10));
	});
}
