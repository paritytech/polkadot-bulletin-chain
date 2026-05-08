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

//! Origin and barrier rejections.
//!
//! XCM `Transact` reports a successful `Outcome` even when the inner call's
//! dispatch fails with `BadOrigin` or returns a runtime error. The signal
//! that the rejection happened is therefore the absence of any storage
//! mutation, which is what these tests assert.

use super::*;

#[test]
fn relay_chain_origin_cannot_authorize() {
	new_test_ext().execute_with(|| {
		let target: AccountId = Sr25519Keyring::Ferdie.to_account_id();
		let call = RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
			who: target.clone(),
			transactions: 5,
			bytes: 1_000,
		});

		assert_ok!(execute_from(Location::parent(), xcm_transact(call, OriginKind::Xcm)));

		assert_eq!(extent_of(&target), empty(), "relay-chain origin must not authorize");
	});
}

#[test]
fn sibling_with_sovereign_origin_kind_cannot_authorize() {
	new_test_ext().execute_with(|| {
		let target: AccountId = Sr25519Keyring::Ferdie.to_account_id();
		let call = RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
			who: target.clone(),
			transactions: 5,
			bytes: 1_000,
		});

		assert_ok!(execute_from(pc_location(), xcm_transact(call, OriginKind::SovereignAccount)));

		assert_eq!(
			extent_of(&target),
			empty(),
			"sibling with OriginKind::SovereignAccount must not authorize",
		);

		let sovereign = LocationToAccountId::convert_location(&pc_location())
			.expect("sibling sovereign account must derive");
		assert_eq!(
			extent_of(&sovereign),
			empty(),
			"derived sibling sovereign must not gain authorization",
		);
	});
}

#[test]
fn random_local_origin_cannot_authorize() {
	new_test_ext().execute_with(|| {
		let target: AccountId = Sr25519Keyring::Ferdie.to_account_id();
		let stranger_loc =
			Location::new(0, [Junction::AccountId32 { network: None, id: [0x42u8; 32] }]);
		let call = RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
			who: target.clone(),
			transactions: 5,
			bytes: 1_000,
		});

		assert_ok!(execute_from(stranger_loc, xcm_transact(call, OriginKind::Xcm)));

		// `XcmPassthrough` resolves the origin to `pallet_xcm::Origin::Xcm(stranger)`,
		// which does not match `IsSiblingParachain` and is not in `TestAccounts`,
		// so the inner `authorize_account` dispatch is rejected with `BadOrigin`.
		assert_eq!(extent_of(&target), empty());
	});
}

#[test]
fn authorize_with_zero_bytes_fails() {
	new_test_ext().execute_with(|| {
		let who: AccountId = Sr25519Keyring::Alice.to_account_id();

		assert_ok!(pc_authorize(who.clone(), 1, 0));

		assert_eq!(extent_of(&who), empty());
		assert!(!TransactionStorage::account_has_active_authorization(&who));
	});
}
