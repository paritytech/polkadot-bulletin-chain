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

//! Tests for transaction-storage pallet.

use super::{
	extension::ValidateStorageCalls,
	mock::{
		new_test_ext, run_to_block, RuntimeCall, RuntimeEvent, RuntimeOrigin, StoreRenewPriority,
		System, Test, TransactionStorage,
	},
	pallet::Origin,
	AllowedAuthorizers, AuthorizationExtent, AuthorizationScope, AuthorizedCaller,
	AuthorizerBudget, EnsureAllowedAuthorizers, Event, TransactionInfo,
	AUTHORIZATION_NOT_EXHAUSTED, AUTHORIZATION_NOT_EXPIRED, AUTHORIZER_NOT_FOUND, BAD_DATA_SIZE,
	DEFAULT_MAX_BLOCK_TRANSACTIONS, DEFAULT_MAX_TRANSACTION_SIZE,
};

use crate::{
	migrations::{v1::OldTransactionInfo, PopulateAllowedAuthorizersIfEmpty},
	mock::RuntimeGenesisConfig,
};
use bulletin_transaction_storage_primitives::cids::{CidConfig, HashingAlgorithm};
use codec::Encode;
use polkadot_sdk_frame::{
	deps::frame_support::{
		storage::unhashed,
		traits::{GetStorageVersion, OnRuntimeUpgrade},
		BoundedVec,
	},
	hashing::blake2_256,
	prelude::*,
	testing_prelude::*,
	traits::StorageVersion,
};
use sp_transaction_storage_proof::{random_chunk, registration::build_proof, CHUNK_SIZE};

type Call = super::Call<Test>;
type Error = super::Error<Test>;

type Authorizations = super::Authorizations<Test>;
type BlockTransactions = super::BlockTransactions<Test>;
type RetentionPeriod = super::RetentionPeriod<Test>;
type Transactions = super::Transactions<Test>;

fn test_budget(transactions: u32, bytes: u64) -> AuthorizerBudget<u64> {
	AuthorizerBudget {
		transactions_budget: transactions,
		bytes_budget: bytes,
		authorization_period: None,
	}
}

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
		for transaction in transactions.clone() {
			assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), transaction));
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
		let max = DEFAULT_MAX_TRANSACTION_SIZE as usize;
		let sizes: [usize; 6] = [
			1,           // minimum valid size
			2000,        // small
			max / 4,     // 25%
			max / 2,     // 50%
			max * 3 / 4, // 75%
			max,         // 100% (exactly at limit)
		];
		let total_bytes: u64 = sizes.iter().map(|s| *s as u64).sum();
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			sizes.len() as u32,
			total_bytes
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

		// Zero-size data must be rejected
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 1));
		let empty_call = Call::store { data: vec![] };
		assert_noop!(TransactionStorage::pre_dispatch_signed(&who, &empty_call), BAD_DATA_SIZE);
		assert_noop!(
			Into::<RuntimeCall>::into(empty_call).dispatch(RuntimeOrigin::none()),
			Error::BadDataSize,
		);

		// Assert that a payload exceeding the max size fails, even with fresh authorization
		let oversize: usize = max + 1;
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			1,
			oversize as u64
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

#[test]
fn signed_store_prefers_preimage_authorization_over_account() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];
		let content_hash = blake2_256(&data);

		// Setup: user has account authorization
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 2, 4000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 2, bytes: 4000 }
		);

		// Setup: preimage authorization also exists for the same content
		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::root(),
			content_hash,
			2000
		));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent { transactions: 1, bytes: 2000 }
		);

		// Store the pre-authorized content using a signed transaction
		let call = Call::store { data: data.clone() };
		assert_ok!(TransactionStorage::validate_signed(&who, &call));
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));

		// Verify: preimage authorization was consumed, not account authorization
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent { transactions: 0, bytes: 0 },
			"Preimage authorization should be consumed"
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 2, bytes: 4000 },
			"Account authorization should remain unchanged"
		);

		// User can still use their account authorization for different content
		let other_data = vec![99u8; 1000];
		let other_call = Call::store { data: other_data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &other_call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 3000 },
			"Account authorization should be used for non-pre-authorized content"
		);
	});
}

#[test]
fn signed_store_falls_back_to_account_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];
		let different_hash = blake2_256(&[0u8; 100]); // Hash for different content

		// Setup: user has account authorization
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 2, 4000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 2, bytes: 4000 }
		);

		// Setup: preimage authorization exists but for DIFFERENT content
		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::root(),
			different_hash,
			1000
		));

		// Store content that doesn't have preimage authorization
		let call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));

		// Verify: account authorization was consumed since no preimage auth for this content
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 2000 },
			"Account authorization should be consumed when no matching preimage auth"
		);
		// Preimage authorization for different content should remain unchanged
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(different_hash),
			AuthorizationExtent { transactions: 1, bytes: 1000 },
			"Unrelated preimage authorization should remain unchanged"
		);
	});
}

#[test]
fn signed_renew_uses_account_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];
		let content_hash = blake2_256(&data);

		// Setup: authorize preimage and store the data
		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::root(),
			content_hash,
			2000
		));
		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch(&store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		run_to_block(3, || None);

		// Setup: user has account authorization for renew
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 2000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 2000 }
		);

		// Renew the stored data using signed transaction.
		// Since preimage authorization was consumed during store, renew falls back to account.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));

		// Verify: account authorization was consumed for renew
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 0, bytes: 0 },
			"Account authorization should be consumed for renew when no preimage auth"
		);
	});
}

