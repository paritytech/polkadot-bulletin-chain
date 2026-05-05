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
		new_test_ext, run_to_block, MaxPermanentStorageSize, RuntimeCall, RuntimeEvent,
		RuntimeOrigin, StoreRenewPriority, System, Test, TransactionStorage,
	},
	pallet::Origin,
	AuthorizationExtent, AuthorizationScope, AuthorizedCaller, Event, TransactionInfo,
	AUTHORIZATION_HAS_PERMANENT_STORAGE, AUTHORIZATION_NOT_EXPIRED, BAD_DATA_SIZE,
	CHAIN_PERMANENT_CAP_REACHED, DEFAULT_MAX_BLOCK_TRANSACTIONS, DEFAULT_MAX_TRANSACTION_SIZE,
	PERMANENT_ALLOWANCE_EXCEEDED, PERMANENT_STORAGE_NEAR_CAP_PERCENT,
};
use crate::migrations::v1::OldTransactionInfo;
use bulletin_transaction_storage_primitives::cids::{CidConfig, HashingAlgorithm};
use codec::Encode;
use polkadot_sdk_frame::{
	deps::frame_support::{
		storage::unhashed,
		traits::{GetStorageVersion, Hooks, OnRuntimeUpgrade},
		BoundedVec,
	},
	hashing::blake2_256,
	prelude::*,
	runtime::prelude::weights::WeightMeter,
	testing_prelude::*,
	traits::StorageVersion,
};
use sp_transaction_storage_proof::{random_chunk, registration::build_proof, CHUNK_SIZE};

type Call = super::Call<Test>;
type Error = super::Error<Test>;

