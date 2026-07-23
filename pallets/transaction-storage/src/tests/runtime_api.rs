// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Tests for the pallet-side helpers backing
//! [`pallet_bulletin_transaction_storage_runtime_api::BulletinTransactionStorageApi`].
//! `account_authorization` and `can_renew` moved to
//! `pallet-bulletin-transaction-storage-renewal` and are tested there.

use crate::{
	mock::{new_test_ext, run_to_block, RuntimeOrigin, Test, TransactionStorage},
	DEFAULT_MAX_TRANSACTION_SIZE,
};
use polkadot_sdk_frame::testing_prelude::*;

type Call = crate::Call<Test>;

const MAX_DATA_SIZE: u32 = DEFAULT_MAX_TRANSACTION_SIZE;

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
