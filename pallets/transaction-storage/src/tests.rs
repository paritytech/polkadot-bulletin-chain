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
	extension::ValidateAuthorizedCalls,
	mock::{
		new_test_ext, run_to_block, RuntimeCall, RuntimeEvent, RuntimeOrigin, StoreRenewPriority,
		System, Test, TransactionStorage,
	},
	pallet::Origin,
	AllowedAuthorizers, AuthorizationExtent, AuthorizationOrigin, AuthorizationScope,
	AuthorizedCaller, AuthorizerBudget, EnsureAllowedAuthorizers, Event, Quota, TransactionInfo,
	AUTHORIZATION_NOT_EXHAUSTED, AUTHORIZATION_NOT_EXPIRED, AUTHORIZER_NOT_FOUND, BAD_DATA_SIZE,
	DEFAULT_MAX_BLOCK_TRANSACTIONS, DEFAULT_MAX_TRANSACTION_SIZE,
};

use crate::mock::RuntimeGenesisConfig;
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

fn test_budget(transactions: u32, bytes: u64) -> AuthorizerBudget<u64> {
	AuthorizerBudget {
		quota: Some(Quota { transactions, bytes }),
		valid_until: None,
		feeless: false,
	}
}

const MAX_DATA_SIZE: u32 = DEFAULT_MAX_TRANSACTION_SIZE;

mod runtime_api;

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
				extra: (),
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
				extra: (),
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
				extra: (),
				bytes_allowance: 2001,
				transactions: 2,
				transactions_allowance: 0,
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

		assert_ok!(TransactionStorage::apply_block_inherents(
			RuntimeOrigin::none(),
			Some(proof),
		));
		assert!(
			<super::ProofChecked<Test>>::get(),
			"apply_block_inherents proof step should succeed by using transactions_at() on the v2-shaped entry",
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
fn authorization_expires() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 0, 2000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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

/// `renew` accepts a content-hash variant of [`TransactionRef`] equivalently to
/// the position variant.

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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
				bytes_allowance: 1000,
				transactions: 0,
				transactions_allowance: 1,
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
				extra: (),
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
				extra: (),
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
		// bytes(u64), bytes_allowance(u64), extra(`()`, zero bytes)
		let corrupted_auth = (0u32, 0u32, 0u64, 0u64, 100u64); // all zero counters, bytes_allowance=0, expiration=100
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

// ---- ValidateAuthorizedCalls extension tests ----

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

		// Run ValidateAuthorizedCalls::validate - this should transform the origin
		let ext = ValidateAuthorizedCalls::<Test>::default();
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
		let ext2 = ValidateAuthorizedCalls::<Test>::default();
		assert_ok!(ext2.prepare(val, &origin_for_prepare, &call, &info, 0));

		// After prepare: 16 bytes used, entry at cap (not removed).
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			AuthorizationExtent {
				bytes: 16,
				extra: (),
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

		// Run ValidateAuthorizedCalls::validate
		let ext = ValidateAuthorizedCalls::<Test>::default();
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

		// Run ValidateAuthorizedCalls::validate - should pass through unchanged
		let ext = ValidateAuthorizedCalls::<Test>::default();
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
fn add_authorizer_inserts_overwrites_and_emits_event() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 42u64;
		// First insert.
		assert_ok!(TransactionStorage::add_authorizer(
			RuntimeOrigin::root(),
			who,
			test_budget(100, 1024),
		));
		assert_eq!(AllowedAuthorizers::<Test>::get(who).unwrap(), test_budget(100, 1024));
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::AuthorizerAdded { who }));

		// Second call with a different budget overwrites the first.
		assert_ok!(TransactionStorage::add_authorizer(
			RuntimeOrigin::root(),
			who,
			test_budget(200, 2048),
		));
		assert_eq!(AllowedAuthorizers::<Test>::get(who).unwrap(), test_budget(200, 2048));
	});
}