type Authorizations = super::Authorizations<Test>;
type BlockTransactions = super::BlockTransactions<Test>;
type PermanentStorageLedger = super::PermanentStorageLedger<Test>;
type PermanentStorageLedgerCursor = super::PermanentStorageLedgerCursor<Test>;
type PermanentStorageUsed = super::PermanentStorageUsed<Test>;
type RetentionPeriod = super::RetentionPeriod<Test>;
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
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), caller, 0, 2001));
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 2001,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2001,
				transactions: 1,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2001,
				transactions: 2,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2002,
				transactions: 0,
				transactions_allowance: 1,
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
				bytes_permanent: 0,
				bytes_allowance: 2002,
				transactions: 1,
				transactions_allowance: 1,
			}
		);
		assert_ok!(Into::<RuntimeCall>::into(call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);
		// Renew also uses the same preimage auth; it bumps `bytes_permanent` rather than `bytes`.
		let call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch(&call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 2000,
				bytes_allowance: 2002,
				transactions: 2,
				transactions_allowance: 1,
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
fn checks_proof_with_v2_shaped_transactions_entry() {
	use crate::migrations::v3::V2TransactionInfo;

	new_test_ext().execute_with(|| {
		let data = vec![0u8; 2000];

		run_to_block(1, || None);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data.clone()));
		run_to_block(2, || None);

		// Rewrite the freshly-written v3 entry at block 1 into the old v2 shape to
		// simulate the MBM window where historical `Transactions` entries have not yet
		// been rewritten by `MigrateV2ToV3`, while `check_proof` still executes every block.
		let txs_v3 = Transactions::get(1).expect("block 1 entry stored in v3 shape");
		let txs_v2: Vec<V2TransactionInfo> = txs_v3
			.into_iter()
			.map(|tx| V2TransactionInfo {
				chunk_root: tx.chunk_root,
				content_hash: tx.content_hash,
				hashing: tx.hashing,
				cid_codec: tx.cid_codec,
				size: tx.size,
				block_chunks: tx.block_chunks,
			})
			.collect();
		let bounded: BoundedVec<V2TransactionInfo, ConstU32<DEFAULT_MAX_BLOCK_TRANSACTIONS>> =
			txs_v2.try_into().expect("within bounds");
		unhashed::put_raw(&Transactions::hashed_key_for(1u64), &bounded.encode());

		// Direct decode as the live v3 type now fails.
		assert!(Transactions::get(1).is_none());

		run_to_block(11, || None);
		let parent_hash = System::parent_hash();
		let proof = build_proof(parent_hash.as_ref(), vec![data]).unwrap().unwrap();

		assert_ok!(TransactionStorage::check_proof(RuntimeOrigin::none(), proof));
		assert!(
			<super::ProofChecked<Test>>::get(),
			"check_proof should succeed by using transactions_at() on the v2-shaped entry",
		);
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 0,
			},
		);
		assert_ok!(TransactionStorage::validate_signed(&who, &call));
		run_to_block(11, || None);
		// Expired authorizations report as zero extent.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 0,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 0,
			},
		);
		assert!(!System::providers(&who).is_zero());

		let call = Call::store { data: vec![0; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 1000,
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 0,
			},
		);
		// Second consumption saturates at the cap.
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 2,
				transactions_allowance: 0,
			},
		);
		// Third consumption pushes `bytes` over the cap but still succeeds.
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 3000,
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 3,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: total_bytes,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: total_bytes,
				transactions: 6,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 1,
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1,
			},
			"Preimage authorization should be consumed"
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
			},
			"Account authorization should be consumed when no matching preimage auth"
		);
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(different_hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 1000,
				transactions: 0,
				transactions_allowance: 1,
			},
			"Unrelated preimage authorization should remain unchanged"
		);
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
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
			},
		);

		run_to_block(3, || None);

		// No preimage authorization exists for the content hash — renew uses account auth.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));

		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 2000,
				bytes_allowance: 4000,
				transactions: 2,
				transactions_allowance: 0,
			},
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
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 1,
			}
		);
		// Account auth was unaffected by the preimage authorize.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
			}
		);

		// Renew using signed transaction - should prefer preimage authorization
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));

		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 2000,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1,
			},
			"Preimage authorization should be consumed for renew"
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1,
			}
		);
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		// sha2 preimage consumed to cap; entry stays.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(sha2_hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 1,
				transactions_allowance: 1,
			}
		);
		// Blake2 authorization should remain unconsumed.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(blake2_hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 1,
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

		// Renew with the sha2 preimage auth still present — succeeds, accumulates on
		// `bytes_permanent` while leaving `bytes` (store-only) untouched.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch(&renew_call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(sha2_hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 2000,
				bytes_allowance: 2000,
				transactions: 2,
				transactions_allowance: 1,
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions: 2,
				transactions_allowance: 0,
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

		// Run v0→v1 (single-block) and v2→v3 (multi-block) migrations in sequence
		// to fully promote storage to the current `TransactionInfo` layout.
		// (The v1→v2 Authorization migration is unrelated to `Transactions` and
		// is skipped here.)
		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();
		drive_v2_to_v3_migration();
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(3));

		let txs1 = Transactions::get(1).expect("should decode after v0→v3 chain");
		assert_eq!(txs1.len(), 2);
		for tx in txs1.iter() {
			assert_eq!(tx.hashing, HashingAlgorithm::Blake2b256);
			assert_eq!(tx.cid_codec, 0x55);
			assert_eq!(tx.size, 2000);
			assert_eq!(tx.extrinsic_index, u32::MAX);
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

		// Run v0→v1 then v1→v2 to bring storage fully up to date.
		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();
		drive_v2_to_v3_migration();

		// Old entry promoted v0 → v1 → v2 (extrinsic_index = u32::MAX sentinel).
		let old_entry_after = Transactions::get(5).expect("should decode after v0→v2 chain");
		assert_eq!(old_entry_after.len(), 2);
		for tx in old_entry_after.iter() {
			assert_eq!(tx.extrinsic_index, u32::MAX);
		}

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
		assert_eq!(TransactionStorage::in_code_storage_version(), StorageVersion::new(3));

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
		// bytes(u64), bytes_permanent(u64), bytes_allowance(u64)
		let corrupted_auth = (0u32, 0u32, 0u64, 0u64, 0u64, 100u64); // all zero counters, bytes_allowance=0, expiration=100
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

/// Happy path for the hard-side invariants: a real `renew` keeps
/// `PermanentStorageUsed == Σ bytes_permanent == Σ ledger sizes`.
#[test]
fn try_state_passes_after_renew() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store_call = Call::store { data: vec![42u8; 2000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));
		assert_eq!(PermanentStorageUsed::get(), 2000);
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

/// `PermanentStorageUsed` desync from `Σ bytes_permanent` is caught.
#[test]
fn try_state_detects_permanent_used_mismatch_with_authorizations() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 2000;
		});
		// PermanentStorageUsed deliberately left at 0 — desync.
		assert_err!(
			TransactionStorage::do_try_state(System::block_number()),
			"PermanentStorageUsed != Σ bytes_permanent across Authorizations"
		);
	});
}

/// `PermanentStorageUsed` desync from `Σ ledger sizes` is caught.
#[test]
fn try_state_detects_permanent_used_mismatch_with_ledger() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 2000;
		});
		PermanentStorageUsed::put(2000);
		// Ledger deliberately empty — desync with the counter.
		assert_err!(
			TransactionStorage::do_try_state(System::block_number()),
			"PermanentStorageUsed != Σ size across PermanentStorageLedger entries"
		);
	});
}

/// A ledger entry whose block is below the cursor is caught.
#[test]
fn try_state_detects_ledger_entry_before_cursor() {
	new_test_ext().execute_with(|| {
		run_to_block(5, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 1000;
		});
		PermanentStorageUsed::put(1000);
		// Cursor at block 3, but a ledger entry at block 1 (before cursor) — invariant violation.
		PermanentStorageLedgerCursor::put(3u64);
		PermanentStorageLedger::mutate(1u64, |entries| {
			entries.try_push((AuthorizationScope::Account(who), 1000)).unwrap();
		});
		assert_err!(
			TransactionStorage::do_try_state(System::block_number()),
			"PermanentStorageLedger entry exists before cursor"
		);
	});
}

