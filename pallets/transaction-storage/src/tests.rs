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
		new_test_ext, run_to_block, set_period, RuntimeCall, RuntimeEvent, RuntimeOrigin,
		StoreRenewPriority, System, Test, TransactionStorage,
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

const MAX_DATA_SIZE: u32 = DEFAULT_MAX_TRANSACTION_SIZE;
const TX_BUDGET: u32 = 1_000;

/// Set the mock clock to period 1 (default starting point for most tests).
fn start_at_period_1() {
	set_period(1);
}

/// Build an `AuthorizationExtent` with the new five-field shape; the four positional
/// arguments mirror the old layout (`bytes, bytes_permanent, bytes_allowance,
/// transactions_used`) and `transactions_allowance` defaults to [`TX_BUDGET`].
fn extent(
	bytes: u64,
	bytes_permanent: u64,
	bytes_allowance: u64,
	transactions_used: u32,
) -> AuthorizationExtent {
	AuthorizationExtent {
		bytes,
		bytes_permanent,
		bytes_allowance,
		transactions_used,
		transactions_allowance: TX_BUDGET,
	}
}

/// Issue an account authorization for the current period using [`TX_BUDGET`] tx slots.
fn authorize_account_now(who: u64, bytes: u64) {
	let p = TransactionStorage::current_period();
	assert_ok!(TransactionStorage::authorize_account(
		RuntimeOrigin::root(),
		who,
		TX_BUDGET,
		bytes,
		p,
	));
}

/// Issue a preimage authorization for the current period.
fn authorize_preimage_now(hash: [u8; 32], max_size: u64) {
	let p = TransactionStorage::current_period();
	assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), hash, max_size, p));
}

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
		start_at_period_1();
		run_to_block(1, || None);
		let caller = 1;
		authorize_account_now(caller, 2001);
		assert_eq!(TransactionStorage::account_authorization_extent(caller), extent(0, 0, 2001, 0));
		let call = Call::store { data: vec![0u8; 2000] };
		// A caller without any Authorization entry is still rejected.
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&5, &call),
			InvalidTransaction::Payment,
		);
		assert_ok!(TransactionStorage::pre_dispatch_signed(&caller, &call));
		// Store of 2000 bytes within the 2001 cap: in-budget, tx_used = 1.
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			extent(2000, 0, 2001, 1),
		);
		// A second 2-byte store overshoots the byte cap → soft-cap demotion: bytes saturate
		// upward, but `transactions_used` is NOT incremented (over-budget calls are free of
		// tx-quota cost).
		let call = Call::store { data: vec![0u8; 2] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&caller, &call));
		assert_eq!(
			TransactionStorage::account_authorization_extent(caller),
			extent(2002, 0, 2001, 1),
		);
	});
}

#[test]
fn uses_preimage_authorization() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let data = vec![2; 2000];
		let hash = blake2_256(&data);
		// Preimage auth always uses transactions_allowance = 1.
		let p = TransactionStorage::current_period();
		assert_ok!(TransactionStorage::authorize_preimage(RuntimeOrigin::root(), hash, 2002, p));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent {
				bytes: 0,
				bytes_permanent: 0,
				bytes_allowance: 2002,
				transactions_used: 0,
				transactions_allowance: 1,
			}
		);
		// Data with a non-matching hash has no preimage auth → rejected.
		let call = Call::store { data: vec![1; 2000] };
		assert_noop!(TransactionStorage::pre_dispatch(&call), InvalidTransaction::Payment);
		// Matching data consumes allowance.
		let call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch(&call));
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 2002,
				transactions_used: 1,
				transactions_allowance: 1,
			}
		);
		assert_ok!(Into::<RuntimeCall>::into(call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);
		// Renew of the same preimage would push tx_used to 2, but transactions_allowance
		// is 1 — hard-cap rejects on the tx axis.
		let call = Call::renew { block: 1, index: 0 };
		assert_noop!(TransactionStorage::pre_dispatch(&call), InvalidTransaction::Payment);
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

		run_to_block(1, || None);
		for transaction in transactions.clone() {
			assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), transaction));
		}
		run_to_block(2, || None);

		let tx_infos = Transactions::get(1).unwrap();
		let total_chunks = TransactionInfo::total_chunks(&tx_infos);
		assert_eq!(expected_total_chunks, total_chunks);
		assert_eq!(9, tx_infos.len());

		for chunk_index in 0..total_chunks {
			let mut random_hash = [0u8; 32];
			random_hash[..8].copy_from_slice(&(chunk_index as u64).to_be_bytes());
			let selected_chunk_index = random_chunk(random_hash.as_ref(), total_chunks);
			assert_eq!(selected_chunk_index, chunk_index);

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
		assert_ok!(TransactionStorage::renew(RuntimeOrigin::none(), 1, 0,));
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
fn period_rollover_expires_current_grant() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		authorize_account_now(who, 2000);
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(0, 0, 2000, 0));

		// Validate succeeds within the granted period.
		let call = Call::store { data: vec![0; 2000] };
		assert_ok!(TransactionStorage::validate_signed(&who, &call));
		// validate_signed does not mutate counters.
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(0, 0, 2000, 0));

		// Advance to the next period — `current` slot is no longer live for that period.
		set_period(2);
		// `account_authorization_extent` now returns a zero extent.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent::default()
		);
		assert_noop!(
			TransactionStorage::validate_signed(&who, &call),
			InvalidTransaction::Payment,
		);
	});
}

