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

// Tests still call the deprecated `ValidateUnsigned::{validate_unsigned, pre_dispatch}` directly.
// Migration to `#[pallet::authorize]` is tracked separately; silence here so `-D warnings` in CI
// does not block the SDK bump.
#![allow(deprecated)]

use super::{
	extension::ValidateStorageCalls,
	mock::{
		new_test_ext, run_to_block, RuntimeCall, RuntimeEvent, RuntimeOrigin, StoreRenewPriority,
		System, Test, TransactionStorage,
	},
	pallet::Origin,
	AuthorizationExtent, AuthorizationScope, AuthorizedCaller, Event, TransactionInfo,
	AUTHORIZATION_NOT_EXPIRED, BAD_DATA_SIZE, DEFAULT_MAX_BLOCK_TRANSACTIONS,
	DEFAULT_MAX_TRANSACTION_SIZE,
};
use crate::migrations::v1::OldTransactionInfo;
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
type TransactionByContentHash = super::TransactionByContentHash<Test>;

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
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), caller, 0, 2001));
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 2001,
				transactions: 0,
				transactions_allowance: 0
			}
		);
		let call = Call::store { data: vec![0u8; 2000] };
		// A caller without any Authorization entry is still rejected.
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&5, &call),
			InvalidTransaction::Payment,
		);
		assert_ok!(TransactionStorage::pre_dispatch_signed(&caller, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 2001,
				transactions: 1,
				transactions_allowance: 0
			}
		);
		// A second store that overshoots the allowance no longer rejects; `bytes` saturates
		// upward and the entry stays put.
		let call = Call::store { data: vec![0u8; 2] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&caller, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			AuthorizationExtent {
				bytes: 2002,
				bytes_allowance: 2001,
				transactions: 2,
				transactions_allowance: 0
			}
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
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 2002,
				transactions: 0,
				transactions_allowance: 1
			}
		);
		// Data with a non-matching hash has no preimage auth → rejected.
		let call = Call::store { data: vec![1; 2000] };
		assert_noop!(TransactionStorage::pre_dispatch(&call), InvalidTransaction::Payment);
		// Matching data consumes allowance but the entry stays (new behaviour).
		let call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch(&call));
		// Entry persists with the remainder (2002 - 2000 = 2 bytes); the
		// transaction count is exhausted so further stores still fail.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 2002,
				transactions: 1,
				transactions_allowance: 1
			}
		);
		assert_ok!(Into::<RuntimeCall>::into(call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);
		// Renew also uses the same preimage auth; it still exists so no rejection even
		// though the used counter is pushed over the cap.
		let call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch(&call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent {
				bytes: 4000,
				bytes_allowance: 2002,
				transactions: 2,
				transactions_allowance: 1
			}
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
			TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), Some(proof)),
			Error::UnexpectedProof,
		);
		run_to_block(11, || None);
		let parent_hash = System::parent_hash();

		let invalid_proof =
			build_proof(parent_hash.as_ref(), vec![vec![0u8; 1000]]).unwrap().unwrap();
		assert_noop!(
			TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), Some(invalid_proof)),
			Error::InvalidProof,
		);

		let proof = build_proof(parent_hash.as_ref(), vec![vec![0u8; MAX_DATA_SIZE as usize]])
			.unwrap()
			.unwrap();
		assert_ok!(TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), Some(proof)));
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
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 2000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 0
			},
		);
		let call = Call::store { data: vec![0; 2000] };
		assert_ok!(TransactionStorage::validate_signed(&who, &call));
		run_to_block(10, || None);
		// validate_signed does not consume — extent unchanged.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 0
			},
		);
		assert_ok!(TransactionStorage::validate_signed(&who, &call));
		run_to_block(11, || None);
		// Expired authorizations report as zero extent.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 0,
				transactions: 0,
				transactions_allowance: 0
			},
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
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 2000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 0
			},
		);
		assert!(!System::providers(&who).is_zero());

		// User consumes 1000 bytes of the 2000-byte allowance.
		run_to_block(2, || None);
		let store_call = Call::store { data: vec![0; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 1000,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 0
			},
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
fn consumed_authorization_stays_over_cap() {
	// `check_authorization` always adds and never removes the entry on overshoot, so the
	// Authorization stays in storage (and the provider reference with it) even when `bytes`
	// exceeds `bytes_allowance`. Only expiration cleans it up.
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert!(System::providers(&who).is_zero());
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 2000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 0
			},
		);
		assert!(!System::providers(&who).is_zero());

		let call = Call::store { data: vec![0; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 1000,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 0
			},
		);
		// Second consumption saturates at the cap.
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 2000,
				transactions: 2,
				transactions_allowance: 0
			},
		);
		// Third consumption pushes `bytes` over the cap but still succeeds.
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 3000,
				bytes_allowance: 2000,
				transactions: 3,
				transactions_allowance: 0
			},
		);
		// Entry is still in storage and the provider reference is still held.
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert!(!System::providers(&who).is_zero());
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
			0,
			total_bytes
		));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: total_bytes,
				transactions: 0,
				transactions_allowance: 0
			},
		);

		for size in sizes {
			let call = Call::store { data: vec![0u8; size] };
			assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
			assert_ok!(Into::<RuntimeCall>::into(call).dispatch(RuntimeOrigin::none()));
		}

		// After using exactly the authorized allowance, bytes == bytes_allowance — entry stays.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: total_bytes,
				bytes_allowance: total_bytes,
				transactions: 6,
				transactions_allowance: 0
			},
		);
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert!(!System::providers(&who).is_zero());

		// Zero-size data must be rejected
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 1));
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
			0,
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
fn renew_content_hash_works() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);

		// Renewing a non-existent content hash should fail
		let bogus_hash = [0u8; 32];
		assert_noop!(
			TransactionStorage::renew_content_hash(RuntimeOrigin::none(), bogus_hash),
			Error::RenewedNotFound,
		);

		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));

		// Verify the content hash map was populated
		assert_eq!(TransactionByContentHash::get(content_hash), Some((1, 0)));

		run_to_block(6, || None);
		assert_ok!(TransactionStorage::renew_content_hash(RuntimeOrigin::none(), content_hash));

		// Map should now point to the new block
		assert_eq!(TransactionByContentHash::get(content_hash), Some((6, 0)));

		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::Renewed {
			index: 0,
			content_hash,
		}));
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
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 4000,
				transactions: 0,
				transactions_allowance: 0
			}
		);

		// Setup: preimage authorization also exists for the same content
		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::root(),
			content_hash,
			2000
		));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 1
			}
		);

		// Store the pre-authorized content using a signed transaction
		let call = Call::store { data: data.clone() };
		assert_ok!(TransactionStorage::validate_signed(&who, &call));
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));

		// Preimage auth was used (bytes incremented), account untouched.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1
			},
			"Preimage authorization should be consumed"
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 4000,
				transactions: 0,
				transactions_allowance: 0
			},
			"Account authorization should remain unchanged"
		);

		// Different content has no matching preimage auth → falls back to account.
		let other_data = vec![99u8; 1000];
		let other_call = Call::store { data: other_data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &other_call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 1000,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0
			},
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
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 4000,
				transactions: 0,
				transactions_allowance: 0
			}
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

		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0
			},
			"Account authorization should be consumed when no matching preimage auth"
		);
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(different_hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 1000,
				transactions: 0,
				transactions_allowance: 1
			},
			"Unrelated preimage authorization should remain unchanged"
		);
	});
}

