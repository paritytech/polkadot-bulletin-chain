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

//! Tests for transction-storage pallet.

use super::{
	mock::{
		new_test_ext, run_to_block, RuntimeCall, RuntimeEvent, RuntimeOrigin, System, Test,
		TransactionStorage,
	},
	AuthorizationExtent, AuthorizationScope, Event, TransactionInfo, AUTHORIZATION_NOT_EXPIRED,
	BAD_DATA_SIZE, DEFAULT_MAX_TRANSACTION_SIZE,
};
use polkadot_sdk_frame::{
	prelude::{frame_system::RawOrigin, *},
	testing_prelude::*,
};
use sp_transaction_storage_proof::{random_chunk, registration::build_proof, CHUNK_SIZE};

type Call = super::Call<Test>;
type Error = super::Error<Test>;

type Authorizations = super::Authorizations<Test>;
type BlockTransactions = super::BlockTransactions<Test>;
type Transactions = super::Transactions<Test>;

const MAX_DATA_SIZE: u32 = DEFAULT_MAX_TRANSACTION_SIZE;

#[test]
fn discards_data() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![0u8; 2000]));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![0u8; 2000]));
		let proof_provider = || {
			let block_num = System::block_number();
			if block_num == 11 {
				let parent_hash = System::parent_hash();
				build_proof(parent_hash.as_ref(), vec![vec![0u8; 2000], vec![0u8; 2000]]).unwrap()
			} else {
				None
			}
		};
		run_to_block(11, proof_provider);
		assert!(Transactions::get(1).is_some());
		let transactions = Transactions::get(1).unwrap();
		assert_eq!(transactions.len(), 2);
		run_to_block(12, proof_provider);
		assert!(Transactions::get(1).is_none());
	});
}

#[test]
fn uses_account_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let caller = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), caller, 2, 2001));
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			AuthorizationExtent { transactions: 2, bytes: 2001 }
		);
		let call = Call::store { data: vec![0u8; 2000] };
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&5, &call),
			InvalidTransaction::Payment,
		);
		assert_ok!(TransactionStorage::pre_dispatch_signed(&caller, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			AuthorizationExtent { transactions: 1, bytes: 1 }
		);
		let call = Call::store { data: vec![0u8; 2] };
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&caller, &call),
			InvalidTransaction::Payment,
		);
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
			AuthorizationExtent { transactions: 1, bytes: 2002 }
		);
		let call = Call::store { data: vec![1; 2000] };
		assert_noop!(TransactionStorage::pre_dispatch(&call), InvalidTransaction::Payment);
		let call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch(&call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent { transactions: 0, bytes: 0 }
		);
		assert_ok!(Into::<RuntimeCall>::into(call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);
		let call = Call::renew { block: 1, index: 0 };
		assert_noop!(TransactionStorage::pre_dispatch(&call), InvalidTransaction::Payment);
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), hash, 2000));
		assert_ok!(TransactionStorage::pre_dispatch(&call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent { transactions: 0, bytes: 0 }
		);
	});
}

#[test]
fn checks_proof() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert_ok!(TransactionStorage::store(
			RuntimeOrigin::none(),
			vec![0u8; MAX_DATA_SIZE as usize]
		));
		run_to_block(10, || None);
		let parent_hash = System::parent_hash();
		let proof = build_proof(parent_hash.as_ref(), vec![vec![0u8; MAX_DATA_SIZE as usize]])
			.unwrap()
			.unwrap();
		assert_noop!(
			TransactionStorage::check_proof(RuntimeOrigin::none(), proof),
			Error::UnexpectedProof,
		);
		run_to_block(11, || None);
		let parent_hash = System::parent_hash();

		let invalid_proof =
			build_proof(parent_hash.as_ref(), vec![vec![0u8; 1000]]).unwrap().unwrap();
		assert_noop!(
			TransactionStorage::check_proof(RuntimeOrigin::none(), invalid_proof),
			Error::InvalidProof,
		);

		let proof = build_proof(parent_hash.as_ref(), vec![vec![0u8; MAX_DATA_SIZE as usize]])
			.unwrap()
			.unwrap();
		assert_ok!(TransactionStorage::check_proof(RuntimeOrigin::none(), proof));
	});
}

#[test]
fn verify_chunk_proof_works() {
	new_test_ext().execute_with(|| {
		// Prepare a bunch of transactions with variable chunk sizes.
		let transactions = vec![
			vec![0u8; CHUNK_SIZE - 1],
			vec![1u8; CHUNK_SIZE],
			vec![2u8; CHUNK_SIZE + 1],
			vec![3u8; 2 * CHUNK_SIZE - 1],
			vec![3u8; 2 * CHUNK_SIZE],
			vec![3u8; 2 * CHUNK_SIZE + 1],
			vec![4u8; 7 * CHUNK_SIZE - 1],
			vec![4u8; 7 * CHUNK_SIZE],
			vec![4u8; 7 * CHUNK_SIZE + 1],
		];
		let expected_total_chunks =
			transactions.iter().map(|t| t.len().div_ceil(CHUNK_SIZE) as u32).sum::<u32>();

		// Store a couple of transactions in one block.
		run_to_block(1, || None);
		let caller = 1;
		for transaction in transactions.clone() {
			assert_ok!(TransactionStorage::store(RawOrigin::Signed(caller).into(), transaction));
		}
		run_to_block(2, || None);

		// Read all the block transactions metadata.
		let tx_infos = Transactions::get(1).unwrap();
		let total_chunks = TransactionInfo::total_chunks(&tx_infos);
		assert_eq!(expected_total_chunks, total_chunks);
		assert_eq!(9, tx_infos.len());

		// Verify proofs for all possible chunk indexes.
		for chunk_index in 0..total_chunks {
			// chunk index randomness
			let mut random_hash = [0u8; 32];
			random_hash[..8].copy_from_slice(&(chunk_index as u64).to_be_bytes());
			let selected_chunk_index = random_chunk(random_hash.as_ref(), total_chunks);
			assert_eq!(selected_chunk_index, chunk_index);

			// build/check chunk proof roundtrip
			let proof = build_proof(random_hash.as_ref(), transactions.clone())
				.expect("valid proof")
				.unwrap();
			assert_ok!(TransactionStorage::verify_chunk_proof(
				proof,
				random_hash.as_ref(),
				tx_infos.to_vec(),
			));
		}
	});
}