#[test]
fn forward_claim_for_next_period_lives_in_next_slot() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		// Forward claim for period 2 only.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, TX_BUDGET, 2000, 2));

		// In period 1 the auth is "future" — extent is empty, store is rejected.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent::default()
		);
		let call = Call::store { data: vec![0; 100] };
		assert_noop!(
			TransactionStorage::validate_signed(&who, &call),
			InvalidTransaction::Payment,
		);

		// Roll forward into period 2 → prune-and-shift promotes `next` to `current`.
		set_period(2);
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(0, 0, 2000, 0));
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(100, 0, 2000, 1));
	});
}

#[test]
fn for_period_two_ahead_rejected() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		let who = 1;
		assert_noop!(
			TransactionStorage::authorize_account(RuntimeOrigin::root(), who, TX_BUDGET, 2000, 3),
			Error::InvalidPeriod,
		);
	});
}

#[test]
fn same_slot_re_authorize_adds_allowance() {
	// A second People-Chain claim for the same period should ADD to the existing slot's
	// `bytes_allowance` and `transactions_allowance`, NOT replace, and must preserve any
	// counters already used during the period.
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		authorize_account_now(who, 1000);
		// Spend a bit so we can verify "used" counters survive the second authorize.
		let call = Call::store { data: vec![0; 200] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(200, 0, 1000, 1));

		// Second authorize for the same period adds 500 bytes and 1000 tx slots.
		authorize_account_now(who, 500);
		// `bytes_allowance` is now 1500; tx allowance doubles to 2 * TX_BUDGET; used counters
		// preserved.
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent {
				bytes: 200,
				bytes_permanent: 0,
				bytes_allowance: 1500,
				transactions_used: 1,
				transactions_allowance: 2 * TX_BUDGET,
			},
		);
	});
}

#[test]
fn store_overshoot_does_not_consume_tx_quota() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		authorize_account_now(who, 2000);

		// First in-budget store: counters tick.
		let in_budget = Call::store { data: vec![0; 1500] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &in_budget));
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(1500, 0, 2000, 1));

		// Over-byte-cap store: bytes saturates upward, tx_used stays put.
		let over_byte = Call::store { data: vec![0; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &over_byte));
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(2500, 0, 2000, 1));
	});
}

#[test]
fn tx_quota_dos_shield_caps_in_budget_stores() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		// Generous bytes so the byte-cap doesn't gate; tight tx budget.
		let p = TransactionStorage::current_period();
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			3,
			1_000_000,
			p,
		));

		let one_byte = Call::store { data: vec![0; 1] };
		// First three 1-byte stores ride the boost tier and tick `transactions_used`.
		for expected_used in 1..=3 {
			assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &one_byte));
			let e = TransactionStorage::account_authorization_extent(who);
			assert_eq!(e.transactions_used, expected_used);
		}
		// Fourth store: tx-budget exhausted → falls to bottom tier. The store still
		// validates (soft cap) but `transactions_used` MUST NOT increment past 3.
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &one_byte));
		let e = TransactionStorage::account_authorization_extent(who);
		assert_eq!(e.transactions_used, 3, "over-tx-budget store must not consume a slot");
		assert_eq!(e.bytes, 4, "bytes counter still saturates upward");
	});
}

#[test]
fn renew_over_byte_cap_rejected() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		authorize_account_now(who, 3000);

		// Store 2000 bytes → bytes = 2000, combined = 2000.
		let store_call = Call::store { data: vec![0; 2000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);

		// Renew of the same 2000-byte item → combined would be 4000 > 3000 → REJECT.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&who, &renew_call),
			InvalidTransaction::Payment,
		);
		// Counters unchanged.
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(2000, 0, 3000, 1));
	});
}