#[test]
fn remove_authorizer_removes_emits_event_and_ignores_absent() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 42u64;
		AllowedAuthorizers::<Test>::insert(who, test_budget(100, 1024));

		// Present → remove, emits event.
		assert_ok!(TransactionStorage::remove_authorizer(RuntimeOrigin::root(), who));
		assert!(!AllowedAuthorizers::<Test>::contains_key(who));
		System::assert_has_event(RuntimeEvent::TransactionStorage(Event::AuthorizerRemoved {
			who,
		}));

		// Absent → no-op success, no phantom event (state unchanged).
		let events_before = System::events().len();
		assert_ok!(TransactionStorage::remove_authorizer(RuntimeOrigin::root(), who));
		assert_eq!(System::events().len(), events_before);
	});
}

#[test]
fn add_remove_authorizer_reject_non_manager_origin() {
	new_test_ext().execute_with(|| {
		let who = 42u64;
		AllowedAuthorizers::<Test>::insert(who, test_budget(100, 1024));
		assert_noop!(
			TransactionStorage::add_authorizer(
				RuntimeOrigin::signed(1),
				who,
				test_budget(100, 1024),
			),
			DispatchError::BadOrigin,
		);
		assert_noop!(
			TransactionStorage::remove_authorizer(RuntimeOrigin::signed(1), who),
			DispatchError::BadOrigin,
		);
	});
}

#[test]
fn ensure_allowed_authorizers_origin_rules() {
	new_test_ext().execute_with(|| {
		let registered = 7u64;
		AllowedAuthorizers::<Test>::insert(registered, test_budget(100, 1024));
		// Signed by a registered account → accepted, returns the full
		// `AuthorizationOrigin` carrying the authorizer, `valid_until` and `feeless`
		// (both `None`/`false` for `test_budget`).
		assert_eq!(
			EnsureAllowedAuthorizers::<Test>::try_origin(RuntimeOrigin::signed(registered)).ok(),
			Some(Some(AuthorizationOrigin {
				authorizer: registered,
				valid_until: None,
				feeless: false,
			})),
		);
		// Signed by an unregistered account, Root, and None all rejected.
		assert!(EnsureAllowedAuthorizers::<Test>::try_origin(RuntimeOrigin::signed(99)).is_err());
		assert!(EnsureAllowedAuthorizers::<Test>::try_origin(RuntimeOrigin::root()).is_err());
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
			allowed_authorizers: vec![(1, 100, 1024), (2, 200, 2048)],
		},
	}
	.build_storage()
	.unwrap();
	TestExternalities::new(t).execute_with(|| {
		// Genesis authorizers default to `feeless: true`; root can re-add them
		// later to flip `feeless` or set a `valid_until`.
		let expected = |tx, by| AuthorizerBudget { feeless: true, ..test_budget(tx, by) };
		assert_eq!(AllowedAuthorizers::<Test>::iter().count(), 2);
		assert_eq!(AllowedAuthorizers::<Test>::get(1).unwrap(), expected(100, 1024));
		assert_eq!(AllowedAuthorizers::<Test>::get(2).unwrap(), expected(200, 2048));
	});
}

/// Verify that `ProvideInherent::create_inherent` actually emits the composite inherent call
/// when `PendingAutoRenewals` is non-empty, even with no storage proof in `InherentData`.
///
/// This is the direct test for "the block author will inject the inherent that drains pending
/// renewals" — if `create_inherent` ever stops returning the call when only renewals (and no
/// proof) are pending, the chain would panic at on_finalize without any test catching it.

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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
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
				extra: (),
				bytes_allowance: 0,
				transactions: 0,
				transactions_allowance: 0,
			},
		);
	});
}

// ---- v2 → v3 multi-block migration tests ----

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
			"expected InsufficientWeight, got {res:?}",
		);
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
			meta: (),
		};
		let v2_bounded: BoundedVec<TransactionInfo<()>, ConstU32<DEFAULT_MAX_BLOCK_TRANSACTIONS>> =
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