/// `PermanentStorageUsed` over `MaxPermanentStorageSize` is caught.
#[test]
fn try_state_detects_permanent_used_exceeds_chain_cap() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 2000;
		});
		PermanentStorageUsed::put(2000);
		PermanentStorageLedger::mutate(1u64, |entries| {
			entries.try_push((AuthorizationScope::Account(who), 2000)).unwrap();
		});
		// Counter and ledger consistent, but cap pushed below the counter.
		MaxPermanentStorageSize::set(&500);
		assert_err!(
			TransactionStorage::do_try_state(System::block_number()),
			"PermanentStorageUsed exceeds MaxPermanentStorageSize"
		);
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
				bytes_permanent: 0,
				bytes_allowance: 16,
				transactions: 1,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 3000,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 0,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 5000,
				transactions: 1,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 1500,
				transactions: 0,
				transactions_allowance: 0,
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
				bytes_permanent: 0,
				bytes_allowance: 3000,
				transactions: 0,
				transactions_allowance: 1,
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
				bytes_permanent: 0,
				bytes_allowance: 0,
				transactions: 0,
				transactions_allowance: 0,
			},
		);
	});
}

/// `refresh_account_authorization` only extends expiration — it does NOT reset any
/// consumed counters (`bytes`, `bytes_permanent`, `transactions`). In particular,
/// `bytes_permanent` MUST be left intact: permanent storage stays on chain across refresh
/// cycles, so its accounting cannot be erased. See the comment in `refresh_authorization`.
#[test]
fn refresh_does_not_reset_consumed_counters() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		// Authorize: all counters start at 0.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 0,
				transactions_allowance: 0,
			},
		);

		// Store: bumps `bytes` and `transactions`; `bytes_permanent` untouched.
		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 4000,
				transactions: 1,
				transactions_allowance: 0,
			},
			"store must advance `bytes` and `transactions`"
		);

		run_to_block(3, || None);

		// Renew: bumps `bytes_permanent` and `transactions`; `bytes` untouched.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 2000,
				bytes_allowance: 4000,
				transactions: 2,
				transactions_allowance: 0,
			},
			"renew must advance `bytes_permanent` and `transactions`"
		);

		// Refresh: all consumed counters preserved; only expiration moves.
		assert_ok!(TransactionStorage::refresh_account_authorization(RuntimeOrigin::root(), who));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 2000,
				bytes_allowance: 4000,
				transactions: 2,
				transactions_allowance: 0,
			},
			"refresh must not reset any consumed counters"
		);
	});
}

/// `authorize_account` on an expired-but-present entry must preserve `bytes_permanent`.
/// Resetting it would let a holder commit unbounded permanent storage by letting the
/// authorization expire and getting re-granted. Pairs with the
/// `remove_expired_account_authorization` guard.
#[test]
fn authorize_account_after_expiry_preserves_bytes_permanent() {
	new_test_ext().execute_with(|| {
		run_to_block(5, || None);
		let who = 1;

		// Authorize and seed `bytes_permanent = 2000` directly (simulates a past renew).
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 2000;
			// Force expiry without advancing blocks.
			auth.expiration = 1;
		});
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 0,
				transactions: 0,
				transactions_allowance: 0,
			},
			"expired authorization reports zero extent",
		);

		// Re-authorize: cap is re-granted, `bytes` resets to 0, `bytes_permanent` is preserved.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 1000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 2000,
				bytes_allowance: 1000,
				transactions: 0,
				transactions_allowance: 0,
			},
			"re-authorize after expiry must not zero `bytes_permanent`",
		);
	});
}

/// `remove_expired_account_authorization` must refuse while `bytes_permanent > 0`.
/// Removing the entry would orphan the (lazy) ledger drain — its decrement would have
/// nowhere to go.
#[test]
fn remove_expired_account_authorization_refuses_while_bytes_permanent_outstanding() {
	new_test_ext().execute_with(|| {
		run_to_block(5, || None);
		let who = 1;

		// Authorize, seed `bytes_permanent > 0`, force expiry.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 2000;
			auth.expiration = 1;
		});

		// Cannot be removed while permanent state is outstanding.
		assert_noop!(
			TransactionStorage::remove_expired_account_authorization(RuntimeOrigin::none(), who),
			Error::AuthorizationHasPermanentStorage,
		);

		// Once `bytes_permanent` is cleared (simulating a successful ledger drain),
		// removal succeeds.
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 0;
		});
		assert_ok!(TransactionStorage::remove_expired_account_authorization(
			RuntimeOrigin::none(),
			who,
		));
		assert!(!Authorizations::contains_key(AuthorizationScope::Account(who)));
	});
}