#[test]
fn renew_over_tx_cap_rejected() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		// Plenty of bytes, exactly 1 tx slot.
		let p = TransactionStorage::current_period();
		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who,
			1,
			1_000_000,
			p,
		));

		// One store consumes the only tx slot.
		let store_call = Call::store { data: vec![0; 100] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		run_to_block(3, || None);

		// Renew would push transactions_used to 2 against an allowance of 1 → REJECT.
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&who, &renew_call),
			InvalidTransaction::Payment,
		);
	});
}

#[test]
fn signed_store_prefers_preimage_authorization_over_account() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];
		let content_hash = blake2_256(&data);

		authorize_account_now(who, 4000);
		authorize_preimage_now(content_hash, 2000);

		let call = Call::store { data: data.clone() };
		assert_ok!(TransactionStorage::validate_signed(&who, &call));
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));

		// Preimage auth was used; account untouched.
		assert_eq!(
			TransactionStorage::preimage_authorization_extent(content_hash),
			AuthorizationExtent {
				bytes: 2000,
				bytes_permanent: 0,
				bytes_allowance: 2000,
				transactions_used: 1,
				transactions_allowance: 1,
			}
		);
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(0, 0, 4000, 0));
	});
}

#[test]
fn signed_store_falls_back_to_account_authorization() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];
		let different_hash = blake2_256(&[0u8; 100]);

		authorize_account_now(who, 4000);
		authorize_preimage_now(different_hash, 1000);

		let call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(2000, 0, 4000, 1));
	});
}

#[test]
fn signed_renew_uses_account_authorization() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		let data = vec![42u8; 2000];

		authorize_account_now(who, 4000);
		let store_call = Call::store { data };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(Into::<RuntimeCall>::into(store_call).dispatch(RuntimeOrigin::none()));
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(2000, 0, 4000, 1));

		run_to_block(3, || None);
		let renew_call = Call::renew { block: 1, index: 0 };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(2000, 2000, 4000, 2));
	});
}

#[test]
fn store_with_cid_config_uses_custom_hashing() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let data = vec![42u8; 2000];

		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data.clone()));
		let default_info = BlockTransactions::get().last().unwrap().clone();
		assert_eq!(default_info.hashing, HashingAlgorithm::Blake2b256);
		assert_eq!(default_info.cid_codec, 0x55);

		let sha2_config = CidConfig { codec: 0x55, hashing: HashingAlgorithm::Sha2_256 };
		assert_ok!(TransactionStorage::store_with_cid_config(
			RuntimeOrigin::none(),
			sha2_config.clone(),
			data.clone(),
		));
		let sha2_info = BlockTransactions::get().last().unwrap().clone();
		assert_eq!(sha2_info.hashing, HashingAlgorithm::Sha2_256);
		assert_ne!(default_info.content_hash, sha2_info.content_hash);

		run_to_block(2, || None);
		let txs = Transactions::get(1).expect("transactions stored at block 1");
		assert_eq!(txs.len(), 2);
	});
}

#[test]
fn stores_various_sizes_with_account_authorization() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		let max = DEFAULT_MAX_TRANSACTION_SIZE as usize;
		let sizes: [usize; 6] = [1, 2000, max / 4, max / 2, max * 3 / 4, max];
		let total_bytes: u64 = sizes.iter().map(|s| *s as u64).sum();
		authorize_account_now(who, total_bytes);
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			extent(0, 0, total_bytes, 0),
		);

		for size in sizes {
			let call = Call::store { data: vec![0u8; size] };
			assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
			assert_ok!(Into::<RuntimeCall>::into(call).dispatch(RuntimeOrigin::none()));
		}

		// All in-budget; tx_used = 6 (one per size).
		assert_eq!(
			TransactionStorage::account_authorization_extent(who),
			extent(total_bytes, 0, total_bytes, 6),
		);
		assert!(Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert!(!System::providers(&who).is_zero());

		// Zero-size data must be rejected.
		let empty_call = Call::store { data: vec![] };
		assert_noop!(TransactionStorage::pre_dispatch_signed(&who, &empty_call), BAD_DATA_SIZE);
	});
}