/// The no-op v3→v4 migration bumps the storage version 3 → 4 (the `AutoRenewals` reshape
/// moved to `pallet-bulletin-transaction-storage-renewal`).
#[test]
fn migrate_v3_to_v4_bumps_storage_version() {
	use crate::migrations::v4::MigrateV3ToV4;

	new_test_ext().execute_with(|| {
		StorageVersion::new(3).put::<TransactionStorage>();

		MigrateV3ToV4::<Test>::on_runtime_upgrade();

		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(4));
	});
}

/// Running the migration against state already at/beyond v4 must not downgrade the storage version.
#[test]
fn migrate_v3_to_v4_does_not_downgrade_storage_version() {
	use crate::migrations::v4::MigrateV3ToV4;

	new_test_ext().execute_with(|| {
		// Chain is already at v5 (e.g. summit).
		StorageVersion::new(5).put::<TransactionStorage>();

		MigrateV3ToV4::<Test>::on_runtime_upgrade();

		assert_eq!(
			TransactionStorage::on_chain_storage_version(),
			StorageVersion::new(5),
			"migration must not downgrade the storage version",
		);
	});
}

/// Stale `Transactions[block]` leftovers (block < current - RetentionPeriod) — e.g. from
/// a chain whose `RetentionPeriod` was previously longer — must be pruned by the v2→v3
/// migration rather than carried forward, otherwise `try_state` rejects them.
#[test]
fn migrate_v2_to_v3_prunes_stale_entries() {
	new_test_ext().execute_with(|| {
		StorageVersion::new(2).put::<TransactionStorage>();
		// Default `RetentionPeriod` in mock is 10. Run to block 50 so blocks 1..=39 are
		// "stale" (block < 50 - 10 = 40) and blocks 40..=50 are still in retention.
		System::set_block_number(50);

		insert_v2_format_transactions(1, 1); // stale
		insert_v2_format_transactions(20, 1); // stale
		insert_v2_format_transactions(40, 1); // in retention
		insert_v2_format_transactions(45, 1); // in retention

		drive_v2_to_v3_migration();

		assert!(Transactions::get(1).is_none(), "stale entry must be pruned");
		assert!(Transactions::get(20).is_none(), "stale entry must be pruned");
		assert!(Transactions::get(40).is_some(), "in-retention entry must be migrated");
		assert!(Transactions::get(45).is_some(), "in-retention entry must be migrated");
		assert_eq!(Transactions::get(40).unwrap()[0].extrinsic_index, u32::MAX);

		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(3));

		// `do_try_state` must accept the post-migration state (no stale entries left).
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
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
			meta: (),
		};
		let v3_bounded: BoundedVec<TransactionInfo<()>, ConstU32<DEFAULT_MAX_BLOCK_TRANSACTIONS>> =
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

// ---- Authorizer budget tests ----

#[test]
fn remove_exhausted_authorizer_removes_zero_budget_entries() {
	// Any of: both zero, transactions zero, bytes zero — all qualify as "exhausted".
	for (tx, bytes) in [(0, 0), (0, 1000), (100, 0)] {
		new_test_ext().execute_with(|| {
			run_to_block(1, || None);
			let who = 42u64;
			AllowedAuthorizers::<Test>::insert(who, test_budget(tx, bytes));

			let call = Call::remove_exhausted_authorizer { who };
			assert_ok!(TransactionStorage::pre_dispatch(&call));
			assert_ok!(
				TransactionStorage::remove_exhausted_authorizer(RuntimeOrigin::none(), who,)
			);
			assert!(!AllowedAuthorizers::<Test>::contains_key(who));
			System::assert_has_event(RuntimeEvent::TransactionStorage(
				Event::ExhaustedAuthorizerRemoved { who },
			));
		});
	}
}

#[test]
fn remove_exhausted_authorizer_rejects_when_not_removable() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);

		// Missing entry → AuthorizerNotFound (mempool + dispatch agree).
		let call = Call::remove_exhausted_authorizer { who: 99u64 };
		assert_noop!(TransactionStorage::pre_dispatch(&call), AUTHORIZER_NOT_FOUND);
		assert_noop!(
			TransactionStorage::remove_exhausted_authorizer(RuntimeOrigin::none(), 99u64),
			Error::AuthorizerNotFound,
		);

		// Present with non-zero budget + no expiry → AuthorizerBudgetNotExhausted.
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
fn add_authorizer_valid_until() {
	new_test_ext().execute_with(|| {
		run_to_block(5, || None);
		// `valid_until` is absolute and must be strictly in the future.
		let ok = AuthorizerBudget { valid_until: Some(25), ..test_budget(100, 1024) };
		assert_ok!(TransactionStorage::add_authorizer(RuntimeOrigin::root(), 42u64, ok));
		assert_eq!(AllowedAuthorizers::<Test>::get(42u64).unwrap().valid_until, Some(25));

		// Reject `== now` (expired immediately) and `< now` (already past).
		for t in [5, 0] {
			let bad = AuthorizerBudget { valid_until: Some(t), ..test_budget(100, 1024) };
			assert_noop!(
				TransactionStorage::add_authorizer(RuntimeOrigin::root(), 99u64, bad),
				Error::InvalidValidUntil,
			);
		}
		assert!(!AllowedAuthorizers::<Test>::contains_key(99u64));
	});
}