#[test]
fn content_hash_map_cleaned_on_expiry() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		assert!(TransactionByContentHash::get(content_hash).is_some());

		let proof_provider = || {
			let block_num = System::block_number();
			if block_num == 11 {
				let parent_hash = System::parent_hash();
				build_proof(parent_hash.as_ref(), vec![vec![0u8; 2000]]).unwrap()
			} else {
				None
			}
		};

		// Advance past storage period; block 1 data expires at block 12
		run_to_block(12, proof_provider);
		assert!(TransactionByContentHash::get(content_hash).is_none());
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
		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0
			},
		);

		run_to_block(3, || None);

		// No preimage authorization exists for the content hash — renew uses account auth.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));

		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 4000,
				bytes_allowance: 4000,
				transactions: 2,
				transactions_allowance: 0
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
		assert_ok!(TransactionStorage::renew_content_hash(RuntimeOrigin::none(), content_hash));
		assert_eq!(TransactionByContentHash::get(content_hash), Some((6, 0)));

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
		assert_eq!(TransactionByContentHash::get(content_hash), Some((6, 0)));

		// Block 6 data expires at block 17
		run_to_block(17, proof_provider);
		assert!(TransactionByContentHash::get(content_hash).is_none());
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
		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		// Account auth now at the cap (still present, just fully used).
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0
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
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 1
			}
		);
		// Account auth is untouched by `authorize_preimage`.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0
			}
		);

		// Renew using signed transaction - should prefer preimage authorization
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));

		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1
			},
			"Preimage authorization should be consumed for renew"
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0
			},
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
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1
			}
		);
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		// sha2 preimage consumed to cap; entry stays.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(sha2_hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1
			}
		);
		// Blake2 authorization should remain unconsumed.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(blake2_hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 1
			}
		);

		// Finalize block so Transactions storage is populated.
		run_to_block(3, || None);

		// Verify stored entry uses SHA2-256 and content_hash matches.
		let txs = Transactions::get(1).expect("transactions stored at block 1");
		assert_eq!(txs.len(), 1);
		assert_eq!(txs[0].hashing, HashingAlgorithm::Sha2_256);
		assert_eq!(txs[0].cid_codec, 0x55);
		assert_eq!(txs[0].content_hash, sha2_hash);

		// Renew with the sha2 preimage auth still present — succeeds, pushes bytes over cap.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch(&renew_call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(sha2_hash),
			AuthorizationExtent {
				bytes: 4000,
				bytes_allowance: 2000,
				transactions: 2,
				transactions_allowance: 1
			}
		);
	});
}

