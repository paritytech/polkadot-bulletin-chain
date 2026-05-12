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

//! `refresh_account_authorization` semantics.

use super::*;

#[test]
fn extends_only_expiration() {
	new_test_ext().execute_with(|| {
		let who: AccountId = Sr25519Keyring::Alice.to_account_id();

		assert_ok!(pc_authorize(who.clone(), 5, 1_000));
		assert_eq!(extent_of(&who), extent(0, 1_000, 0, 5));

		let half = auth_period() / 2;
		let now = System::block_number();
		System::set_block_number(now + half);

		assert_ok!(pc_refresh(who.clone()));

		assert_eq!(extent_of(&who), extent(0, 1_000, 0, 5));

		// A block past the *original* expiry must still be active because
		// refresh extended the window.
		System::set_block_number(now + auth_period());
		assert!(
			TransactionStorage::account_has_active_authorization(&who),
			"refresh must have extended expiry past the original window",
		);
	});
}

#[test]
fn without_prior_authorize_fails() {
	new_test_ext().execute_with(|| {
		let who: AccountId = Sr25519Keyring::Alice.to_account_id();

		// XCM completes; the inner `refresh_account_authorization` returns
		// `Error::AccountNotAuthorized` which is reported via runtime events,
		// not as an XCM-level instruction error.
		assert_ok!(pc_refresh(who.clone()));

		assert_eq!(extent_of(&who), empty());
		assert!(!TransactionStorage::account_has_active_authorization(&who));
	});
}