/// `remove_expired_account_authorization` must also be rejected at *validate* time when
/// `bytes_permanent > 0` — pool ingress should reject so the tx never reaches dispatch.
/// Pairs with the dispatch-time guard above.
#[test]
fn remove_expired_validate_rejects_while_bytes_permanent_outstanding() {
	new_test_ext().execute_with(|| {
		run_to_block(5, || None);
		let who = 1;

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 2000;
			auth.expiration = 1;
		});

		let remove_call = Call::remove_expired_account_authorization { who };
		assert_noop!(
			TransactionStorage::pre_dispatch(&remove_call),
			AUTHORIZATION_HAS_PERMANENT_STORAGE,
		);

		// Same guard applies to preimage scope.
		let content_hash = blake2_256(&[7u8; 8]);
		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::root(),
			content_hash,
			4000,
		));
		Authorizations::mutate(AuthorizationScope::Preimage(content_hash), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 2000;
			auth.expiration = 1;
		});
		let remove_preimage_call = Call::remove_expired_preimage_authorization { content_hash };
		assert_noop!(
			TransactionStorage::pre_dispatch(&remove_preimage_call),
			AUTHORIZATION_HAS_PERMANENT_STORAGE,
		);
	});
}

/// A successful renew bumps the chain-wide `PermanentStorageUsed` counter and appends a
/// `(scope, size)` entry to `PermanentStorageLedger` keyed by the current block, so the
/// lazy drain has something to decrement once retention elapses.
#[test]
fn renew_bumps_permanent_used_and_appends_to_ledger() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		assert_eq!(PermanentStorageUsed::get(), 0, "store must not bump permanent counter");
		assert!(PermanentStorageLedger::iter().next().is_none(), "store must not append to ledger");

		run_to_block(3, || None);

		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));
		assert_ok!(Into::<RuntimeCall>::into(renew_call).dispatch(RuntimeOrigin::none()));

		assert_eq!(
			PermanentStorageUsed::get(),
			2000,
			"renew must bump the chain-wide permanent counter",
		);
		let entries = PermanentStorageLedger::get(3);
		assert_eq!(
			entries.into_inner(),
			vec![(AuthorizationScope::Account(who), 2000)],
			"renew must append (scope, size) to the ledger at the current block",
		);
	});
}

/// `renew` rejects with [`PERMANENT_ALLOWANCE_EXCEEDED`] when the per-account hard cap is
/// reached: `bytes_permanent + size > bytes_allowance`. The chain-wide counter and ledger
/// must remain untouched.
#[test]
fn renew_rejects_when_per_account_allowance_exceeded() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		// Allowance is below `size`, so renew must reject. Store still succeeds because the
		// non-renew path is the soft side — overshoot is allowed (and demoted in priority by
		// `AllowanceBasedPriority`).
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 1500));
		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		run_to_block(3, || None);

		let renew_call = Call::renew { block: 1, index: 0 };
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&who, &renew_call),
			PERMANENT_ALLOWANCE_EXCEEDED,
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who).bytes_permanent,
			0,
			"rejected renew must not bump bytes_permanent",
		);
		assert_eq!(PermanentStorageUsed::get(), 0, "rejected renew must not bump chain counter");
		assert!(
			PermanentStorageLedger::iter().next().is_none(),
			"rejected renew must not append to ledger"
		);
	});
}

// ---- v1 → v2 multi-block migration tests ----

/// Drive the v1→v2 stepped migration to completion against the test externalities.
fn drive_v2_to_v3_migration() {
	use crate::migrations::v3::MigrateV2ToV3;
	use polkadot_sdk_frame::deps::frame_support::{
		migrations::SteppedMigration, weights::WeightMeter,
	};

	let mut meter = WeightMeter::new();
	let mut cursor: Option<<MigrateV2ToV3<Test> as SteppedMigration>::Cursor> = None;
	loop {
		cursor = MigrateV2ToV3::<Test>::step(cursor, &mut meter).expect("MBM step must not fail");
		if cursor.is_none() {
			break;
		}
	}
}

/// Insert a `BoundedVec<V2TransactionInfo, _>` raw blob under
/// `Transactions::hashed_key_for(block)`. `count` items are produced with synthetic field values.
fn insert_v2_format_transactions(block: u64, count: u32) {
	use crate::migrations::v3::V2TransactionInfo;
	use polkadot_sdk_frame::deps::sp_runtime::traits::{BlakeTwo256, Hash};

	let v2_txs: Vec<V2TransactionInfo> = (0..count)
		.map(|i| V2TransactionInfo {
			chunk_root: BlakeTwo256::hash(&[i as u8]),
			content_hash: BlakeTwo256::hash(&[i as u8 + 100]).into(),
			hashing: HashingAlgorithm::Blake2b256,
			cid_codec: 0x55,
			size: 2000,
			block_chunks: (i + 1) * 8,
		})
		.collect();
	let bounded: BoundedVec<V2TransactionInfo, ConstU32<DEFAULT_MAX_BLOCK_TRANSACTIONS>> =
		v2_txs.try_into().expect("within bounds");
	let key = Transactions::hashed_key_for(block);
	unhashed::put_raw(&key, &bounded.encode());
}

