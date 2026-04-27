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

//! Tests for hop-promotion pallet.

use crate::mock::*;
use codec::Encode;
use frame_support::{assert_noop, assert_ok, traits::Authorize};
use sp_runtime::transaction_validity::{InvalidTransaction, TransactionSource};

fn authorized_origin() -> RuntimeOrigin {
	frame_system::Origin::<Test>::Authorized.into()
}

// ---- Dispatch tests ----

#[test]
fn promote_succeeds_with_valid_data() {
	new_test_ext().execute_with(|| {
		System::run_to_block::<AllPalletsWithSystem>(1);
		frame_system::Pallet::<Test>::set_extrinsic_index(0);
		let data = vec![42u8; 100];
		assert_ok!(HopPromotion::promote(authorized_origin(), data));
	});
}

#[test]
fn promote_rejects_empty_data() {
	new_test_ext().execute_with(|| {
		System::run_to_block::<AllPalletsWithSystem>(1);
		frame_system::Pallet::<Test>::set_extrinsic_index(0);
		assert_noop!(
			HopPromotion::promote(authorized_origin(), vec![]),
			pallet_bulletin_transaction_storage::Error::<Test>::BadDataSize,
		);
	});
}

#[test]
fn promote_rejects_oversized_data() {
	new_test_ext().execute_with(|| {
		System::run_to_block::<AllPalletsWithSystem>(1);
		frame_system::Pallet::<Test>::set_extrinsic_index(0);
		assert_noop!(
			HopPromotion::promote(
				authorized_origin(),
				vec![0u8; TEST_MAX_TRANSACTION_SIZE as usize + 1]
			),
			pallet_bulletin_transaction_storage::Error::<Test>::BadDataSize,
		);
	});
}

#[test]
fn promote_rejects_non_authorized_origins() {
	new_test_ext().execute_with(|| {
		System::run_to_block::<AllPalletsWithSystem>(1);
		let data = vec![42u8; 100];
		assert_noop!(
			HopPromotion::promote(RuntimeOrigin::none(), data.clone()),
			sp_runtime::traits::BadOrigin,
		);
		assert_noop!(
			HopPromotion::promote(RuntimeOrigin::signed(1), data.clone()),
			sp_runtime::traits::BadOrigin,
		);
		assert_noop!(
			HopPromotion::promote(RuntimeOrigin::root(), data),
			sp_runtime::traits::BadOrigin,
		);
	});
}

// ---- Authorize closure tests ----

fn make_promote_call(data: Vec<u8>) -> RuntimeCall {
	RuntimeCall::HopPromotion(crate::Call::promote { data })
}

#[test]
fn authorize_rejects_external_source() {
	new_test_ext().execute_with(|| {
		let call = make_promote_call(vec![1u8; 100]);
		assert_eq!(
			call.authorize(TransactionSource::External),
			Some(Err(InvalidTransaction::Call.into())),
		);
	});
}

#[test]
fn authorize_accepts_local_source() {
	new_test_ext().execute_with(|| {
		let call = make_promote_call(vec![1u8; 100]);
		assert!(matches!(call.authorize(TransactionSource::Local), Some(Ok(_))));
	});
}

#[test]
fn authorize_accepts_in_block_source() {
	new_test_ext().execute_with(|| {
		let call = make_promote_call(vec![1u8; 100]);
		assert!(matches!(call.authorize(TransactionSource::InBlock), Some(Ok(_))));
	});
}

#[test]
fn authorize_rejects_empty_data() {
	new_test_ext().execute_with(|| {
		let call = make_promote_call(vec![]);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::Custom(0).into())),
		);
	});
}

#[test]
fn authorize_rejects_oversized_data() {
	new_test_ext().execute_with(|| {
		let call = make_promote_call(vec![0u8; TEST_MAX_TRANSACTION_SIZE as usize + 1]);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::Custom(0).into())),
		);
	});
}

#[test]
fn authorize_valid_transaction_properties() {
	new_test_ext().execute_with(|| {
		let data = vec![1u8; 100];
		let call = make_promote_call(data.clone());
		let result = call.authorize(TransactionSource::Local);
		let (valid_tx, weight) = result.unwrap().unwrap();
		assert_eq!(valid_tx.priority, 0);
		assert_eq!(valid_tx.longevity, 5);
		assert!(!valid_tx.propagate);
		assert_eq!(weight, frame_support::weights::Weight::zero());
		let hash = sp_io::hashing::blake2_256(&data);
		let expected_tag = ("HopPromotion", hash).encode();
		assert!(valid_tx.provides.contains(&expected_tag));
	});
}

#[test]
fn promote_has_lower_priority_than_store_and_renew() {
	new_test_ext().execute_with(|| {
		System::run_to_block::<AllPalletsWithSystem>(1);
		frame_system::Pallet::<Test>::set_extrinsic_index(0);

		// Get promote priority.
		let promote_call = make_promote_call(vec![1u8; 100]);
		let (promote_tx, _) = promote_call.authorize(TransactionSource::Local).unwrap().unwrap();

		// Authorize an account for store + renew.
		let who: u64 = 1;
		let data = vec![2u8; 100];
		assert_ok!(pallet_bulletin_transaction_storage::Pallet::<Test>::authorize_account(
			RuntimeOrigin::root(),
			who,
			2,
			2 * data.len() as u64,
		));

		// Get store priority.
		let store_call = pallet_bulletin_transaction_storage::Call::<Test>::store { data: data.clone() };
		let (store_tx, _) =
			pallet_bulletin_transaction_storage::Pallet::<Test>::validate_signed(&who, &store_call).unwrap();

		// Store data so we can renew it.
		assert_ok!(pallet_bulletin_transaction_storage::Pallet::<Test>::store(RuntimeOrigin::none(), data,));

		// Advance so the stored transaction is available for renew.
		System::run_to_block::<AllPalletsWithSystem>(3);

		// Get renew priority.
		let renew_call = pallet_bulletin_transaction_storage::Call::<Test>::renew { block: 1, index: 0 };
		let (renew_tx, _) =
			pallet_bulletin_transaction_storage::Pallet::<Test>::validate_signed(&who, &renew_call).unwrap();

		assert!(
			promote_tx.priority < store_tx.priority,
			"promote priority ({}) must be strictly less than store priority ({})",
			promote_tx.priority,
			store_tx.priority,
		);
		assert!(
			promote_tx.priority < renew_tx.priority,
			"promote priority ({}) must be strictly less than renew priority ({})",
			promote_tx.priority,
			renew_tx.priority,
		);
	});
}