#[test]
fn expired_authorizer_cannot_authorize() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let authorizer = 10u64;
		// `valid_until = 6` → authorizes through block 5, expires at block 6.
		let budget = AuthorizerBudget { valid_until: Some(6), ..test_budget(100, 10_000) };
		assert_ok!(TransactionStorage::add_authorizer(RuntimeOrigin::root(), authorizer, budget,));
		let call = Call::authorize_account { who: 1, transactions: 1, bytes: 1000 };

		// Still valid at block 5.
		run_to_block(5, || None);
		assert_ok!(TransactionStorage::pre_dispatch_signed(&authorizer, &call));

		// Expired at block 6 (now >= valid_until).
		run_to_block(6, || None);
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&authorizer, &call),
			InvalidTransaction::BadSigner,
		);
	});
}

#[test]
fn remove_exhausted_authorizer_works_for_expired() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 42u64;
		// Has budget but expires at block 6.
		let budget = AuthorizerBudget { valid_until: Some(6), ..test_budget(100, 1000) };
		assert_ok!(TransactionStorage::add_authorizer(RuntimeOrigin::root(), who, budget));

		// Before expiry: removal rejected (budget present, not expired).
		assert_noop!(
			TransactionStorage::remove_exhausted_authorizer(RuntimeOrigin::none(), who),
			Error::AuthorizerBudgetNotExhausted,
		);

		// After expiry: removal succeeds.
		run_to_block(6, || None);
		assert_ok!(TransactionStorage::remove_exhausted_authorizer(RuntimeOrigin::none(), who));
		assert!(!AllowedAuthorizers::<Test>::contains_key(who));
		System::assert_has_event(RuntimeEvent::TransactionStorage(
			Event::ExhaustedAuthorizerRemoved { who },
		));
	});
}

#[test]
fn authorizer_budget_decrements_on_authorize() {
	// `authorize_account` consumes `(transactions, bytes)`; `authorize_preimage` is
	// equivalent to consuming `(1, max_size)`. Budget consumption happens inside
	// the dispatch body, so we dispatch via `Signed(authorizer)` directly.
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let authorizer = 10u64;
		AllowedAuthorizers::<Test>::insert(authorizer, test_budget(5, 10_000));

		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::signed(authorizer),
			1,
			2,
			4000,
		));
		assert_eq!(AllowedAuthorizers::<Test>::get(authorizer).unwrap(), test_budget(3, 6000));

		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::signed(authorizer),
			[0u8; 32],
			3000,
		));
		assert_eq!(AllowedAuthorizers::<Test>::get(authorizer).unwrap(), test_budget(2, 3000));
	});
}