#[test]
fn renews_data() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![0u8; 2000]));
		let info = BlockTransactions::get().last().unwrap().clone();
		run_to_block(6, || None);
		assert_ok!(TransactionStorage::renew(
			RuntimeOrigin::none(),
			1, // block
			0, // transaction
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
		assert!(Transactions::get(1).is_none());
		assert_eq!(Transactions::get(6).unwrap().first(), Some(info).as_ref());
		run_to_block(17, proof_provider);
		assert!(Transactions::get(6).is_none());
	});
}

#[test]
fn authorization_expires() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 2000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 2000 },
		);
		let call = Call::store { data: vec![0; 2000] };
		assert_ok!(TransactionStorage::validate_signed(&who, &call));
		run_to_block(10, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 2000 },
		);
		assert_ok!(TransactionStorage::validate_signed(&who, &call));
		run_to_block(11, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 0, bytes: 0 },
		);
		assert_noop!(TransactionStorage::validate_signed(&who, &call), InvalidTransaction::Payment);
	});
}

#[test]
fn expired_authorization_clears() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert!(System::providers(&who).is_zero());
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 2, 2000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 2, bytes: 2000 },
		);
		assert!(!System::providers(&who).is_zero());

		// User uses some of the authorization, and the remaining amount gets updated appropriately
		run_to_block(2, || None);
		let store_call = Call::store { data: vec![0; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 1000 },
		);

		// Can't remove too early
		run_to_block(10, || None);
		let remove_call = Call::remove_expired_account_authorization { who };
		assert_noop!(TransactionStorage::pre_dispatch(&remove_call), AUTHORIZATION_NOT_EXPIRED);
		assert_noop!(
			Into::<RuntimeCall>::into(remove_call.clone()).dispatch(RuntimeOrigin::none()),
			Error::AuthorizationNotExpired,
		);

		// User has sufficient storage authorization, but it has expired
		run_to_block(11, || None);
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert!(!System::providers(&who).is_zero());
		// User cannot use authorization
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&who, &store_call),
			InvalidTransaction::Payment,
		);
		// Anyone can remove it
		assert_ok!(TransactionStorage::pre_dispatch(&remove_call));
		assert_ok!(Into::<RuntimeCall>::into(remove_call).dispatch(RuntimeOrigin::none()));
		System::assert_has_event(RuntimeEvent::TransactionStorage(
			Event::ExpiredAccountAuthorizationRemoved { who },
		));
		// No longer in storage
		assert!(!Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert!(System::providers(&who).is_zero());
	});
}

#[test]
fn consumed_authorization_clears() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert!(System::providers(&who).is_zero());
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 2, 2000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 2, bytes: 2000 },
		);
		assert!(!System::providers(&who).is_zero());

		// User uses some of the authorization, and the remaining amount gets updated appropriately
		let call = Call::store { data: vec![0; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		// Debited half the authorization
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 1000 },
		);
		assert!(!System::providers(&who).is_zero());
		// Consume the remaining amount
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		// Key should be cleared from Authorizations
		assert!(!Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert!(System::providers(&who).is_zero());
	});
}

#[test]
fn stores_various_sizes_with_account_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		#[allow(clippy::identity_op)]
		let sizes: [usize; 5] = [
			2000,            // 2 KB
			1 * 1024 * 1024, // 1 MB
			4 * 1024 * 1024, // 4 MB
			6 * 1024 * 1024, // 6 MB
			8 * 1024 * 1024, // 8 MB
		];
		let total_bytes: u64 = sizes.iter().map(|s| *s as u64).sum();
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			sizes.len() as u32,
			total_bytes,
		));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: sizes.len() as u32, bytes: total_bytes },
		);

		for size in sizes {
			let call = Call::store { data: vec![0u8; size] };
			assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
			assert_ok!(Into::<RuntimeCall>::into(call).dispatch(RuntimeOrigin::none()));
		}

		// After consuming the authorized sizes, authorization should be removed and providers
		// cleared
		assert!(!Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert!(System::providers(&who).is_zero());

		// Now assert that an 11 MB payload exceeds the max size and fails, even with fresh
		// authorization
		let oversize: usize = 11 * 1024 * 1024; // 11 MB > DEFAULT_MAX_TRANSACTION_SIZE (8 MB)
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			1,
			oversize as u64,
		));
		let too_big_call = Call::store { data: vec![0u8; oversize] };
		// pre_dispatch should reject due to BAD_DATA_SIZE
		assert_noop!(TransactionStorage::pre_dispatch_signed(&who, &too_big_call), BAD_DATA_SIZE);
		// dispatch should also reject with pallet Error::BadDataSize
		assert_noop!(
			Into::<RuntimeCall>::into(too_big_call).dispatch(RuntimeOrigin::none()),
			Error::BadDataSize,
		);
		run_to_block(2, || None);
	});
}
