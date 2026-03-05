// Copyright (C) Gautam Dhameja.
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

//! Tests for the Validator Set pallet.

#![cfg(test)]

use super::mock::{
	new_test_ext, next_block, next_session, AccountId, MockSessionKeys, RuntimeOrigin, Session,
	System, Test, ValidatorSet,
};
use polkadot_sdk_frame::{
	deps::sp_runtime::{testing::UintAuthorityId, Perbill},
	prelude::*,
	testing_prelude::*,
	traits::ValidatorRegistration,
};
use sp_staking::offence::{OffenceDetails, OnOffenceHandler};
use std::collections::HashSet;

type Error = super::Error<Test>;

fn validators() -> HashSet<AccountId> {
	ValidatorSet::validators().into_iter().collect()
}

fn active_validators() -> HashSet<AccountId> {
	Session::validators().into_iter().collect()
}

#[test]
fn initial_validators() {
	new_test_ext().execute_with(|| {
		assert_eq!(validators(), HashSet::from([1, 2, 3]));
		assert_eq!(active_validators(), HashSet::from([1, 2, 3]));
	});
}

#[test]
fn add_validator_updates_validators_list() {
	new_test_ext().execute_with(|| {
		assert_eq!(validators(), HashSet::from([1, 2, 3]));
		assert_ok!(ValidatorSet::add_validator(RuntimeOrigin::root(), 4));
		assert_eq!(validators(), HashSet::from([1, 2, 3, 4]));

		// add_validator should take effect in the session after next, provided the keys have been
		// set
		let val = 4;
		let mut key = MockSessionKeys { mock: UintAuthorityId(val) };
		assert_ok!(Session::set_keys(
			RuntimeOrigin::signed(val),
			key.clone(),
			key.create_ownership_proof(&val.encode()).unwrap().encode(),
		));
		assert_eq!(active_validators(), HashSet::from([1, 2, 3]));
		next_session();
		assert_eq!(active_validators(), HashSet::from([1, 2, 3]));
		next_session();
		assert_eq!(active_validators(), HashSet::from([1, 2, 3, 4]));
	});
}

#[test]
fn remove_validator_updates_validators_list() {
	new_test_ext().execute_with(|| {
		assert_ok!(ValidatorSet::remove_validator(RuntimeOrigin::root(), 2));
		assert_eq!(validators(), HashSet::from([1, 3]));
		// Add validator again
		assert_ok!(ValidatorSet::add_validator(RuntimeOrigin::root(), 2));
		assert_eq!(validators(), HashSet::from([1, 2, 3]));
	});
}

#[test]
fn add_validator_fails_with_invalid_origin() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			ValidatorSet::add_validator(RuntimeOrigin::signed(1), 4),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn remove_validator_fails_with_invalid_origin() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			ValidatorSet::remove_validator(RuntimeOrigin::signed(1), 4),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn duplicate_check() {
	new_test_ext().execute_with(|| {
		assert_ok!(ValidatorSet::add_validator(RuntimeOrigin::root(), 4));
		assert_eq!(validators(), HashSet::from([1, 2, 3, 4]));
		assert_noop!(ValidatorSet::add_validator(RuntimeOrigin::root(), 4), Error::Duplicate);
	});
}

#[test]
fn too_many_validators_check() {
	new_test_ext().execute_with(|| {
		assert_ok!(ValidatorSet::add_validator(RuntimeOrigin::root(), 4));
		assert_ok!(ValidatorSet::add_validator(RuntimeOrigin::root(), 5));
		assert_ok!(ValidatorSet::add_validator(RuntimeOrigin::root(), 6));
		assert_noop!(
			ValidatorSet::add_validator(RuntimeOrigin::root(), 7),
			Error::TooManyValidators
		);
	});
}

#[test]
fn not_a_validator_check() {
	new_test_ext().execute_with(|| {
		assert_ok!(ValidatorSet::remove_validator(RuntimeOrigin::root(), 3));
		assert_noop!(
			ValidatorSet::remove_validator(RuntimeOrigin::root(), 3),
			Error::NotAValidator
		);
	});
}

#[test]
fn remove_purges_keys_and_decs_providers() {
	new_test_ext().execute_with(|| {
		assert!(Session::is_registered(&3));
		assert!(!System::providers(&3).is_zero());
		assert_ok!(ValidatorSet::remove_validator(RuntimeOrigin::root(), 3));
		assert!(!Session::is_registered(&3));
		assert!(System::providers(&3).is_zero());
	});
}

#[test]
fn offender_disabled_and_removed() {
	new_test_ext().execute_with(|| {
		assert_eq!(validators(), HashSet::from([1, 2, 3]));
		ValidatorSet::on_offence(
			&[OffenceDetails { offender: (3, 3), reporters: vec![] }],
			&[Perbill::from_rational(1u32, 2u32)],
			0,
		);
		assert_eq!(validators(), HashSet::from([1, 2]));

		// The offender should be disabled for the rest of this session and the next session. The
		// removal should take effect by the session after next.
		assert_eq!(active_validators(), HashSet::from([1, 2, 3]));
		next_session();
		assert_eq!(active_validators(), HashSet::from([1, 2, 3]));
		next_session();
		assert_eq!(active_validators(), HashSet::from([1, 2]));
	});
}

#[test]
fn non_validator_cant_set_keys() {
	new_test_ext().execute_with(|| {
		assert_noop!(ValidatorSet::validate_set_keys(&4), InvalidTransaction::BadSigner);
		assert_noop!(ValidatorSet::pre_dispatch_set_keys(&4), InvalidTransaction::BadSigner);
	});
}

#[test]
fn set_keys_has_cooldown() {
	new_test_ext().execute_with(|| {
		assert_ok!(ValidatorSet::pre_dispatch_set_keys(&3));
		assert_noop!(ValidatorSet::pre_dispatch_set_keys(&3), InvalidTransaction::Future);
		next_block();
		assert_noop!(ValidatorSet::pre_dispatch_set_keys(&3), InvalidTransaction::Future);
		next_block();
		assert_ok!(ValidatorSet::pre_dispatch_set_keys(&3));
	});
}