#[test]
fn migrate_v2_to_v3_sets_sentinel_for_existing_entries() {
	use crate::migrations::v3::MigrateV2ToV3;
	use polkadot_sdk_frame::deps::frame_support::{
		migrations::SteppedMigration, weights::WeightMeter,
	};
	new_test_ext().execute_with(|| {
		StorageVersion::new(2).put::<TransactionStorage>();
		insert_v2_format_transactions(1, 3);

		let mut meter = WeightMeter::new();
		let mut cursor: Option<<MigrateV2ToV3<Test> as SteppedMigration>::Cursor> = None;
		loop {
			cursor = MigrateV2ToV3::<Test>::step(cursor, &mut meter).expect("step should not fail");
			if cursor.is_none() {
				break;
			}
		}

		let txs = Transactions::get(1).expect("entry decodes as v2 after migration");
		assert_eq!(txs.len(), 3);
		for tx in txs.iter() {
			assert_eq!(tx.extrinsic_index, u32::MAX);
			assert_eq!(tx.size, 2000);
			assert_eq!(tx.hashing, HashingAlgorithm::Blake2b256);
			assert_eq!(tx.cid_codec, 0x55);
		}

		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(3));
	});
}

#[test]
fn migrate_v2_to_v3_resumes_across_steps() {
	use crate::{migrations::v3::MigrateV2ToV3, weights::WeightInfo};
	use polkadot_sdk_frame::deps::frame_support::{
		migrations::SteppedMigration, weights::WeightMeter,
	};
	new_test_ext().execute_with(|| {
		StorageVersion::new(2).put::<TransactionStorage>();
		for block in 1..=20u64 {
			insert_v2_format_transactions(block, 1);
		}

		let per_entry_weight = <Test as crate::Config>::WeightInfo::migrate_v2_to_v3_step();
		let mut total_steps = 0u32;
		let mut cursor: Option<<MigrateV2ToV3<Test> as SteppedMigration>::Cursor> = None;
		loop {
			let mut meter = WeightMeter::with_limit(per_entry_weight.saturating_mul(5));
			cursor = MigrateV2ToV3::<Test>::step(cursor, &mut meter).expect("step should not fail");
			total_steps += 1;
			if cursor.is_none() {
				break;
			}
			assert!(total_steps < 100, "migration must converge");
		}
		assert!(total_steps >= 2, "expected ≥2 step calls; got {total_steps}");

		for block in 1..=20u64 {
			let txs = Transactions::get(block).expect("entry decodes as v2");
			assert_eq!(txs.len(), 1);
			assert_eq!(txs[0].extrinsic_index, u32::MAX);
		}
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(3));
	});
}

#[test]
fn migrate_v2_to_v3_insufficient_weight_returns_err() {
	use crate::migrations::v3::MigrateV2ToV3;
	use polkadot_sdk_frame::deps::frame_support::{
		migrations::{SteppedMigration, SteppedMigrationError},
		weights::WeightMeter,
	};
	new_test_ext().execute_with(|| {
		StorageVersion::new(2).put::<TransactionStorage>();
		insert_v2_format_transactions(1, 1);

		let mut meter = WeightMeter::with_limit(Weight::zero());
		let res = MigrateV2ToV3::<Test>::step(None, &mut meter);
		assert!(
			matches!(res, Err(SteppedMigrationError::InsufficientWeight { .. })),
			"expected InsufficientWeight, got {:?}",
			res,
		);
	});
}

/// `renew` rejects with [`CHAIN_PERMANENT_CAP_REACHED`] when the chain-wide hard cap is
/// reached: `PermanentStorageUsed + size > MaxPermanentStorageSize`. Per-account state
/// must remain untouched.
#[test]
fn renew_rejects_when_chain_wide_cap_reached() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));

		// Lower the chain-wide cap below what a renewal would require.
		MaxPermanentStorageSize::set(&1000);

		run_to_block(3, || None);

		let renew_call = Call::renew { block: 1, index: 0 };
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&who, &renew_call),
			CHAIN_PERMANENT_CAP_REACHED,
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who).bytes_permanent,
			0,
			"rejected renew must not bump bytes_permanent",
		);
		assert_eq!(PermanentStorageUsed::get(), 0, "rejected renew must not bump chain counter");
		assert!(
			PermanentStorageLedger::iter().next().is_none(),
			"rejected renew must not append to ledger"
		);
	});
}

/// Lazy drain via `on_poll`: once `RetentionPeriod` blocks have elapsed since the renew,
/// the ledger entry is consumed and both `bytes_permanent` and `PermanentStorageUsed` are
/// decremented. `on_poll` is invoked directly here because `frame_system::run_to_block`
/// only fires `on_initialize`/`on_finalize`.
#[test]
fn on_poll_drains_ledger_after_retention_elapses() {
	new_test_ext().execute_with(|| {
		// Seed accounting as if a renew of 2000 bytes had landed at block 3, with retention
		// already elapsed by the current block. Avoids dispatching real store/renew (which
		// would require providing storage proofs in `on_finalize` past block 11).
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 2000;
		});
		PermanentStorageUsed::put(2000);
		PermanentStorageLedger::mutate(3u64, |entries| {
			entries.try_push((AuthorizationScope::Account(who), 2000)).unwrap();
		});

		// Retention is 10. At block 13, ledger bucket 3 is drainable (`13 - 3 = 10 >= 10`).
		System::set_block_number(13);
		let mut meter = WeightMeter::new();
		TransactionStorage::on_poll(13, &mut meter);

		assert_eq!(
			PermanentStorageUsed::get(),
			0,
			"on_poll must decrement chain-wide counter once retention elapses",
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who).bytes_permanent,
			0,
			"on_poll must decrement per-account bytes_permanent",
		);
		assert!(
			PermanentStorageLedger::iter().next().is_none(),
			"on_poll must clear drained ledger entries"
		);
		assert!(
			PermanentStorageLedgerCursor::get() >= 3,
			"cursor must have advanced past the drained block",
		);
	});
}