#[test]
fn authorizer_budget_insufficient_rejects_without_writing() {
	// Both axes (transactions, bytes) gate independently; on rejection the budget
	// must be unchanged (`try_mutate` rolls back when the closure returns Err).
	let scenarios = [
		// (initial_budget, transactions, bytes)
		(test_budget(1, 10_000), 5, 1000),
		(test_budget(100, 500), 1, 1000),
	];
	for (initial, transactions, bytes) in scenarios {
		new_test_ext().execute_with(|| {
			run_to_block(1, || None);
			let authorizer = 10u64;
			AllowedAuthorizers::<Test>::insert(authorizer, initial.clone());
			assert_noop!(
				TransactionStorage::authorize_account(
					RuntimeOrigin::signed(authorizer),
					1,
					transactions,
					bytes,
				),
				Error::InsufficientAuthorizerBudget,
			);
			assert_eq!(AllowedAuthorizers::<Test>::get(authorizer).unwrap(), initial);
		});
	}
}

#[test]
fn root_bypasses_authorizer_budget() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		// Root can authorize without being in AllowedAuthorizers
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), 1, 10, 10_000));
		assert_eq!(
			TransactionStorage::account_authorization_extent(1),
			AuthorizationExtent {
				bytes: 0,
				extra: (),
				bytes_allowance: 10_000,
				transactions: 0,
				transactions_allowance: 10,
			},
		);
	});
}

#[test]
fn valid_until_clamps_authorization_expiry() {
	// Mock's `AuthorizationPeriod = 10`. An authorizer with `valid_until = 6`
	// (issued at block 1) should produce an authorization that expires at block
	// 6, not at block 11 — a grant cannot outlive its grantor.
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let authorizer = 10u64;
		AllowedAuthorizers::<Test>::insert(
			authorizer,
			AuthorizerBudget { valid_until: Some(6), ..test_budget(100, 100_000) },
		);

		let who = 1u64;
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::signed(authorizer),
			who,
			1,
			1000,
		));

		// At block 5, authorization still valid.
		run_to_block(5, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				extra: (),
				bytes_allowance: 1000,
				transactions: 0,
				transactions_allowance: 1,
			},
		);
		// At block 6 (= valid_until), authorization expired — extent reads as zeros.
		run_to_block(6, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent::default(),
		);
	});
}

#[test]
fn valid_until_clamps_refresh_authorization_expiry() {
	// Refresh must respect `valid_until` the same way `authorize` does: a refresh
	// issued by an authorizer with `valid_until = 6` cannot extend the grant past
	// block 6 even if `now + AuthorizationPeriod` would land later.
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let authorizer = 10u64;
		AllowedAuthorizers::<Test>::insert(
			authorizer,
			AuthorizerBudget { valid_until: Some(6), ..test_budget(100, 100_000) },
		);
		let who = 1u64;
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::signed(authorizer),
			who,
			1,
			1000,
		));

		// At block 5: refresh would naively set expiration = 5 + 10 = 15, but
		// must be clamped to authorizer's valid_until = 6.
		run_to_block(5, || None);
		assert_ok!(TransactionStorage::refresh_account_authorization(
			RuntimeOrigin::signed(authorizer),
			who,
		));
		// At block 6 the refreshed authorization is already expired.
		run_to_block(6, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent::default(),
		);
	});
}

#[test]
fn valid_until_beyond_default_period_does_not_clamp() {
	// `valid_until` past `now + AuthorizationPeriod` has no effect — the grant
	// gets the full default window.
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let authorizer = 11u64;
		// AuthorizationPeriod = 10, so default expiry from block 1 would be 11.
		// `valid_until = 100` is past that — no clamping.
		AllowedAuthorizers::<Test>::insert(
			authorizer,
			AuthorizerBudget { valid_until: Some(100), ..test_budget(100, 100_000) },
		);

		let who = 2u64;
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::signed(authorizer),
			who,
			1,
			1000,
		));

		// At block 10, still valid (default window covers 1..11).
		run_to_block(10, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 0,
				extra: (),
				bytes_allowance: 1000,
				transactions: 0,
				transactions_allowance: 1,
			},
		);
		// At block 11 (= default expiry), expired.
		run_to_block(11, || None);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent::default(),
		);
	});
}