#[test]
fn signed_renew_prefers_preimage_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];
		let content_hash = blake2_256(&data);

		// Setup: store data using account authorization
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 2000));
		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		// Account authorization consumed after store
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 0, bytes: 0 }
		);

		run_to_block(3, || None);

		// Setup: authorize both preimage and account for renew
		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::root(),
			content_hash,
			2000
		));
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 2000));

		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent { transactions: 1, bytes: 2000 }
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 2000 }
		);

		// Renew using signed transaction - should prefer preimage authorization
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));

		// Verify: preimage authorization was consumed, account authorization unchanged
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent { transactions: 0, bytes: 0 },
			"Preimage authorization should be consumed for renew"
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 2000 },
			"Account authorization should remain unchanged when preimage auth is used"
		);
	});
}

#[test]
fn store_with_cid_config_uses_custom_hashing() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let data = vec![42u8; 2000];

		// Store with default config (Blake2b256 + raw codec 0x55)
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data.clone()));
		let default_info = BlockTransactions::get().last().unwrap().clone();
		assert_eq!(default_info.hashing, HashingAlgorithm::Blake2b256);
		assert_eq!(default_info.cid_codec, 0x55);

		// Store with explicit SHA2-256 config
		let sha2_config = CidConfig { codec: 0x55, hashing: HashingAlgorithm::Sha2_256 };
		assert_ok!(TransactionStorage::store_with_cid_config(
			RuntimeOrigin::none(),
			sha2_config.clone(),
			data.clone(),
		));
		let sha2_info = BlockTransactions::get().last().unwrap().clone();
		assert_eq!(sha2_info.hashing, HashingAlgorithm::Sha2_256);
		assert_eq!(sha2_info.cid_codec, 0x55);
		// Content hashes differ because different hashing algorithms are used
		assert_ne!(default_info.content_hash, sha2_info.content_hash);

		// Store with explicit Blake2b256 config (same as default but explicitly set)
		let blake2_config = CidConfig { codec: 0x55, hashing: HashingAlgorithm::Blake2b256 };
		assert_ok!(TransactionStorage::store_with_cid_config(
			RuntimeOrigin::none(),
			blake2_config.clone(),
			data.clone(),
		));
		let blake2_info = BlockTransactions::get().last().unwrap().clone();
		assert_eq!(blake2_info.hashing, HashingAlgorithm::Blake2b256);
		assert_eq!(blake2_info.cid_codec, 0x55);
		assert_eq!(default_info.content_hash, blake2_info.content_hash);

		// Finalize block 1 and verify Transactions storage
		run_to_block(2, || None);
		let txs = Transactions::get(1).expect("transactions should be stored for block 1");
		assert_eq!(txs.len(), 3);
		assert_eq!(txs[0].hashing, HashingAlgorithm::Blake2b256);
		assert_eq!(txs[0].cid_codec, 0x55);
		assert_eq!(txs[1].hashing, HashingAlgorithm::Sha2_256);
		assert_eq!(txs[1].cid_codec, 0x55);
		assert_eq!(txs[2].hashing, HashingAlgorithm::Blake2b256);
		assert_eq!(txs[2].cid_codec, 0x55);
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
		let store_call =
			Call::store_with_cid_config { cid: sha2_config.clone(), data: data.clone() };
		run_to_block(1, || None);
		assert_noop!(TransactionStorage::pre_dispatch(&store_call), InvalidTransaction::Payment);

		// Authorize preimage with SHA2 hash (matching the CID config's algorithm).
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), sha2_hash, 2000));

		// store_with_cid_config goes through check_unsigned → check_store_renew_unsigned.
		assert_ok!(TransactionStorage::pre_dispatch(&store_call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(sha2_hash),
			AuthorizationExtent { transactions: 0, bytes: 0 }
		);
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		// Preimage authorization for sha2 hash should be consumed.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(sha2_hash),
			AuthorizationExtent { transactions: 0, bytes: 0 }
		);
		// Blake2 authorization should remain unconsumed.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(blake2_hash),
			AuthorizationExtent { transactions: 1, bytes: 2000 }
		);

		// Finalize block so Transactions storage is populated.
		run_to_block(3, || None);

		// Verify stored entry uses SHA2-256 and content_hash matches.
		let txs = Transactions::get(1).expect("transactions stored at block 1");
		assert_eq!(txs.len(), 1);
		assert_eq!(txs[0].hashing, HashingAlgorithm::Sha2_256);
		assert_eq!(txs[0].cid_codec, 0x55);
		assert_eq!(txs[0].content_hash, sha2_hash);

		// Renew without authorization fails.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_noop!(TransactionStorage::pre_dispatch(&renew_call), InvalidTransaction::Payment);

		// Authorize preimage with SHA2 hash (renew uses stored content_hash).
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), sha2_hash, 2000));
		assert_ok!(TransactionStorage::pre_dispatch(&renew_call));

		// Preimage authorization for sha2 hash should be consumed.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(sha2_hash),
			AuthorizationExtent { transactions: 0, bytes: 0 }
		);
	});
}