/// Mandatory `on_initialize` fallback: if the lazy `on_poll` drain has been skipped long
/// enough that the cursor is more than one `RetentionPeriod` behind, `on_initialize` runs
/// a bounded mandatory drain. Simulated here by stuffing the ledger directly and
/// rewinding the cursor.
#[test]
fn on_initialize_mandatory_drain_fires_when_cursor_lags() {
	new_test_ext().execute_with(|| {
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));

		// Seed accounting as if a renew of 2000 bytes had landed at block 1.
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 2000;
		});
		PermanentStorageUsed::put(2000);
		PermanentStorageLedger::mutate(1u64, |entries| {
			entries.try_push((AuthorizationScope::Account(who), 2000)).unwrap();
		});
		PermanentStorageLedgerCursor::put(1u64);

		// Retention is 10 blocks. Cursor at 1, threshold is `> RetentionPeriod`, so it must
		// advance to a block where `n - cursor > 10`, i.e. block 12 or later. Run to block 12
		// with cursor still at 1: `12 - 1 = 11 > 10` → mandatory fallback fires.
		run_to_block(12, || None);

		assert_eq!(
			PermanentStorageUsed::get(),
			0,
			"on_initialize fallback must drain when cursor lags by more than RetentionPeriod",
		);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who).bytes_permanent,
			0,
			"on_initialize fallback must decrement bytes_permanent",
		);
		assert!(
			PermanentStorageLedger::iter().next().is_none(),
			"on_initialize fallback must clear drained ledger entries",
		);
	});
}

/// Regression: when `RetentionPeriod` is 0 (e.g. the pallet's genesis didn't run, as in the
/// runtime-level `ExtBuilder` tests), the drain loop must terminate. The naive
/// `current_block - cursor < retention` saturates to 0 and stays `false` forever, burning
/// the entire weight meter on no-op cursor advances — and via `Executive::inherents_applied`,
/// that meter is the whole remaining block weight, so subsequent extrinsics end up
/// rejected with `ExhaustsResources`.
#[test]
fn drain_terminates_when_retention_period_is_zero() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		RetentionPeriod::put(0);

		let mut meter = WeightMeter::new();
		TransactionStorage::on_poll(1, &mut meter);

		// Tiny budget consumed (only a couple of cursor-advance iterations), nowhere close
		// to the meter limit. If the loop hadn't terminated, this would hit the limit.
		assert!(
			meter.consumed().all_lt(meter.limit()),
			"drain must terminate without exhausting the weight meter when retention is 0",
		);
	});
}

/// Renew emits `PermanentStorageUsedUpdated { used }` so off-chain capacity-planning
/// dashboards can track the chain-wide counter without polling storage.
#[test]
fn renew_emits_permanent_storage_used_updated() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);

		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));

		System::assert_has_event(RuntimeEvent::TransactionStorage(
			Event::PermanentStorageUsedUpdated { used: 2000 },
		));
	});
}

/// Drain emits `PermanentStorageUsedUpdated { used }` per decrement so off-chain consumers
/// see the counter walking back down as ledger entries age out.
#[test]
fn drain_emits_permanent_storage_used_updated() {
	new_test_ext().execute_with(|| {
		// Seed a renewed entry directly so we can run the drain in isolation.
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 4000));
		Authorizations::mutate(AuthorizationScope::Account(who), |maybe_auth| {
			let auth = maybe_auth.as_mut().expect("authorization present");
			auth.extent.bytes_permanent = 2000;
		});
		PermanentStorageUsed::put(2000);
		PermanentStorageLedger::mutate(3u64, |entries| {
			entries.try_push((AuthorizationScope::Account(who), 2000)).unwrap();
		});

		System::set_block_number(13);
		// Drain at block 13 (`13 - 3 = 10 >= retention`).
		let mut meter = WeightMeter::new();
		TransactionStorage::on_poll(13, &mut meter);

		System::assert_has_event(RuntimeEvent::TransactionStorage(
			Event::PermanentStorageUsedUpdated { used: 0 },
		));
	});
}