#[test]
fn validate_signed_account_authorization_has_provides_tag() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1u64;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 2000));

		let call = Call::store { data: vec![0u8; 2000] };

		// validate_signed still doesn't consume authorization (correct behaviour).
		for _ in 0..2 {
			assert_ok!(TransactionStorage::validate_signed(&who, &call));
		}
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 0
			},
		);

		let (vt, _) = TransactionStorage::validate_signed(&who, &call).unwrap();
		assert!(!vt.provides.is_empty(), "validate_signed must emit a `provides` tag");

		// Two calls with the same signer + content produce identical tags, confirming
		// that the mempool will deduplicate them.
		let (vt2, _) = TransactionStorage::validate_signed(&who, &call).unwrap();
		assert_eq!(vt.provides, vt2.provides);

		// Both pre_dispatch calls succeed: the entry stays and `bytes` saturates upward.
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 4000,
				bytes_allowance: 2000,
				transactions: 2,
				transactions_allowance: 0
			},
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
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 2000));

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
		assert_eq!(TransactionStorage::in_code_storage_version(), StorageVersion::new(2));

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
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 10000));
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));

		// Partially consume authorization
		let call = Call::store { data: vec![0; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

#[test]
fn try_state_detects_zero_authorization_allowance() {
	// The only invariant left on stored authorizations is that `bytes_allowance > 0`; `bytes`
	// (used) can be any value (including over cap) since consumption saturates upward.
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);

		// Authorization SCALE layout: extent(AuthorizationExtent), expiration(u64)
		// AuthorizationExtent SCALE layout: transactions(u32), transactions_allowance(u32),
		// bytes(u64), bytes_allowance(u64)
		let corrupted_auth = (0u32, 0u32, 0u64, 0u64, 100u64); // bytes_allowance=0, expiration=100
		let key = Authorizations::hashed_key_for(AuthorizationScope::Account(1u64));
		unhashed::put_raw(&key, &corrupted_auth.encode());

		assert_err!(
			TransactionStorage::do_try_state(System::block_number()),
			"Stored authorization has zero bytes_allowance"
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
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), caller, 0, 16));

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

		// Run prepare — this should call pre_dispatch_signed and add to the used counter.
		let ext2 = ValidateStorageCalls::<Test>::default();
		assert_ok!(ext2.prepare(val, &origin_for_prepare, &call, &info, 0));

		// After prepare: 16 bytes used, entry at cap (not removed).
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			AuthorizationExtent {
				bytes: 16,
				bytes_allowance: 16,
				transactions: 1,
				transactions_allowance: 0
			},
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

