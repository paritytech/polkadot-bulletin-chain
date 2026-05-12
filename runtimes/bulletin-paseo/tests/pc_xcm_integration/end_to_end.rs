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

//! End-to-end: People Chain authorizes, then the user submits a signed
//! `store` and a signed `renew` on Bulletin Chain.

use super::*;

#[test]
fn authorize_then_user_stores_and_renews() {
	new_test_ext().execute_with(|| {
		let account = Sr25519Keyring::Alice;
		let who: AccountId = account.to_account_id();

		// PC issues a 2-tx, 4_000-byte allowance via XCM.
		assert_ok!(pc_authorize(who.clone(), 2, 4_000));
		assert_eq!(extent_of(&who), extent(0, 4_000, 0, 2));

		// Authorized user stores 1_000 bytes (feeless, boost-tier).
		advance_block();
		let stored_block = System::block_number();
		assert_extrinsic_ok(construct_and_apply_extrinsic(
			Some(account.pair()),
			RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
				data: vec![0u8; 1_000],
			}),
		));
		assert_eq!(extent_of(&who), extent(1_000, 4_000, 1, 2));

		// Same user renews against the just-stored block/index. `renew`
		// charges the per-window permanent quota (`bytes_permanent`).
		advance_block();
		assert_extrinsic_ok(construct_and_apply_extrinsic(
			Some(account.pair()),
			RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::renew {
				block: stored_block,
				index: 0,
			}),
		));
		assert_eq!(
			extent_of(&who),
			AuthorizationExtent {
				bytes: 1_000,
				bytes_permanent: 1_000,
				bytes_allowance: 4_000,
				transactions: 2,
				transactions_allowance: 2,
			},
		);
	});
}