#[test]
fn validate_signed_account_authorization_has_provides_tag() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1u64;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 2000));

		let call = Call::store { data: vec![0u8; 2000] };

		// validate_signed still doesn't consume authorization (correct behaviour).
		for _ in 0..2 {
			assert_ok!(TransactionStorage::validate_signed(&who, &call));
		}
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 2000 },
		);

		let (vt, _) = TransactionStorage::validate_signed(&who, &call).unwrap();
		assert!(!vt.provides.is_empty(), "validate_signed must emit a `provides` tag");

		// Two calls with the same signer + content produce identical tags, confirming
		// that the mempool will deduplicate them.
		let (vt2, _) = TransactionStorage::validate_signed(&who, &call).unwrap();
		assert_eq!(vt.provides, vt2.provides);

		// pre_dispatch still enforces the authorization: only the first succeeds.
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&who, &call),
			InvalidTransaction::Payment,
		);

		// Now test the preimage-authorized path: signed preimage tags must match unsigned
		// preimage tags so the pool deduplicates across both submission types.
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);
		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::root(),
			content_hash,
			2000,
		));
		// Re-authorize account so validate_signed can fall through if needed.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 2000));

		let (signed_vt, _) = TransactionStorage::validate_signed(&who, &call).unwrap();
		let unsigned_vt = <TransactionStorage as ValidateUnsigned>::validate_unsigned(
			TransactionSource::External,
			&call,
		)
		.unwrap();
		assert_eq!(
			signed_vt.provides, unsigned_vt.provides,
			"signed preimage path must produce the same tag as unsigned preimage path"
		);

		// A different signer submitting the same pre-authorized content must get the same
		// tag, proving dedup is content-based, not signer-based.
		let other_who = 2u64;
		let (other_vt, _) = TransactionStorage::validate_signed(&other_who, &call).unwrap();
		assert_eq!(
			signed_vt.provides, other_vt.provides,
			"different signers with same preimage-authorized content must share the same tag"
		);
	});
}

// ---- Migration tests ----

/// Write old-format `OldTransactionInfo` entries as raw bytes into the `Transactions`
/// storage slot for `block_num`. Uses synthetic field values — the migration re-encodes
/// fields 1:1 without validating chunk roots or content hashes.
fn insert_old_format_transactions(block_num: u64, count: u32) {
	use polkadot_sdk_frame::deps::sp_runtime::traits::{BlakeTwo256, Hash};

	let old_txs: Vec<OldTransactionInfo> = (0..count)
		.map(|i| OldTransactionInfo {
			chunk_root: BlakeTwo256::hash(&[i as u8]),
			content_hash: BlakeTwo256::hash(&[i as u8 + 100]),
			size: 2000,
			block_chunks: (i + 1) * 8,
		})
		.collect();
	let bounded: BoundedVec<OldTransactionInfo, ConstU32<DEFAULT_MAX_BLOCK_TRANSACTIONS>> =
		old_txs.try_into().expect("within bounds");
	let key = Transactions::hashed_key_for(block_num);
	unhashed::put_raw(&key, &bounded.encode());
}

#[test]
fn migration_v1_old_entries_only() {
	new_test_ext().execute_with(|| {
		// Simulate pre-migration state: on-chain version 0
		StorageVersion::new(0).put::<TransactionStorage>();
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(0));

		// Insert old-format entries at blocks 1, 2, 3
		insert_old_format_transactions(1, 2);
		insert_old_format_transactions(2, 1);
		insert_old_format_transactions(3, 3);

		// Can't decode with new type
		assert!(Transactions::get(1).is_none());
		assert!(Transactions::get(2).is_none());
		assert!(Transactions::get(3).is_none());

		// But raw bytes exist
		assert!(Transactions::contains_key(1));
		assert!(Transactions::contains_key(2));
		assert!(Transactions::contains_key(3));

		// Run v0→v1 migration
		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(1));

		// Entries are now directly decodable after v0→v1 (v1 layout matches TransactionInfo)
		let txs1 = Transactions::get(1).expect("should decode after v1 migration");
		assert_eq!(txs1.len(), 2);
		for tx in txs1.iter() {
			assert_eq!(tx.hashing, HashingAlgorithm::Blake2b256);
			assert_eq!(tx.cid_codec, 0x55);
			assert_eq!(tx.size, 2000);
		}

		let txs2 = Transactions::get(2).expect("should decode");
		assert_eq!(txs2.len(), 1);

		let txs3 = Transactions::get(3).expect("should decode");
		assert_eq!(txs3.len(), 3);
	});
}

#[test]
fn migration_v1_new_entries_only() {
	new_test_ext().execute_with(|| {
		StorageVersion::new(0).put::<TransactionStorage>();
		run_to_block(1, || None);

		// Store via normal (new-format) code path
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![0u8; 2000]));
		run_to_block(2, || None);

		let original = Transactions::get(1).expect("should decode");
		assert_eq!(original.len(), 1);

		// Run migration
		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();

		// Entry unchanged
		let after = Transactions::get(1).expect("should decode");
		assert_eq!(original, after);
	});
}