/// Helper: initialize block N with proper extrinsic context for manual on_initialize + dispatch.
fn init_block(n: u64) {
	System::set_block_number(n);
	System::reset_events();
	// Set extrinsic index so sp_io::transaction_index::renew works
	unhashed::put::<u32>(b":extrinsic_index", &0);
	<TransactionStorage as polkadot_sdk_frame::traits::Hooks<u64>>::on_initialize(n);
}

type AutoRenewals = super::AutoRenewals<Test>;
type PendingAutoRenewals = super::PendingAutoRenewals<Test>;

#[test]
fn enable_auto_renew_works() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		// Authorize and store. Note: store accepts unsigned origin (or the custom
		// Origin::Authorized set by ValidateStorageCalls extension). Plain signed origin
		// is rejected by ensure_authorized().
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		// Enable auto-renew
		assert_ok!(
			TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), content_hash,)
		);

		// Verify storage
		let renewal_data = AutoRenewals::get(content_hash).unwrap();
		assert_eq!(renewal_data.account, who);

		// Verify event
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::AutoRenewalEnabled {
			content_hash,
			who,
		}));

		// Enabling again should fail
		assert_noop!(
			TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), content_hash),
			Error::AutoRenewalAlreadyEnabled,
		);
	});
}

#[test]
fn enable_auto_renew_rejects_invalid() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;

		// Enabling for non-existent content hash fails
		let bogus_hash = blake2_256(&[99u8; 100]);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_noop!(
			TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), bogus_hash),
			Error::RenewedNotFound,
		);

		// Enabling without account authorization fails
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		let unauthorized_user = 99;
		assert_noop!(
			TransactionStorage::enable_auto_renew(
				RuntimeOrigin::signed(unauthorized_user),
				content_hash
			),
			Error::AuthorizationNotFound,
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
		assert_ok!(TransactionStorage::enable_auto_renew(
			RuntimeOrigin::signed(owner),
			content_hash,
		));

		// Another user cannot disable
		assert_noop!(
			TransactionStorage::disable_auto_renew(RuntimeOrigin::signed(other), content_hash),
			Error::NotAutoRenewalOwner,
		);

		// Owner can disable
		assert_ok!(TransactionStorage::disable_auto_renew(
			RuntimeOrigin::signed(owner),
			content_hash,
		));

		assert!(AutoRenewals::get(content_hash).is_none());
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::AutoRenewalDisabled {
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
			TransactionStorage::disable_auto_renew(RuntimeOrigin::signed(who), content_hash),
			Error::AutoRenewalNotEnabled,
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
		assert_ok!(
			TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), content_hash,)
		);

		// Verify initial state
		assert_eq!(TransactionByContentHash::get(content_hash), Some((1, 0)));
		assert!(Transactions::get(1).is_some());

		// Build proof provider for both the original block and the renewal block
		let proof_provider = move || {
			let block_num = System::block_number();
			let period: u64 = RetentionPeriod::get();
			let target = block_num.saturating_sub(period);
			if target > 0 && Transactions::get(target).is_some() {
				let parent_hash = System::parent_hash();
				let txs = Transactions::get(target).unwrap();
				let data_vec: Vec<Vec<u8>> = txs.iter().map(|_| data.clone()).collect();
				build_proof(parent_hash.as_ref(), data_vec).unwrap()
			} else {
				None
			}
		};

		// Advance to block 11 (retention_period=10, so block 1's data expires at block 12).
		// At block 12's on_initialize, obsolete = 12 - 10 - 1 = 1, so Transactions(1) is taken.
		// But we need to provide proof at block 11 for block 1's data.
		run_to_block(11, proof_provider);

		// Verify data still exists before expiry
		assert!(Transactions::get(1).is_some());

		// Block 12: on_initialize takes Transactions(1) and populates PendingAutoRenewals.
		// But run_to_block runs on_initialize + on_finalize. The on_finalize will panic
		// because PendingAutoRenewals is not empty (no inherent ran).
		// We need to manually advance and call process_auto_renewals.

		// Advance block number to 12 manually
		init_block(12);

		// Verify PendingAutoRenewals was populated
		let pending = PendingAutoRenewals::get();
		assert_eq!(pending.len(), 1);
		assert_eq!(pending[0].0, content_hash);

		// Process auto-renewals (simulating the mandatory extrinsic)
		// Refresh authorization before renewal (AuthorizationPeriod is 10 blocks,
		// so auth granted at block 1 expired at block 11)
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));

		assert_ok!(TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), None));

		// Verify PendingAutoRenewals is now empty
		assert!(PendingAutoRenewals::get().is_empty());

		// Verify data was renewed into the current block
		assert_eq!(TransactionByContentHash::get(content_hash), Some((12, 0)));

		// Verify event
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::DataAutoRenewed {
			index: 0,
			content_hash,
			account: who,
		}));

		// Verify old Transactions entry was removed and new one exists
		assert!(Transactions::get(1).is_none());

		// Auto-renewal registration should still exist
		assert!(AutoRenewals::get(content_hash).is_some());
	});
}

