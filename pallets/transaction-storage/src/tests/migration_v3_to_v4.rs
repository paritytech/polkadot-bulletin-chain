//! Coverage for the v3→v4 stepped migration.

use super::*;
use crate::{
	migrations::v4::{LegacyAuthorization, MigrateV3ToV4},
	mock::set_relay_now,
	weights::WeightInfo,
	AuthorizationExtent, AuthorizationScope,
};
use polkadot_sdk_frame::deps::frame_support::{
	migrations::{SteppedMigration, SteppedMigrationError},
	weights::WeightMeter,
};

type AuthorizationSlots = super::AuthorizationSlots;
type LegacyAuthorizations = crate::migrations::v4::Authorizations<Test>;

const RELAY_NOW: u32 = 1_000;

/// Set the mock relay block to a non-zero value and pin the on-chain
/// storage version to v3 so the migration has a real pre-migration state
/// to walk.
fn setup_v3() {
	set_relay_now(RELAY_NOW);
	StorageVersion::new(3).put::<TransactionStorage>();
}

/// Per-step weight from the mock `()` `WeightInfo`. Used to size meters.
fn step_weight() -> polkadot_sdk_frame::deps::sp_runtime::Weight {
	<Test as crate::Config>::WeightInfo::migrate_v3_to_v4_step()
}

/// Drive the v3→v4 stepped migration to completion against the test
/// externalities, mirroring the v2→v3 helper.
fn drive_migration() {
	let mut meter = WeightMeter::new();
	let mut cursor: Option<<MigrateV3ToV4<Test> as SteppedMigration>::Cursor> = None;
	loop {
		cursor =
			MigrateV3ToV4::<Test>::step(cursor, &mut meter).expect("v3->v4 step must not fail");
		if cursor.is_none() {
			break;
		}
	}
}

fn legacy_with(
	bytes_allowance: u64,
	transactions_allowance: u32,
	expiration: u64,
) -> LegacyAuthorization<u64> {
	LegacyAuthorization {
		extent: AuthorizationExtent {
			bytes: 0,
			bytes_permanent: 0,
			transactions: 0,
			bytes_allowance,
			transactions_allowance,
		},
		expiration,
	}
}

#[test]
fn translates_active_account_auth() {
	new_test_ext().execute_with(|| {
		setup_v3();
		let who = 1u64;
		let scope = AuthorizationScope::Account(who);
		let parachain_now = System::block_number();
		let mut entry = legacy_with(5_000, 10, parachain_now + 100);
		// Non-zero used counters to confirm the reset.
		entry.extent.bytes = 1234;
		entry.extent.bytes_permanent = 567;
		entry.extent.transactions = 3;
		LegacyAuthorizations::insert(&scope, entry);

		drive_migration();

		let slots = AuthorizationSlots::get(&scope).expect("slot exists").into_inner();
		assert_eq!(slots.len(), 1);
		let slot = &slots[0];
		assert_eq!(slot.starts_at, RELAY_NOW);
		assert_eq!(
			slot.expiration,
			RELAY_NOW + <Test as crate::Config>::DefaultAuthorizationWindow::get(),
		);
		assert_eq!(slot.extent.bytes_allowance, 5_000);
		assert_eq!(slot.extent.transactions_allowance, 10);
		assert_eq!(slot.extent.bytes, 0);
		assert_eq!(slot.extent.bytes_permanent, 0);
		assert_eq!(slot.extent.transactions, 0);
		assert_eq!(System::providers(&who), 1);
		assert!(LegacyAuthorizations::get(&scope).is_none());
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(4));
	});
}

#[test]
fn drops_expired_account_auth() {
	new_test_ext().execute_with(|| {
		setup_v3();
		let who = 1u64;
		let scope = AuthorizationScope::Account(who);
		let parachain_now = System::block_number();
		LegacyAuthorizations::insert(&scope, legacy_with(5_000, 10, parachain_now));

		drive_migration();

		assert!(AuthorizationSlots::get(&scope).is_none());
		assert!(LegacyAuthorizations::get(&scope).is_none());
		assert_eq!(System::providers(&who), 0);
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(4));
	});
}

#[test]
fn drops_empty_account_auth() {
	new_test_ext().execute_with(|| {
		setup_v3();
		let who = 1u64;
		let scope = AuthorizationScope::Account(who);
		let parachain_now = System::block_number();
		// `bytes_allowance == 0` ⇒ already-unusable in the v3 invariant.
		LegacyAuthorizations::insert(&scope, legacy_with(0, 10, parachain_now + 100));

		drive_migration();

		assert!(AuthorizationSlots::get(&scope).is_none());
		assert!(LegacyAuthorizations::get(&scope).is_none());
		assert_eq!(System::providers(&who), 0);
	});
}