/// `PermanentStorageNearCap` fires once on the rising edge across the threshold and is
/// **not** re-emitted while still above the threshold. Decrementing back below and rising
/// again re-arms the signal.
#[test]
fn permanent_storage_near_cap_fires_on_rising_edge_only() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;

		// Cap = 1000; threshold = 1000 * 80 / 100 = 800.
		MaxPermanentStorageSize::set(&1000);

		// Generous per-account allowance so renews are only gated by the chain-wide cap.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, u64::MAX,));

		// Helper: store `size` bytes at the current block, advance one block, then renew it.
		// Captures the store block so the renew always points at the just-stored tx, not
		// some earlier one.
		let store_and_renew = |size: usize| {
			let store_block = System::block_number();
			let store_call = Call::store { data: vec![0u8; size] };
			assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
			assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
			run_to_block(store_block + 1, || None);
			let renew_call = Call::renew { block: store_block, index: 0 };
			assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));
		};

		// Step 1: 500 bytes (PermanentStorageUsed: 0 → 500). Below threshold; no near-cap.
		store_and_renew(500);
		assert_eq!(PermanentStorageUsed::get(), 500);
		let evs = System::events();
		assert!(!evs.iter().any(|r| matches!(
			r.event,
			RuntimeEvent::TransactionStorage(Event::PermanentStorageNearCap { .. })
		)));

		// Step 2: +400 bytes (500 → 900). Crosses 800 threshold → near-cap fires.
		System::reset_events();
		store_and_renew(400);
		assert_eq!(PermanentStorageUsed::get(), 900);
		System::assert_has_event(RuntimeEvent::TransactionStorage(
			Event::PermanentStorageNearCap { used: 900, cap: 1000 },
		));

		// Quick sanity check on the threshold formula matching the constant.
		assert_eq!(PERMANENT_STORAGE_NEAR_CAP_PERCENT, 80);
	});
}

#[cfg(feature = "try-runtime")]
#[test]
fn migrate_v2_to_v3_post_upgrade_allows_pruned_entries() {
	use crate::migrations::v3::MigrateV2ToV3;
	use polkadot_sdk_frame::deps::frame_support::migrations::SteppedMigration;

	new_test_ext().execute_with(|| {
		StorageVersion::new(2).put::<TransactionStorage>();
		insert_v2_format_transactions(1, 1);
		insert_v2_format_transactions(2, 1);
		insert_v2_format_transactions(3, 1);

		let state = MigrateV2ToV3::<Test>::pre_upgrade().expect("pre_upgrade succeeds");

		Transactions::remove(2u64);
		drive_v2_to_v3_migration();

		MigrateV2ToV3::<Test>::post_upgrade(state).expect("pruned entries are allowed");
	});
}

#[test]
fn migrate_v2_to_v3_skips_already_v3_entries() {
	use crate::migrations::v3::MigrateV2ToV3;
	use polkadot_sdk_frame::deps::{
		frame_support::{migrations::SteppedMigration, weights::WeightMeter},
		sp_runtime::traits::{BlakeTwo256, Hash},
	};
	new_test_ext().execute_with(|| {
		StorageVersion::new(2).put::<TransactionStorage>();

		// Block 1: pre-migration v1 layout.
		insert_v2_format_transactions(1, 1);
		// Block 2: already-v2 layout, written by current code paths.
		let v2_tx = TransactionInfo {
			chunk_root: BlakeTwo256::hash(&[42]),
			content_hash: BlakeTwo256::hash(&[43]).into(),
			hashing: HashingAlgorithm::Blake2b256,
			cid_codec: 0x55,
			size: 999,
			extrinsic_index: 7, // distinct from u32::MAX so we can detect corruption
			block_chunks: 4,
		};
		let v2_bounded: BoundedVec<TransactionInfo, ConstU32<DEFAULT_MAX_BLOCK_TRANSACTIONS>> =
			vec![v2_tx.clone()].try_into().unwrap();
		Transactions::insert(2u64, v2_bounded);

		// Drive migration to completion.
		let mut meter = WeightMeter::new();
		let mut cursor: Option<<MigrateV2ToV3<Test> as SteppedMigration>::Cursor> = None;
		loop {
			cursor = MigrateV2ToV3::<Test>::step(cursor, &mut meter).expect("step should not fail");
			if cursor.is_none() {
				break;
			}
		}

		// Block 1: migrated v1 → v2 with sentinel.
		let txs1 = Transactions::get(1).expect("decodes as v2");
		assert_eq!(txs1[0].extrinsic_index, u32::MAX);

		// Block 2: untouched — original `extrinsic_index = 7` preserved.
		let txs2 = Transactions::get(2).expect("decodes as v2");
		assert_eq!(txs2[0].extrinsic_index, 7);
		assert_eq!(txs2[0].size, 999);

		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(3));
	});
}

#[test]
fn transactions_at_decodes_v2_entry_with_sentinel() {
	new_test_ext().execute_with(|| {
		insert_v2_format_transactions(5, 2);

		// Direct `Transactions::get` cannot decode v2-shape bytes as the live (v3) layout.
		assert!(Transactions::get(5).is_none());

		let txs = TransactionStorage::transactions_at(5)
			.expect("v2 entries decode through transactions_at");
		assert_eq!(txs.len(), 2);
		for tx in txs.iter() {
			assert_eq!(tx.extrinsic_index, u32::MAX);
			assert_eq!(tx.size, 2000);
		}

		// The on-chain storage MUST be untouched: read-only API path does not write.
		assert!(Transactions::get(5).is_none());
	});
}