#[test]
fn auto_renewal_consumes_authorization() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		// Authorize with exactly enough for 2 operations (store doesn't consume here,
		// since it's unsigned, but renew does via process_auto_renewals)
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 3, 6000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(
			TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), content_hash,)
		);

		let initial_extent = TransactionStorage::account_authorization_extent(who);
		assert_eq!(
			initial_extent,
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 6000,
				transactions: 0,
				transactions_allowance: 3,
			},
		);

		// Trigger expiry at block 12 — refresh auth first (AuthorizationPeriod = 10 blocks).
		// The block-1 authorization expired at block 11, so this creates a fresh entry.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 3, 6000));
		assert_ok!(TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), None));

		// Authorization should have been consumed (1 transaction, 2000 bytes)
		let after_extent = TransactionStorage::account_authorization_extent(who);
		assert_eq!(
			after_extent,
			AuthorizationExtent {
				bytes: 2000,
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

		// Authorize (needed for enable_auto_renew check)
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 5, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(
			TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), content_hash,)
		);

		// First renewal at block 12 — refresh with exactly 1 operation worth of auth
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 1, 2000));
		assert_ok!(TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), None));

		// Authorization is now fully consumed (used == allowance)
		let extent = TransactionStorage::account_authorization_extent(who);
		assert_eq!(
			extent,
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1,
			},
		);

		// Data was renewed to block 12
		assert_eq!(TransactionByContentHash::get(content_hash), Some((12, 0)));

		// Simulate on_finalize: move BlockTransactions → Transactions(12)
		let block_txs = BlockTransactions::take();
		if !block_txs.is_empty() {
			Transactions::insert(12u64, &block_txs);
		}

		// Second renewal at block 23 (12 + 10 + 1) — should fail
		// We need block 23 because: obsolete = 23 - 10 - 1 = 12
		init_block(23);
		let pending = PendingAutoRenewals::get();
		assert_eq!(pending.len(), 1, "Should have pending renewal");

		assert_ok!(TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), None));

		// Should have failed — event emitted and auto-renewal removed
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::AutoRenewalFailed {
			content_hash,
			account: who,
		}));
		assert!(AutoRenewals::get(content_hash).is_none(), "Auto-renewal should be removed");
	});
}

#[test]
fn process_auto_renewals_rejects_signed_origin() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert_noop!(
			TransactionStorage::apply_block_inherents(RuntimeOrigin::signed(1), None),
			DispatchError::BadOrigin,
		);
	});
}

#[test]
fn process_auto_renewals_noop_when_empty() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		// Calling with no pending renewals should succeed (no-op)
		assert_ok!(TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), None));
		assert!(PendingAutoRenewals::get().is_empty());
	});
}

#[test]
fn pending_auto_renewals_populated_only_for_registered_items() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data1 = vec![0u8; 2000];
		let data2 = vec![1u8; 2000];
		let hash1 = blake2_256(&data1);
		let _hash2 = blake2_256(&data2);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data1));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data2));
		run_to_block(2, || None);

		// Only enable auto-renew for hash1, not hash2
		assert_ok!(TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), hash1,));

		// Trigger expiry
		init_block(12);

		let pending = PendingAutoRenewals::get();
		assert_eq!(pending.len(), 1, "Only hash1 should be pending");
		assert_eq!(pending[0].0, hash1);
	});
}