#[test]
fn add_remove_authorizer_manages_system_providers() {
	// add → inc, re-add → no double-bump, remove → dec, re-remove → no underflow,
	// `remove_exhausted_authorizer` also dec's.
	new_test_ext().execute_with(|| {
		let who = 77u64;
		let providers_of = |a| frame_system::Account::<Test>::get(a).providers;
		assert_eq!(providers_of(who), 0);

		assert_ok!(TransactionStorage::add_authorizer(
			RuntimeOrigin::root(),
			who,
			test_budget(100, 1024),
		));
		assert_eq!(providers_of(who), 1);

		// Re-add must not double-bump.
		assert_ok!(TransactionStorage::add_authorizer(
			RuntimeOrigin::root(),
			who,
			test_budget(200, 2048),
		));
		assert_eq!(providers_of(who), 1);

		assert_ok!(TransactionStorage::remove_authorizer(RuntimeOrigin::root(), who));
		assert_eq!(providers_of(who), 0);

		// Re-remove must not underflow.
		assert_ok!(TransactionStorage::remove_authorizer(RuntimeOrigin::root(), who));
		assert_eq!(providers_of(who), 0);

		// `remove_exhausted_authorizer` path also dec's.
		AllowedAuthorizers::<Test>::insert(who, test_budget(0, 0));
		frame_system::Pallet::<Test>::inc_providers(&who);
		assert_ok!(TransactionStorage::remove_exhausted_authorizer(RuntimeOrigin::none(), who));
		assert_eq!(providers_of(who), 0);
	});
}

#[test]
fn feeless_if_reflects_authorizer_budget_feeless_flag() {
	// `true` only for `Signed(_)` origins whose `AllowedAuthorizers` entry has
	// `feeless = true`. Root / None / unregistered → not feeless via this flag.
	new_test_ext().execute_with(|| {
		let feeless = 7u64;
		let charged = 8u64;
		let unregistered = 9u64;
		AllowedAuthorizers::<Test>::insert(
			feeless,
			AuthorizerBudget { feeless: true, ..test_budget(10, 1000) },
		);
		AllowedAuthorizers::<Test>::insert(charged, test_budget(10, 1000));

		assert!(TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::signed(feeless)));
		assert!(!TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::signed(charged)));
		// Not in the allow-list → not feeless.
		assert!(!TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::signed(unregistered)));
		// Root / None → not feeless (the dispatch is feeless by other means, not
		// via this flag).
		assert!(!TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::root()));
		assert!(!TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::none()));
	});
}

#[test]
fn feeless_if_ignored_when_authorizer_budget_inactive() {
	// An inactive budget (exhausted on either axis, or past `valid_until`)
	// disables the `feeless` flag — so the dispatcher pays for the call instead
	// of spamming free dispatches that would fail downstream.
	new_test_ext().execute_with(|| {
		let who = 21u64;
		let insert = |b| AllowedAuthorizers::<Test>::insert(who, b);

		// Exhausted on both axes.
		insert(AuthorizerBudget { feeless: true, ..test_budget(0, 0) });
		assert!(!TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::signed(who)));

		// Exhausted on bytes axis only.
		insert(AuthorizerBudget { feeless: true, ..test_budget(0, 100) });
		assert!(!TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::signed(who)));

		// Exhausted on transactions axis only.
		insert(AuthorizerBudget { feeless: true, ..test_budget(5, 0) });
		assert!(!TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::signed(who)));

		// Sanity: budget with room on both axes is still feeless.
		insert(AuthorizerBudget { feeless: true, ..test_budget(5, 100) });
		assert!(TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::signed(who)));

		// `quota = None` (unlimited) is never exhausted.
		insert(AuthorizerBudget { quota: None, feeless: true, ..test_budget(0, 0) });
		assert!(TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::signed(who)));

		// Expired authorizer is inactive even with unlimited quota.
		// (`EnsureAllowedAuthorizers::try_origin` already rejects expired
		// authorizers, so this also exercises that rejection.)
		System::set_block_number(50);
		insert(AuthorizerBudget { quota: None, valid_until: Some(10), feeless: true });
		assert!(!TransactionStorage::is_feeless_authorizer(&RuntimeOrigin::signed(who)));
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
		meta: (),
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