#[test]
fn translates_preimage_auth() {
	new_test_ext().execute_with(|| {
		setup_v3();
		let hash = [42u8; 32];
		let scope = AuthorizationScope::Preimage(hash);
		let parachain_now = System::block_number();
		LegacyAuthorizations::insert(&scope, legacy_with(2_000, 1, parachain_now + 50));

		drive_migration();

		let slots = AuthorizationSlots::get(&scope).expect("slot exists").into_inner();
		assert_eq!(slots.len(), 1);
		assert_eq!(slots[0].extent.bytes_allowance, 2_000);
		assert!(LegacyAuthorizations::get(&scope).is_none());
		// Preimage scope does not bump providers — the storage owner is
		// the content hash, not an account.
	});
}

#[test]
fn resumes_across_steps() {
	new_test_ext().execute_with(|| {
		setup_v3();
		let parachain_now = System::block_number();
		for who in 0u64..10u64 {
			LegacyAuthorizations::insert(
				AuthorizationScope::Account(who),
				legacy_with(1_000, 1, parachain_now + 100),
			);
		}

		let per_step_budget = step_weight().saturating_mul(3);
		let mut total_steps = 0u32;
		let mut cursor: Option<<MigrateV3ToV4<Test> as SteppedMigration>::Cursor> = None;
		loop {
			let mut meter = WeightMeter::with_limit(per_step_budget);
			cursor = MigrateV3ToV4::<Test>::step(cursor, &mut meter).expect("step must not fail");
			total_steps += 1;
			if cursor.is_none() {
				break;
			}
			assert!(total_steps < 100, "migration must converge");
		}
		assert!(total_steps >= 2, "expected ≥2 step calls; got {total_steps}");

		let migrated = AuthorizationSlots::iter().count();
		assert_eq!(migrated, 10);
		assert!(LegacyAuthorizations::iter().next().is_none());
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(4));
	});
}

#[test]
fn bails_on_relay_now_zero() {
	new_test_ext().execute_with(|| {
		set_relay_now(0);
		StorageVersion::new(3).put::<TransactionStorage>();
		let scope = AuthorizationScope::Account(1u64);
		LegacyAuthorizations::insert(&scope, legacy_with(1_000, 1, System::block_number() + 50));

		let mut meter = WeightMeter::new();
		let result = MigrateV3ToV4::<Test>::step(None, &mut meter);
		assert!(matches!(result, Err(SteppedMigrationError::Failed)));
		// Storage untouched.
		assert!(LegacyAuthorizations::get(&scope).is_some());
		assert!(AuthorizationSlots::get(&scope).is_none());
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(3));
	});
}

#[test]
fn version_bumps_only_after_drain() {
	new_test_ext().execute_with(|| {
		setup_v3();
		let scope = AuthorizationScope::Account(1u64);
		LegacyAuthorizations::insert(&scope, legacy_with(1_000, 1, System::block_number() + 50));

		// Step 1: budget for exactly one inner-loop iteration. The single
		// legacy entry translates; the loop breaks before the iter is
		// re-checked, so the version stays at 3 and the cursor is `Some`.
		let mut meter = WeightMeter::with_limit(step_weight());
		let cursor =
			MigrateV3ToV4::<Test>::step(None, &mut meter).expect("first step must not fail");
		assert!(cursor.is_some(), "cursor still points at last-processed scope");
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(3));
		assert!(AuthorizationSlots::get(&scope).is_some());

		// Step 2: with the legacy map empty, the iter exhausts and the
		// version bumps to 4.
		let mut meter = WeightMeter::with_limit(step_weight());
		let cursor =
			MigrateV3ToV4::<Test>::step(cursor, &mut meter).expect("second step must not fail");
		assert!(cursor.is_none());
		assert_eq!(TransactionStorage::on_chain_storage_version(), StorageVersion::new(4));
	});
}

#[test]
fn clears_legacy_storage_prefix() {
	new_test_ext().execute_with(|| {
		setup_v3();
		let parachain_now = System::block_number();
		for who in 0u64..3u64 {
			LegacyAuthorizations::insert(
				AuthorizationScope::Account(who),
				legacy_with(1_000, 1, parachain_now + 50),
			);
		}
		drive_migration();
		assert!(LegacyAuthorizations::iter().next().is_none());
	});
}