#[test]
fn migration_v1_mixed_entries() {
	new_test_ext().execute_with(|| {
		StorageVersion::new(0).put::<TransactionStorage>();

		// Old-format entry at block 5
		insert_old_format_transactions(5, 2);
		assert!(Transactions::get(5).is_none());

		// New-format entry at block 10
		run_to_block(10, || None);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![42u8; 500]));
		run_to_block(11, || None);
		let new_entry_before = Transactions::get(10).expect("new format decodes");

		// Run migration
		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();

		// Old entry transformed to v1 format — now directly decodable
		let old_entry_after = Transactions::get(5).expect("should decode after v1 migration");
		assert_eq!(old_entry_after.len(), 2);

		// New entry preserved exactly
		let new_entry_after = Transactions::get(10).expect("still decodes");
		assert_eq!(new_entry_before, new_entry_after);
	});
}

#[test]
fn migration_v1_version_updated() {
	new_test_ext().execute_with(|| {
		StorageVersion::new(0).put::<TransactionStorage>();
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(0));
		assert_eq!(TransactionStorage::in_code_storage_version(), StorageVersion::new(1));

		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();

		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(1));
	});
}

#[test]
fn migration_v1_idempotent() {
	new_test_ext().execute_with(|| {
		StorageVersion::new(0).put::<TransactionStorage>();
		insert_old_format_transactions(1, 1);

		// First run: migrates old entries to v1 format
		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(1));
		// v1 format is not decodable as v2 TransactionInfo, but raw bytes exist
		let key = Transactions::hashed_key_for(1u64);
		let raw_after_first = unhashed::get_raw(&key).expect("raw bytes exist");

		// Second run: noop (version already 1)
		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(1));
		let raw_after_second = unhashed::get_raw(&key).expect("raw bytes still exist");

		assert_eq!(raw_after_first, raw_after_second);
	});
}

#[test]
fn migration_v1_empty_storage() {
	new_test_ext().execute_with(|| {
		StorageVersion::new(0).put::<TransactionStorage>();
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(0));

		// No Transactions entries exist
		assert_eq!(Transactions::iter().count(), 0);

		// Run migration
		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();

		// Version updated, no entries created
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(1));
		assert_eq!(Transactions::iter().count(), 0);
	});
}

// ---- try_state tests ----

#[test]
fn try_state_passes_on_empty_storage() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

#[test]
fn try_state_passes_after_store_and_finalize() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![0u8; 2000]));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![1u8; 500]));
		run_to_block(2, || None);
		// After finalization, ephemeral storage is cleared and transactions are persisted
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

#[test]
fn try_state_passes_through_retention_lifecycle() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![0u8; 2000]));
		let proof_provider = || {
			let block_num = System::block_number();
			if block_num == 11 {
				let parent_hash = System::parent_hash();
				build_proof(parent_hash.as_ref(), vec![vec![0u8; 2000]]).unwrap()
			} else {
				None
			}
		};
		// Run past retention period; block 1 transactions get cleaned up at block 12
		run_to_block(12, proof_provider);
		assert!(Transactions::get(1).is_none());
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

#[test]
fn try_state_passes_with_active_authorizations() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 5, 10000));
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));

		// Partially consume authorization
		let call = Call::store { data: vec![0; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

#[test]
fn try_state_detects_zero_authorization_transactions() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);

		// Insert a corrupted authorization with zero transactions using raw storage.
		// Authorization SCALE layout: extent(AuthorizationExtent), expiration(u64)
		// AuthorizationExtent SCALE layout: transactions(u32), bytes(u64)
		let corrupted_auth = (0u32, 100u64, 100u64); // transactions=0, bytes=100, expiration=100
		let key = Authorizations::hashed_key_for(AuthorizationScope::Account(1u64));
		unhashed::put_raw(&key, &corrupted_auth.encode());

		assert_err!(
			TransactionStorage::do_try_state(System::block_number()),
			"Stored authorization has zero transactions remaining"
		);
	});
}

#[test]
fn try_state_detects_zero_authorization_bytes() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);

		// Insert a corrupted authorization with zero bytes using raw storage.
		let corrupted_auth = (5u32, 0u64, 100u64); // transactions=5, bytes=0, expiration=100
		let key = Authorizations::hashed_key_for(AuthorizationScope::Account(1u64));
		unhashed::put_raw(&key, &corrupted_auth.encode());

		assert_err!(
			TransactionStorage::do_try_state(System::block_number()),
			"Stored authorization has zero bytes remaining"
		);
	});
}

#[test]
fn try_state_detects_zero_retention_period() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);

		// Set RetentionPeriod to zero
		RetentionPeriod::put(0u64);

		assert_err!(
			TransactionStorage::do_try_state(System::block_number()),
			"RetentionPeriod must not be zero"
		);
	});
}

#[test]
fn try_state_passes_with_preimage_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let hash = blake2_256(&[1u8; 32]);
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), hash, 5000));
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

// ---- ValidateStorageCalls extension tests ----