#[test]
fn transactions_at_handles_mixed_v2_and_v3_entries() {
	use polkadot_sdk_frame::deps::sp_runtime::traits::{BlakeTwo256, Hash};
	new_test_ext().execute_with(|| {
		// Block 1: pre-migration v2-shape (no `extrinsic_index`).
		insert_v2_format_transactions(1, 2);
		assert!(Transactions::get(1).is_none(), "v2 bytes do not decode as v3");

		// Block 2: live v3-shape entry — written by current code paths.
		let v3_tx = TransactionInfo {
			chunk_root: BlakeTwo256::hash(&[42]),
			content_hash: BlakeTwo256::hash(&[43]).into(),
			hashing: HashingAlgorithm::Blake2b256,
			cid_codec: 0x55,
			size: 999,
			extrinsic_index: 7,
			block_chunks: 4,
		};
		let v3_bounded: BoundedVec<TransactionInfo, ConstU32<DEFAULT_MAX_BLOCK_TRANSACTIONS>> =
			vec![v3_tx.clone()].try_into().unwrap();
		Transactions::insert(2u64, v3_bounded);

		// Empty: a block with no entry returns None.
		assert!(TransactionStorage::transactions_at(99).is_none());

		// Slow path: v2 entry promoted to v3 with sentinel.
		let txs1 = TransactionStorage::transactions_at(1).expect("v2 entry decodes");
		assert_eq!(txs1.len(), 2);
		for tx in txs1.iter() {
			assert_eq!(tx.extrinsic_index, u32::MAX);
			assert_eq!(tx.size, 2000);
		}

		// Fast path: v3 entry returned verbatim, real `extrinsic_index` preserved.
		let txs2 = TransactionStorage::transactions_at(2).expect("v3 entry decodes");
		assert_eq!(txs2.len(), 1);
		assert_eq!(txs2[0].extrinsic_index, 7);
		assert_eq!(txs2[0].size, 999);

		// Read-only contract: storage shapes are unchanged after the read.
		assert!(Transactions::get(1).is_none(), "v2 entry must remain v2-shape on disk");
		assert_eq!(
			Transactions::get(2)
				.expect("v3 entry still decodes")
				.into_iter()
				.next()
				.unwrap(),
			v3_tx,
			"v3 entry must be byte-identical pre/post read",
		);
	});
}

#[test]
fn store_records_extrinsic_index_in_transaction_info() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), vec![7u8; 500]));
		run_to_block(2, || None);

		let txs = TransactionStorage::transactions_at(1).expect("block 1 has transactions");
		assert_eq!(txs.len(), 1);
		// The store call ran at extrinsic_index 0 in block 1 (it's the only call).
		assert_eq!(txs[0].extrinsic_index, 0);
		assert_eq!(txs[0].size, 500);
	});
}

/// Test to make sure we can actually access everything we need for build the
/// output times for the runtime API.
#[test]
fn transaction_info_projects_into_upstream_runtime_api_type() {
	use bulletin_transaction_storage_primitives::cids::HashingAlgorithm as PalletHashingAlgorithm;
	use codec::{Decode, Encode};
	use polkadot_sdk_frame::deps::sp_runtime::traits::{BlakeTwo256, Hash};

	type ContentHash = [u8; 32];
	type CidCodec = u64;
	const RAW_CID_CODEC: CidCodec = 0x55;

	#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode, scale_info::TypeInfo)]
	enum HashingAlgorithm {
		Blake2b256,
		Sha2_256,
		Keccak256,
	}

	#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, scale_info::TypeInfo)]
	struct IndexedTransactionInfo {
		pub content_hash: ContentHash,
		pub size: u32,
		pub hashing: HashingAlgorithm,
		pub cid_codec: CidCodec,
		pub extrinsic_index: u32,
	}

	let tx = TransactionInfo {
		chunk_root: BlakeTwo256::hash(&[1]),
		content_hash: BlakeTwo256::hash(&[2]).into(),
		hashing: PalletHashingAlgorithm::Blake2b256,
		cid_codec: RAW_CID_CODEC,
		size: 500,
		extrinsic_index: 7,
		block_chunks: 4,
	};

	let projected = IndexedTransactionInfo {
		content_hash: tx.content_hash,
		size: tx.size,
		hashing: match tx.hashing {
			PalletHashingAlgorithm::Blake2b256 => HashingAlgorithm::Blake2b256,
			PalletHashingAlgorithm::Sha2_256 => HashingAlgorithm::Sha2_256,
			PalletHashingAlgorithm::Keccak256 => HashingAlgorithm::Keccak256,
			_ => panic!("unknown bulletin HashingAlgorithm variant"),
		},
		cid_codec: tx.cid_codec,
		extrinsic_index: tx.extrinsic_index,
	};

	assert_eq!(projected.content_hash, tx.content_hash);
	assert_eq!(projected.size, 500);
	assert_eq!(projected.hashing, HashingAlgorithm::Blake2b256);
	assert_eq!(projected.cid_codec, RAW_CID_CODEC);
	assert_eq!(projected.extrinsic_index, 7);
}