#[test]
fn auto_renew_permissionless_transfer() {
	// Alice stores and enables auto-renew, then disables. Bob enables instead.
	// Anyone can choose to keep data alive on Bulletin, permissionlessly.
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
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		// Alice enables auto-renew
		assert_ok!(TransactionStorage::enable_auto_renew(
			RuntimeOrigin::signed(alice),
			content_hash,
		));
		let renewal = AutoRenewals::get(content_hash).unwrap();
		assert_eq!(renewal.account, alice);

		// Alice disables auto-renew
		assert_ok!(TransactionStorage::disable_auto_renew(
			RuntimeOrigin::signed(alice),
			content_hash,
		));
		assert!(AutoRenewals::get(content_hash).is_none());

		// Bob authorizes and enables auto-renew for the same content
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), bob, 10, 100_000));
		assert_ok!(
			TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(bob), content_hash,)
		);

		let renewal = AutoRenewals::get(content_hash).unwrap();
		assert_eq!(renewal.account, bob, "Bob should now own the auto-renewal");

		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::AutoRenewalEnabled {
			content_hash,
			who: bob,
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
		let max_txns = <<Test as crate::Config>::MaxBlockTransactions as Get<u32>>::get();
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

		// Enable auto-renew for all three
		for hash in &hashes {
			assert_ok!(TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), *hash,));
		}

		// Fill up BlockTransactions so that renewals will hit TooManyTransactions.
		// We do this by manually inserting items up to (max - 1), leaving room for only 1 renewal.
		init_block(12);
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			max_txns + 10,
			100_000_000
		));

		// Verify PendingAutoRenewals was populated with 3 items
		let pending = PendingAutoRenewals::get();
		assert_eq!(pending.len(), 3);

		// Fill block with (max - 1) dummy transactions so only 1 renewal fits
		BlockTransactions::mutate(|txns| {
			for _ in 0..(max_txns - 1) {
				let _ = txns.try_push(TransactionInfo {
					chunk_root: Default::default(),
					size: 100,
					content_hash: [0u8; 32],
					hashing: crate::HashingAlgorithm::Blake2b256,
					cid_codec: 0x55,
					block_chunks: 0,
				});
			}
		});

		// Process auto-renewals — should NOT return an error even though 2 of 3 fail
		assert_ok!(TransactionStorage::apply_block_inherents(RuntimeOrigin::none(), None));

		// PendingAutoRenewals should be fully consumed
		assert!(PendingAutoRenewals::get().is_empty());

		// First item should have succeeded (DataAutoRenewed event).
		// Index is max_txns - 1 because the block already has max_txns - 1 items (0-indexed).
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::DataAutoRenewed {
			index: max_txns - 1,
			content_hash: hashes[0],
			account: who,
		}));

		// Remaining items should have failed (AutoRenewalFailed events)
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::AutoRenewalFailed {
			content_hash: hashes[1],
			account: who,
		}));
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::AutoRenewalFailed {
			content_hash: hashes[2],
			account: who,
		}));

		// Auto-renewal registrations should be removed for failed items
		assert!(AutoRenewals::get(hashes[1]).is_none());
		assert!(AutoRenewals::get(hashes[2]).is_none());
	});
}