#[test]
fn validate_signed_account_authorization_has_provides_tag() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1u64;
		authorize_account_now(who, 2000);

		let call = Call::store { data: vec![0u8; 2000] };
		// validate_signed does not consume.
		for _ in 0..2 {
			assert_ok!(TransactionStorage::validate_signed(&who, &call));
		}
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(0, 0, 2000, 0));

		let (vt, _) = TransactionStorage::validate_signed(&who, &call).unwrap();
		assert!(!vt.provides.is_empty());
		let (vt2, _) = TransactionStorage::validate_signed(&who, &call).unwrap();
		assert_eq!(vt.provides, vt2.provides);

		// Now pre_dispatch twice — saturate up.
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		// Second one is over-byte-cap, so tx_used stays at 1.
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_eq!(TransactionStorage::account_authorization_extent(who), extent(4000, 0, 2000, 1));

		// Signed and unsigned preimage paths must produce the same provides tag.
		let data = vec![0u8; 2000];
		let content_hash = blake2_256(&data);
		authorize_preimage_now(content_hash, 2000);
		authorize_account_now(who, 2000);

		let (signed_vt, _) = TransactionStorage::validate_signed(&who, &call).unwrap();
		let unsigned_vt = <TransactionStorage as ValidateUnsigned>::validate_unsigned(
			TransactionSource::External,
			&call,
		)
		.unwrap();
		assert_eq!(signed_vt.provides, unsigned_vt.provides);
	});
}

#[test]
fn remove_expired_account_authorization_after_rollover() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		assert!(System::providers(&who).is_zero());
		authorize_account_now(who, 2000);
		assert!(!System::providers(&who).is_zero());

		// While the grant is live, the call rejects with NotExpired.
		let remove_call = Call::remove_expired_account_authorization { who };
		assert_noop!(TransactionStorage::pre_dispatch(&remove_call), AUTHORIZATION_NOT_EXPIRED);

		// Roll past the granted period — `current` is dropped, `next` is empty: removal succeeds.
		set_period(2);
		assert_ok!(TransactionStorage::pre_dispatch(&remove_call));
		assert_ok!(Into::<RuntimeCall>::into(remove_call).dispatch(RuntimeOrigin::none()));
		System::assert_has_event(RuntimeEvent::TransactionStorage(
			Event::ExpiredAccountAuthorizationRemoved { who },
		));
		assert!(!Authorizations::contains_key(AuthorizationScope::Account(who)));
		assert!(System::providers(&who).is_zero());
	});
}

// ---- Migration tests (v0→v1 only; v1→v2 has structural type changes) ----

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
		StorageVersion::new(0).put::<TransactionStorage>();
		insert_old_format_transactions(1, 2);
		insert_old_format_transactions(2, 1);
		insert_old_format_transactions(3, 3);

		assert!(Transactions::get(1).is_none());
		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(1));

		let txs1 = Transactions::get(1).expect("decode");
		assert_eq!(txs1.len(), 2);
		for tx in txs1.iter() {
			assert_eq!(tx.hashing, HashingAlgorithm::Blake2b256);
			assert_eq!(tx.cid_codec, 0x55);
			assert_eq!(tx.size, 2000);
		}
		assert_eq!(Transactions::get(2).unwrap().len(), 1);
		assert_eq!(Transactions::get(3).unwrap().len(), 3);
	});
}

#[test]
fn migration_v1_idempotent() {
	new_test_ext().execute_with(|| {
		StorageVersion::new(0).put::<TransactionStorage>();
		insert_old_format_transactions(1, 1);
		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();
		let key = Transactions::hashed_key_for(1u64);
		let raw_after_first = unhashed::get_raw(&key).expect("raw bytes exist");

		crate::migrations::v1::MigrateV0ToV1::<Test>::on_runtime_upgrade();
		let raw_after_second = unhashed::get_raw(&key).expect("raw bytes still exist");
		assert_eq!(raw_after_first, raw_after_second);
	});
}

#[test]
fn migration_v2_translates_v1_authorization_to_two_slot() {
	use crate::{
		migrations::v2::{V1Authorization, V1AuthorizationExtent},
		PeriodGrant,
	};

	new_test_ext().execute_with(|| {
		// Pretend we're at v1.
		StorageVersion::new(1).put::<TransactionStorage>();
		// Park the mock clock at period 7 — the migration must stamp `current.period = 7`.
		set_period(7);

		// Write a v1 layout entry directly at the Authorizations storage key for account 42.
		let scope = AuthorizationScope::Account(42u64);
		let key = Authorizations::hashed_key_for(scope.clone());
		let v1 = V1Authorization::<u64> {
			extent: V1AuthorizationExtent { transactions: 5, bytes: 1234 },
			expiration: 999,
		};
		unhashed::put_raw(&key, &v1.encode());

		// Run the v1→v2 migration.
		crate::migrations::v2::MigrateV1ToV2::<Test>::on_runtime_upgrade();
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(2));

		// Entry now decodes as two-slot Authorization.
		let auth = Authorizations::get(scope).expect("decodes after migration");
		assert_eq!(
			auth.current,
			Some(PeriodGrant {
				period: 7,
				extent: AuthorizationExtent {
					bytes: 0,
					bytes_permanent: 0,
					bytes_allowance: 1234,
					transactions_used: 0,
					transactions_allowance: 5,
				},
			}),
		);
		assert!(auth.next.is_none());
	});
}