#[test]
fn ensure_authorized_extracts_custom_origin() {
	new_test_ext().execute_with(|| {
		let who: u64 = 42;

		// 1. Authorized origin with Account scope
		let authorized_origin: RuntimeOrigin =
			Origin::<Test>::Authorized { who, scope: AuthorizationScope::Account(who) }.into();
		assert_eq!(
			TransactionStorage::ensure_authorized(authorized_origin),
			Ok(AuthorizedCaller::Signed { who, scope: AuthorizationScope::Account(who) }),
		);

		// 2. Authorized origin with Preimage scope
		let content_hash = [0u8; 32];
		let preimage_origin: RuntimeOrigin = Origin::<Test>::Authorized {
			who: 99,
			scope: AuthorizationScope::Preimage(content_hash),
		}
		.into();
		assert_eq!(
			TransactionStorage::ensure_authorized(preimage_origin),
			Ok(AuthorizedCaller::Signed {
				who: 99,
				scope: AuthorizationScope::Preimage(content_hash)
			}),
		);

		// 3. Root origin → Root
		assert_eq!(
			TransactionStorage::ensure_authorized(RuntimeOrigin::root()),
			Ok(AuthorizedCaller::Root),
		);

		// 4. None origin → Unsigned
		assert_eq!(
			TransactionStorage::ensure_authorized(RuntimeOrigin::none()),
			Ok(AuthorizedCaller::Unsigned),
		);

		// 5. Plain signed origin → BadOrigin
		assert_eq!(
			TransactionStorage::ensure_authorized(RuntimeOrigin::signed(123)),
			Err(DispatchError::BadOrigin),
		);
	});
}

#[test]
fn authorize_storage_extension_transforms_origin() {
	use polkadot_sdk_frame::{
		prelude::TransactionSource,
		traits::{DispatchInfoOf, TransactionExtension, TxBaseImplication},
	};

	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let caller = 1u64;
		let data = vec![0u8; 16];

		// Give caller account authorization
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), caller, 1, 16));

		// Create the store call
		let call: RuntimeCall = Call::store { data }.into();
		let info: DispatchInfoOf<RuntimeCall> = Default::default();
		let origin = RuntimeOrigin::signed(caller);

		// Run ValidateStorageCalls::validate - this should transform the origin
		let ext = ValidateStorageCalls::<Test>::default();
		let result = ext.validate(
			origin,
			&call,
			&info,
			0,
			(),
			&TxBaseImplication(&call),
			TransactionSource::External,
		);

		assert!(result.is_ok());
		let (valid_tx, val, transformed_origin) = result.unwrap();

		// Verify the transaction is valid with correct priority
		assert_eq!(valid_tx.priority, StoreRenewPriority::get());

		// Verify val contains the signer
		assert_eq!(val, Some(caller));

		// Verify the origin was transformed and can be extracted with ensure_authorized
		let origin_for_prepare = transformed_origin.clone();
		assert_eq!(
			TransactionStorage::ensure_authorized(transformed_origin),
			Ok(AuthorizedCaller::Signed {
				who: caller,
				scope: AuthorizationScope::Account(caller)
			}),
		);

		// Run prepare — this should call pre_dispatch_signed and consume the authorization
		let ext2 = ValidateStorageCalls::<Test>::default();
		assert_ok!(ext2.prepare(val, &origin_for_prepare, &call, &info, 0));

		// Authorization (1 transaction, 16 bytes) should now be fully consumed
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			AuthorizationExtent { transactions: 0, bytes: 0 },
		);
	});
}

#[test]
fn authorize_storage_extension_transforms_origin_with_preimage_auth() {
	use polkadot_sdk_frame::{
		prelude::TransactionSource,
		traits::{DispatchInfoOf, TransactionExtension, TxBaseImplication},
	};

	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let caller = 1u64;
		let data = vec![0u8; 16];
		let content_hash = blake2_256(&data);

		// Give preimage authorization (not account authorization)
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), content_hash, 16));

		// Create the store call
		let call: RuntimeCall = Call::store { data }.into();
		let info: DispatchInfoOf<RuntimeCall> = Default::default();
		let origin = RuntimeOrigin::signed(caller);

		// Run ValidateStorageCalls::validate
		let ext = ValidateStorageCalls::<Test>::default();
		let result = ext.validate(
			origin,
			&call,
			&info,
			0,
			(),
			&TxBaseImplication(&call),
			TransactionSource::External,
		);

		assert!(result.is_ok());
		let (_, val, transformed_origin) = result.unwrap();

		// Verify val contains the signer
		assert_eq!(val, Some(caller));

		// Verify the origin carries preimage authorization
		assert_eq!(
			TransactionStorage::ensure_authorized(transformed_origin),
			Ok(AuthorizedCaller::Signed {
				who: caller,
				scope: AuthorizationScope::Preimage(content_hash)
			}),
		);
	});
}

#[test]
fn authorize_storage_extension_passes_through_non_storage_calls() {
	use polkadot_sdk_frame::{
		prelude::{TransactionSource, ValidTransaction},
		traits::{AsSystemOriginSigner, DispatchInfoOf, TransactionExtension, TxBaseImplication},
	};

	new_test_ext().execute_with(|| {
		let caller = 1u64;

		// Create a non-TransactionStorage call (using System::remark as example)
		let call: RuntimeCall = frame_system::Call::remark { remark: vec![] }.into();
		let info: DispatchInfoOf<RuntimeCall> = Default::default();
		let origin = RuntimeOrigin::signed(caller);

		// Run ValidateStorageCalls::validate - should pass through unchanged
		let ext = ValidateStorageCalls::<Test>::default();
		let result = ext.validate(
			origin.clone(),
			&call,
			&info,
			0,
			(),
			&TxBaseImplication(&call),
			TransactionSource::External,
		);

		assert!(result.is_ok());
		let (valid_tx, val, returned_origin) = result.unwrap();

		// Verify passthrough behavior
		assert_eq!(valid_tx, ValidTransaction::default());
		assert_eq!(val, None);

		// Origin should still be a signed origin (not transformed)
		assert!(returned_origin.as_system_origin_signer().is_some());
		assert_eq!(returned_origin.as_system_origin_signer().unwrap(), &caller);
	});
}