/// Run a normal block lifecycle past expiry without invoking `apply_block_inherents`.
///
/// `on_initialize` populates `PendingAutoRenewals`; `on_finalize` then enforces that the
/// inherent ran, asserting that the storage is empty. The mock's `run_to_block` always
/// invokes the inherent, hiding this safeguard. This test bypasses the helper to confirm
/// the assert actually fires when an auto-renewal is pending and the inherent is missing.
#[test]
#[should_panic(expected = "All pending auto-renewals must be processed by apply_block_inherents")]
fn on_finalize_panics_when_inherent_missing() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data.clone()));
		run_to_block(2, || None);
		assert_ok!(TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), content_hash));

		let proof_provider = move || {
			let block_num = System::block_number();
			let period: u64 = RetentionPeriod::get();
			let target = block_num.saturating_sub(period);
			if target > 0 && Transactions::get(target).is_some() {
				let parent_hash = System::parent_hash();
				let txs = Transactions::get(target).unwrap();
				let data_vec: Vec<Vec<u8>> = txs.iter().map(|_| data.clone()).collect();
				build_proof(parent_hash.as_ref(), data_vec).unwrap()
			} else {
				None
			}
		};

		// Run normally up to (and including) block 11 — proofs supplied via the inherent.
		run_to_block(11, proof_provider);

		// Manually advance to block 12 and run only on_initialize, which populates
		// PendingAutoRenewals as Transactions(1) expires. We deliberately do NOT call
		// apply_block_inherents, simulating an inherent that was lost or never built.
		init_block(12);
		assert_eq!(
			PendingAutoRenewals::get().len(),
			1,
			"on_initialize should have populated pending"
		);

		// on_finalize must panic on the PendingAutoRenewals invariant. The proof check passes
		// here because target_block (12 - 10 = 2) has no transactions stored.
		<TransactionStorage as polkadot_sdk_frame::traits::Hooks<u64>>::on_finalize(12);
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

		// Baseline: no proof, no pending renewals → no inherent emitted.
		let empty = InherentData::new();
		assert!(
			<TransactionStorage as ProvideInherent>::create_inherent(&empty).is_none(),
			"no inherent should be emitted when neither proof nor pending renewals are present",
		);

		let who = 1;
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 100_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);
		assert_ok!(TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), content_hash));

		// Advance to block 12 with on_initialize only — Transactions(1) expires and
		// PendingAutoRenewals gains one entry.
		run_to_block(11, || None);
		init_block(12);
		assert_eq!(PendingAutoRenewals::get().len(), 1);

		// `InherentData` carries no proof. The provider must still emit the composite call so
		// that the inherent-driven drain runs in this block.
		let result = <TransactionStorage as ProvideInherent>::create_inherent(&empty);
		match result {
			Some(Call::apply_block_inherents { proof: None }) => {},
			other => panic!(
				"expected Some(apply_block_inherents {{ proof: None }}) when only pending renewals \
				 are present, got {:?}",
				other
			),
		}
	});
}

#[test]
fn re_authorize_account_adds_to_allowance_and_keeps_expiry() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let call = Call::store { data: vec![0; 2000] };
		// Initial authorization at block 1: expires at block 1 + 10 = 11.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 2000));

		// Re-authorize at block 5 within the unexpired window: the new `bytes` add to the
		// existing cap, expiry stays at 11.
		run_to_block(5, || None);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 1000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 3000,
				transactions: 0,
				transactions_allowance: 0
			},
		);

		// Still valid at block 10.
		run_to_block(10, || None);
		assert_ok!(TransactionStorage::validate_signed(&who, &call));

		// Expires at block 11 (original expiry, not pushed back).
		run_to_block(11, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 0,
				transactions: 0,
				transactions_allowance: 0
			},
		);
		assert_noop!(TransactionStorage::validate_signed(&who, &call), InvalidTransaction::Payment);
	});
}

#[test]
fn re_authorize_account_preserves_used_bytes() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		// Initial 4000-byte cap, consume 2000.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store = Call::store { data: vec![0; 2000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store));

		// Add another 1000: cap becomes 5000, used stays at 2000.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 1000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_allowance: 5000,
				transactions: 1,
				transactions_allowance: 0
			},
		);
	});
}

#[test]
fn re_authorize_account_after_expiry_resets() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		// Initial authorization at block 1: expires at block 11. Consume some bytes.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store = Call::store { data: vec![0; 2000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store));

		// Re-authorize after expiry: replaces with a fresh entry (zero used, new expiry).
		run_to_block(20, || None);
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 1500));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 1500,
				transactions: 0,
				transactions_allowance: 0
			},
		);
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
		// Preimage re-authorize takes max(existing, new) for `bytes_allowance`.
		run_to_block(5, || None);
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), hash, 3000));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 3000,
				transactions: 0,
				transactions_allowance: 1
			},
		);

		// Still valid at block 10.
		run_to_block(10, || None);
		assert_ok!(TransactionStorage::validate_signed(&1, &call));

		// Expires at block 11 (original expiry), NOT 15.
		run_to_block(11, || None);
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_allowance: 0,
				transactions: 0,
				transactions_allowance: 0
			},
		);
	});
}