#[test]
fn migration_v2_drops_zero_quota_entries() {
	use crate::migrations::v2::{V1Authorization, V1AuthorizationExtent};

	new_test_ext().execute_with(|| {
		StorageVersion::new(1).put::<TransactionStorage>();
		set_period(1);

		// Entry with zero remaining bytes — must be dropped.
		let scope_a = AuthorizationScope::Account(1u64);
		let key_a = Authorizations::hashed_key_for(scope_a.clone());
		let v1_a = V1Authorization::<u64> {
			extent: V1AuthorizationExtent { transactions: 3, bytes: 0 },
			expiration: 100,
		};
		unhashed::put_raw(&key_a, &v1_a.encode());

		// Entry with zero remaining tx count — must also be dropped.
		let scope_b = AuthorizationScope::Account(2u64);
		let key_b = Authorizations::hashed_key_for(scope_b.clone());
		let v1_b = V1Authorization::<u64> {
			extent: V1AuthorizationExtent { transactions: 0, bytes: 100 },
			expiration: 100,
		};
		unhashed::put_raw(&key_b, &v1_b.encode());

		crate::migrations::v2::MigrateV1ToV2::<Test>::on_runtime_upgrade();
		assert!(!Authorizations::contains_key(scope_a));
		assert!(!Authorizations::contains_key(scope_b));
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
fn try_state_passes_with_active_authorizations() {
	new_test_ext().execute_with(|| {
		start_at_period_1();
		run_to_block(1, || None);
		let who = 1;
		authorize_account_now(who, 10_000);
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));

		let call = Call::store { data: vec![0; 1000] };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &call));
		assert_ok!(TransactionStorage::do_try_state(System::block_number()));
	});
}

#[test]
fn try_state_detects_zero_retention_period() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		RetentionPeriod::put(0u64);
		assert_err!(
			TransactionStorage::do_try_state(System::block_number()),
			"RetentionPeriod must not be zero"
		);
	});
}

// ---- ValidateStorageCalls extension tests ----

#[test]
fn ensure_authorized_extracts_custom_origin() {
	new_test_ext().execute_with(|| {
		let who: u64 = 42;

		let authorized_origin: RuntimeOrigin =
			Origin::<Test>::Authorized { who, scope: AuthorizationScope::Account(who) }.into();
		assert_eq!(
			TransactionStorage::ensure_authorized(authorized_origin),
			Ok(AuthorizedCaller::Signed { who, scope: AuthorizationScope::Account(who) }),
		);

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

		assert_eq!(
			TransactionStorage::ensure_authorized(RuntimeOrigin::root()),
			Ok(AuthorizedCaller::Root),
		);
		assert_eq!(
			TransactionStorage::ensure_authorized(RuntimeOrigin::none()),
			Ok(AuthorizedCaller::Unsigned),
		);
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
		start_at_period_1();
		run_to_block(1, || None);
		let caller = 1u64;
		let data = vec![0u8; 16];

		authorize_account_now(caller, 16);

		let call: RuntimeCall = Call::store { data }.into();
		let info: DispatchInfoOf<RuntimeCall> = Default::default();
		let origin = RuntimeOrigin::signed(caller);

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
		assert_eq!(valid_tx.priority, StoreRenewPriority::get());
		assert_eq!(val, Some(caller));

		let origin_for_prepare = transformed_origin.clone();
		assert_eq!(
			TransactionStorage::ensure_authorized(transformed_origin),
			Ok(AuthorizedCaller::Signed {
				who: caller,
				scope: AuthorizationScope::Account(caller)
			}),
		);

		let ext2 = ValidateStorageCalls::<Test>::default();
		assert_ok!(ext2.prepare(val, &origin_for_prepare, &call, &info, 0));

		assert_eq!(TransactionStorage::account_authorization_extent(caller), extent(16, 0, 16, 1));
	});
}