#[test]
fn add_authorizer_inserts_into_storage_and_emits_event() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 42u64;
		assert!(!AllowedAuthorizers::<Test>::contains_key(who));

		assert_ok!(TransactionStorage::add_authorizer(
			RuntimeOrigin::root(),
			who,
			100,
			1024 * 1024,
			None
		));

		assert!(AllowedAuthorizers::<Test>::contains_key(who));
		let budget = AllowedAuthorizers::<Test>::get(who).unwrap();
		assert_eq!(budget.transactions_budget, 100);
		assert_eq!(budget.bytes_budget, 1024 * 1024);
		assert!(budget.authorization_period.is_none());
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::AuthorizerAdded { who }));
	});
}

#[test]
fn remove_authorizer_removes_from_storage_and_emits_event() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 42u64;
		AllowedAuthorizers::<Test>::insert(who, test_budget(100, 1024));

		assert_ok!(TransactionStorage::remove_authorizer(RuntimeOrigin::root(), who));

		assert!(!AllowedAuthorizers::<Test>::contains_key(who));
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::AuthorizerRemoved {
			who,
		}));
	});
}

#[test]
fn add_authorizer_rejects_non_manager_origin() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			TransactionStorage::add_authorizer(RuntimeOrigin::signed(1), 42, 100, 1024, None),
			DispatchError::BadOrigin,
		);
	});
}

#[test]
fn remove_authorizer_rejects_non_manager_origin() {
	new_test_ext().execute_with(|| {
		AllowedAuthorizers::<Test>::insert(42u64, test_budget(100, 1024));
		assert_noop!(
			TransactionStorage::remove_authorizer(RuntimeOrigin::signed(1), 42),
			DispatchError::BadOrigin,
		);
	});
}

#[test]
fn add_authorizer_is_idempotent() {
	new_test_ext().execute_with(|| {
		let who = 42u64;
		assert_ok!(TransactionStorage::add_authorizer(RuntimeOrigin::root(), who, 100, 1024, None));
		// Second call overwrites with same value, still Ok.
		assert_ok!(TransactionStorage::add_authorizer(RuntimeOrigin::root(), who, 200, 2048, None));
		assert!(AllowedAuthorizers::<Test>::contains_key(who));
		let budget = AllowedAuthorizers::<Test>::get(who).unwrap();
		assert_eq!(budget.transactions_budget, 200);
		assert_eq!(budget.bytes_budget, 2048);
	});
}

#[test]
fn remove_authorizer_on_absent_entry_is_ok() {
	new_test_ext().execute_with(|| {
		// Not present; remove is still a successful no-op write.
		assert_ok!(TransactionStorage::remove_authorizer(RuntimeOrigin::root(), 42));
	});
}

#[test]
fn ensure_allowed_authorizer_accepts_signed_account_in_storage() {
	new_test_ext().execute_with(|| {
		let who = 7u64;
		AllowedAuthorizers::<Test>::insert(who, test_budget(100, 1024));
		let origin = RuntimeOrigin::signed(who);
		assert_eq!(EnsureAllowedAuthorizers::<Test>::try_origin(origin).ok(), Some(who));
	});
}

#[test]
fn ensure_allowed_authorizer_rejects_signed_account_not_in_storage() {
	new_test_ext().execute_with(|| {
		let origin = RuntimeOrigin::signed(99);
		assert!(EnsureAllowedAuthorizers::<Test>::try_origin(origin).is_err());
	});
}

#[test]
fn ensure_allowed_authorizer_rejects_root() {
	new_test_ext().execute_with(|| {
		assert!(EnsureAllowedAuthorizers::<Test>::try_origin(RuntimeOrigin::root()).is_err());
	});
}

#[test]
fn ensure_allowed_authorizer_rejects_none() {
	new_test_ext().execute_with(|| {
		assert!(EnsureAllowedAuthorizers::<Test>::try_origin(RuntimeOrigin::none()).is_err());
	});
}

#[test]
fn genesis_populates_allowed_authorizers() {
	let t = RuntimeGenesisConfig {
		system: Default::default(),
		transaction_storage: crate::GenesisConfig::<Test> {
			retention_period: 10,
			byte_fee: 2,
			entry_fee: 200,
			account_authorizations: vec![],
			preimage_authorizations: vec![],
			allowed_authorizers: vec![(1, 100, 1024), (2, 200, 2048), (3, 300, 4096)],
		},
	}
	.build_storage()
	.unwrap();

	TestExternalities::new(t).execute_with(|| {
		assert!(AllowedAuthorizers::<Test>::contains_key(1));
		assert!(AllowedAuthorizers::<Test>::contains_key(2));
		assert!(AllowedAuthorizers::<Test>::contains_key(3));
		assert_eq!(AllowedAuthorizers::<Test>::iter().count(), 3);
		let budget = AllowedAuthorizers::<Test>::get(1).unwrap();
		assert_eq!(budget.transactions_budget, 100);
		assert_eq!(budget.bytes_budget, 1024);
	});
}

