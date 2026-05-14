//! Tests for the pallet-side helpers backing
//! [`pallet_bulletin_transaction_storage_runtime_api::BulletinTransactionStorageApi`].

use crate::{
	mock::{
		new_test_ext, run_to_block, MaxPermanentStorageSize, RuntimeOrigin, Test,
		TransactionStorage,
	},
	DEFAULT_MAX_TRANSACTION_SIZE,
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
fn account_authorization_reports_raw_consumed_bytes() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 4000));

		// Drive the consumption through the signed pre_dispatch path so the
		// account's `bytes` / `bytes_permanent` counters actually update.
		let data = vec![0u8; 1000];
		let store_call = Call::store { data: data.clone() };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &store_call));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		let content_hash = blake2_256(&vec![0u8; 1000]);
		let renew_call = Call::renew_content_hash { content_hash };
		assert_ok!(TransactionStorage::pre_dispatch_signed(&who, &renew_call));
		assert_ok!(TransactionStorage::renew_content_hash(RuntimeOrigin::none(), content_hash));

		let summary = TransactionStorage::account_authorization(who).expect("active");
		assert_eq!(summary.bytes_allowance, 4000);
		assert_eq!(summary.bytes_used, 1000);
		assert_eq!(summary.bytes_permanent_used, 1000);
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
fn can_renew_matches_renew_extrinsic() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 1000];
		let content_hash = blake2_256(&data);

		// Unknown content hash → false.
		assert!(!TransactionStorage::can_renew(&who, content_hash));

		// Store it (no auth needed thanks to `RuntimeOrigin::none()`).
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		// Content exists but `who` has no authorization → false.
		assert!(!TransactionStorage::can_renew(&who, content_hash));

		// Grant a tight authorization — exactly one 1000-byte renewal fits.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 1000));

		// Happy path: can_renew agrees with renew_content_hash succeeding.
		assert!(TransactionStorage::can_renew(&who, content_hash));
		assert_ok!(TransactionStorage::renew_content_hash(RuntimeOrigin::none(), content_hash));

		// After consuming the only slot, per-account permanent capacity is exhausted.
		assert!(!TransactionStorage::can_renew(&who, content_hash));
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
		assert!(!TransactionStorage::can_renew(&who, content_hash));

		// Open the chain-wide cap; now valid.
		MaxPermanentStorageSize::set(&u64::MAX);
		assert!(TransactionStorage::can_renew(&who, content_hash));
	});
}

#[test]
fn can_enable_auto_renew_matches_extrinsic() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let data = vec![0u8; 1000];
		let content_hash = blake2_256(&data);

		// Missing content → false.
		assert!(!TransactionStorage::can_enable_auto_renew(&who, content_hash));

		// Authorize tightly: exactly one renewal's worth.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 1000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), data));
		run_to_block(2, || None);

		// Happy path.
		assert!(TransactionStorage::can_enable_auto_renew(&who, content_hash));
		assert_ok!(
			TransactionStorage::enable_auto_renew(RuntimeOrigin::signed(who), content_hash,)
		);

		// Already enabled → false.
		assert!(!TransactionStorage::can_enable_auto_renew(&who, content_hash));

		// A second user with no authorization can't enable.
		let other = 2;
		assert!(!TransactionStorage::can_enable_auto_renew(&other, content_hash));
	});
}

#[test]
fn can_enable_auto_renew_rejects_when_per_account_cap_exceeded() {
	new_test_ext().execute_with(|| {
		run_to_block(1, || None);
		let who = 1;
		let small_data = vec![0u8; 100];
		let small_hash = blake2_256(&small_data);
		let big_data = vec![1u8; 1500];
		let big_hash = blake2_256(&big_data);

		// 1000-byte allowance — fits the small blob but not the big one.
		assert_ok!(TransactionStorage::authorize_account(RuntimeOrigin::root(), who, 10, 1000));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), small_data));
		assert_ok!(TransactionStorage::store(RuntimeOrigin::none(), big_data));
		run_to_block(2, || None);

		assert!(TransactionStorage::can_enable_auto_renew(&who, small_hash));
		assert!(!TransactionStorage::can_enable_auto_renew(&who, big_hash));
	});
}
