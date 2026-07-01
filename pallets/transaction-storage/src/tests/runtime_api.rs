// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Tests for the pallet-side helpers backing
//! [`pallet_bulletin_transaction_storage_runtime_api::BulletinTransactionStorageApi`].

use crate::{
	mock::{
		new_test_ext, run_to_block, MaxPermanentStorageSize, RuntimeOrigin, Test,
		TransactionStorage,
	},
	TransactionRef, DEFAULT_MAX_TRANSACTION_SIZE,
};
use polkadot_sdk_frame::{hashing::blake2_256, testing_prelude::*};

type Call = crate::Call<Test>;

const MAX_DATA_SIZE: u32 = DEFAULT_MAX_TRANSACTION_SIZE;

#[test]
fn account_authorization_returns_none_when_missing_or_expired() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;

		// No authorization yet.
		assert_eq!(TransactionStorage::account_authorization(who), None);

		// Authorize, then advance past expiry.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 5, 4000));
		assert!(TransactionStorage::account_authorization(who).is_some());

		run_to_block(100, || None);
		assert_eq!(TransactionStorage::account_authorization(who), None);
	});
}

#[test]
fn can_store_matches_store_extrinsic() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;

		// No authorization → can_store false, and the extrinsic would be rejected
		// at validation time.
		assert!(!TransactionStorage::can_store(&who, 100));
		assert_noop!(
			TransactionStorage::pre_dispatch_signed(&who, &Call::store { data: vec![0u8; 100] }),
			InvalidTransaction::Payment,
		);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 4000));

		// Happy path.
		assert!(TransactionStorage::can_store(&who, 100));
		assert_ok!(TransactionStorage::pre_dispatch_signed(
			&who,
			&Call::store { data: vec![0u8; 100] }
		));

		// Oversize / zero-size rejected.
		assert!(!TransactionStorage::can_store(&who, 0));
		assert!(!TransactionStorage::can_store(&who, MAX_DATA_SIZE + 1));

		// `store` saturates over the allowance and uses the priority boost — it is
		// still valid, and can_store agrees.
		assert!(TransactionStorage::can_store(&who, MAX_DATA_SIZE));

		// Expired authorization → can_store false.
		run_to_block(100, || None);
		assert!(!TransactionStorage::can_store(&who, 100));
	});
}

#[test]
fn can_renew_rejects_when_chain_wide_cap_reached() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 1000];
		let content_hash = blake2_256(&data);

		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 10_000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		// Generous per-account cap, but chain-wide cap is too small.
		MaxPermanentStorageSize::set(&500);
		assert!(!TransactionStorage::can_renew(&who, &TransactionRef::ContentHash(content_hash)));

		// Open the chain-wide cap; now valid.
		MaxPermanentStorageSize::set(&u64::MAX);
		assert!(TransactionStorage::can_renew(&who, &TransactionRef::ContentHash(content_hash)));
	});
}

// `enable_auto_renew` shares the `is_renew = true` extension path with one-shot
// `renew`, so `can_renew` also predicts its validation outcome.