#[test]
fn populate_allowed_authorizers_migration_seeds_empty_storage() {
	new_test_ext().execute_with(|| {
		assert_eq!(AllowedAuthorizers::<Test>::iter().count(), 0);

		parameter_types! {
			pub Seed: Vec<u64> = vec![10, 20];
			pub MigrationBudget: AuthorizerBudget<u64> = test_budget(500, 5000);
		}
		PopulateAllowedAuthorizersIfEmpty::<Test, Seed, MigrationBudget>::on_runtime_upgrade();

		assert!(AllowedAuthorizers::<Test>::contains_key(10));
		assert!(AllowedAuthorizers::<Test>::contains_key(20));
		let budget = AllowedAuthorizers::<Test>::get(10).unwrap();
		assert_eq!(budget.transactions_budget, 500);
		assert_eq!(budget.bytes_budget, 5000);
	});
}

#[test]
fn populate_allowed_authorizers_migration_skips_non_empty_storage() {
	new_test_ext().execute_with(|| {
		AllowedAuthorizers::<Test>::insert(99u64, test_budget(100, 1024));

		parameter_types! {
			pub Seed: Vec<u64> = vec![10, 20];
			pub MigrationBudget: AuthorizerBudget<u64> = test_budget(500, 5000);
		}
		PopulateAllowedAuthorizersIfEmpty::<Test, Seed, MigrationBudget>::on_runtime_upgrade();

		// Only the pre-existing entry; seed not applied.
		assert!(AllowedAuthorizers::<Test>::contains_key(99));
		assert!(!AllowedAuthorizers::<Test>::contains_key(10));
		assert!(!AllowedAuthorizers::<Test>::contains_key(20));
	});
}

#[test]
fn populate_allowed_authorizers_migration_is_idempotent() {
	new_test_ext().execute_with(|| {
		parameter_types! {
			pub Seed: Vec<u64> = vec![10];
			pub MigrationBudget: AuthorizerBudget<u64> = test_budget(500, 5000);
		}
		// First run: seeds.
		PopulateAllowedAuthorizersIfEmpty::<Test, Seed, MigrationBudget>::on_runtime_upgrade();
		assert!(AllowedAuthorizers::<Test>::contains_key(10));

		// Second run: non-empty, no-op.
		PopulateAllowedAuthorizersIfEmpty::<Test, Seed, MigrationBudget>::on_runtime_upgrade();
		assert_eq!(AllowedAuthorizers::<Test>::iter().count(), 1);
	});
}

#[test]
fn authorize_account_does_not_push_expiry() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let call = Call::store { data: vec![0; 2000] };
		// Initial authorization at block 1: expires at block 1 + 10 = 11.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 2000));

		// Extend at block 5: expiration should stay at 11, not move to 15.
		run_to_block(5, || None);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 1000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 2, bytes: 3000 },
		);

		// Still valid at block 10.
		run_to_block(10, || None);
		assert_ok!(TransactionStorage::validate_signed(&who, &call));

		// Expires at block 11 (original expiry), NOT 15.
		run_to_block(11, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 0, bytes: 0 },
		);
		assert_noop!(TransactionStorage::validate_signed(&who, &call), InvalidTransaction::Payment);
	});
}

#[test]
fn authorize_preimage_does_not_push_expiry() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let data = vec![0u8; 2000];
		let hash = blake2_256(&data);
		let call = Call::store { data };
		// Initial authorization at block 1: expires at block 1 + 10 = 11.
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), hash, 2000));

		// Re-authorize at block 5 with larger max_size: expiration should stay at 11.
		run_to_block(5, || None);
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), hash, 3000));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent { transactions: 1, bytes: 3000 },
		);

		// Still valid at block 10.
		run_to_block(10, || None);
		assert_ok!(TransactionStorage::validate_signed(&1, &call));

		// Expires at block 11 (original expiry), NOT 15.
		run_to_block(11, || None);
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent { transactions: 0, bytes: 0 },
		);
	});
}

// ---- Authorizer budget tests ----

#[test]
fn remove_exhausted_authorizer_works_when_both_zero() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 42u64;
		AllowedAuthorizers::<Test>::insert(who, test_budget(0, 0));

		// check_unsigned should pass
		let call = Call::remove_exhausted_authorizer { who };
		assert_ok!(TransactionStorage::pre_dispatch(&call));

		// Dispatch should succeed
		assert_ok!(TransactionStorage::remove_exhausted_authorizer(RuntimeOrigin::none(), who));
		assert!(!AllowedAuthorizers::<Test>::contains_key(who));
		System::assert_has_event(RuntimeEvent::TransactionStorage(
			Event::ExhaustedAuthorizerRemoved { who },
		));
	});
}

#[test]
fn remove_exhausted_authorizer_works_when_transactions_zero() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 42u64;
		AllowedAuthorizers::<Test>::insert(who, test_budget(0, 1000));

		let call = Call::remove_exhausted_authorizer { who };
		assert_ok!(TransactionStorage::pre_dispatch(&call));
		assert_ok!(TransactionStorage::remove_exhausted_authorizer(RuntimeOrigin::none(), who));
		assert!(!AllowedAuthorizers::<Test>::contains_key(who));
	});
}

#[test]
fn remove_exhausted_authorizer_works_when_bytes_zero() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 42u64;
		AllowedAuthorizers::<Test>::insert(who, test_budget(100, 0));

		let call = Call::remove_exhausted_authorizer { who };
		assert_ok!(TransactionStorage::pre_dispatch(&call));
		assert_ok!(TransactionStorage::remove_exhausted_authorizer(RuntimeOrigin::none(), who));
		assert!(!AllowedAuthorizers::<Test>::contains_key(who));
	});
}

#[test]
fn remove_exhausted_authorizer_fails_with_remaining_budget() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 42u64;
		AllowedAuthorizers::<Test>::insert(who, test_budget(10, 1000));

		let call = Call::remove_exhausted_authorizer { who };
		assert_noop!(TransactionStorage::pre_dispatch(&call), AUTHORIZATION_NOT_EXHAUSTED);
		assert_noop!(
			TransactionStorage::remove_exhausted_authorizer(RuntimeOrigin::none(), who),
			Error::AuthorizerBudgetNotExhausted,
		);
		assert!(AllowedAuthorizers::<Test>::contains_key(who));
	});
}

#[test]
fn remove_exhausted_authorizer_fails_for_missing_authorizer() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 99u64;

		let call = Call::remove_exhausted_authorizer { who };
		assert_noop!(TransactionStorage::pre_dispatch(&call), AUTHORIZER_NOT_FOUND);
		assert_noop!(
			TransactionStorage::remove_exhausted_authorizer(RuntimeOrigin::none(), who),
			Error::AuthorizerNotFound,
		);
	});
}

#[test]
fn add_authorizer_with_period_override() {
	new_test_ext().execute_with(|| {
		let who = 42u64;
		assert_ok!(TransactionStorage::add_authorizer(
			RuntimeOrigin::root(),
			who,
			100,
			1024,
			Some(5)
		));
		let budget = AllowedAuthorizers::<Test>::get(who).unwrap();
		assert_eq!(budget.authorization_period, Some(5));
	});
}

#[test]
fn authorizer_budget_consumed_on_authorize_account() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let authorizer = 10u64;
		AllowedAuthorizers::<Test>::insert(authorizer, test_budget(5, 10_000));

		// Simulate check_signed PreDispatch for authorize_account
		let call = Call::authorize_account { who: 1, transactions: 2, bytes: 4000 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&authorizer, &call));

		let budget = AllowedAuthorizers::<Test>::get(authorizer).unwrap();
		assert_eq!(budget.transactions_budget, 3);
		assert_eq!(budget.bytes_budget, 6000);
	});
}

#[test]
fn authorizer_budget_consumed_on_authorize_preimage() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let authorizer = 10u64;
		AllowedAuthorizers::<Test>::insert(authorizer, test_budget(5, 10_000));

		let hash = [0u8; 32];
		let call = Call::authorize_preimage { content_hash: hash, max_size: 3000 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&authorizer, &call));

		let budget = AllowedAuthorizers::<Test>::get(authorizer).unwrap();
		// authorize_preimage consumes 1 transaction, max_size bytes
		assert_eq!(budget.transactions_budget, 4);
		assert_eq!(budget.bytes_budget, 7000);
	});
}

#[test]
fn authorizer_budget_insufficient_transactions_rejected() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let authorizer = 10u64;
		AllowedAuthorizers::<Test>::insert(authorizer, test_budget(1, 10_000));

		let call = Call::authorize_account { who: 1, transactions: 5, bytes: 1000 };
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&authorizer, &call),
			InvalidTransaction::Payment,
		);
		// Budget unchanged
		let budget = AllowedAuthorizers::<Test>::get(authorizer).unwrap();
		assert_eq!(budget.transactions_budget, 1);
	});
}

#[test]
fn authorizer_budget_insufficient_bytes_rejected() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let authorizer = 10u64;
		AllowedAuthorizers::<Test>::insert(authorizer, test_budget(100, 500));

		let call = Call::authorize_account { who: 1, transactions: 1, bytes: 1000 };
		assert_err!(
			TransactionStorage::pre_dispatch_signed(&authorizer, &call),
			TransactionValidityError::Invalid(InvalidTransaction::Payment),
		);
		// Budget unchanged — try_consume failed before writing
		let budget = AllowedAuthorizers::<Test>::get(authorizer).unwrap();
		assert_eq!(budget.bytes_budget, 500);
	});
}

#[test]
fn root_bypasses_authorizer_budget() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		// Root can authorize without being in AllowedAuthorizers
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), 1, 10, 10_000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(1),
			AuthorizationExtent { transactions: 10, bytes: 10_000 },
		);
	});
}

#[test]
fn authorization_period_override_via_pending_storage() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let authorizer = 10u64;
		// Authorizer with custom 5-block period (default is 10)
		AllowedAuthorizers::<Test>::insert(
			authorizer,
			AuthorizerBudget {
				transactions_budget: 100,
				bytes_budget: 100_000,
				authorization_period: Some(5),
			},
		);

		let who = 1u64;
		let call = Call::authorize_account { who, transactions: 1, bytes: 1000 };
		// pre_dispatch_signed stashes the period override and consumes budget
		assert_ok!(TransactionStorage::pre_dispatch_signed(&authorizer, &call));

		// Now dispatch — authorize() should pick up the pending period
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 1000));

		// The authorization should expire at block 1 + 5 = 6 (not 1 + 10 = 11)
		// Check: at block 5, authorization still valid
		run_to_block(5, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1, bytes: 1000 },
		);
		// At block 6, authorization expired
		run_to_block(6, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 0, bytes: 0 },
		);
	});
}
