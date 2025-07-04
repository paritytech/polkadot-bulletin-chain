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

//! Tests for relayer set pallet.

#![cfg(test)]

use super::mock::{new_test_ext, next_block, AccountId, RelayerSet, RuntimeOrigin, System, Test};
use polkadot_sdk_frame::{prelude::*, testing_prelude::*};
use std::collections::HashSet;

type Error = super::Error<Test>;

fn relayers() -> HashSet<AccountId> {
	RelayerSet::relayers().into_iter().collect()
}

#[test]
fn initial_relayers() {
	new_test_ext().execute_with(|| {
		assert_eq!(relayers(), HashSet::from([1, 2, 3]));
	});
}

#[test]
fn add_relayer_updates_relayers_list() {
	new_test_ext().execute_with(|| {
		assert_eq!(relayers(), HashSet::from([1, 2, 3]));
		assert_ok!(RelayerSet::add_relayer(RuntimeOrigin::root(), 4));
		assert_eq!(relayers(), HashSet::from([1, 2, 3, 4]));
	});
}

#[test]
fn remove_relayer_updates_relayers_list() {
	new_test_ext().execute_with(|| {
		assert_ok!(RelayerSet::remove_relayer(RuntimeOrigin::root(), 2));
		assert_eq!(relayers(), HashSet::from([1, 3]));
		// Add relayer again
		assert_ok!(RelayerSet::add_relayer(RuntimeOrigin::root(), 2));
		assert_eq!(relayers(), HashSet::from([1, 2, 3]));
	});
}

#[test]
fn add_relayer_fails_with_invalid_origin() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			RelayerSet::add_relayer(RuntimeOrigin::signed(1), 4),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn remove_relayer_fails_with_invalid_origin() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			RelayerSet::remove_relayer(RuntimeOrigin::signed(1), 4),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn duplicate_check() {
	new_test_ext().execute_with(|| {
		assert_ok!(RelayerSet::add_relayer(RuntimeOrigin::root(), 4));
		assert_eq!(relayers(), HashSet::from([1, 2, 3, 4]));
		assert_noop!(RelayerSet::add_relayer(RuntimeOrigin::root(), 4), Error::Duplicate);
	});
}

#[test]
fn not_a_relayer_check() {
	new_test_ext().execute_with(|| {
		assert_ok!(RelayerSet::remove_relayer(RuntimeOrigin::root(), 3));
		assert_noop!(RelayerSet::remove_relayer(RuntimeOrigin::root(), 3), Error::NotARelayer);
	});
}

#[test]
fn remove_decs_providers() {
	new_test_ext().execute_with(|| {
		assert!(!System::providers(&3).is_zero());
		assert_ok!(RelayerSet::remove_relayer(RuntimeOrigin::root(), 3));
		assert!(System::providers(&3).is_zero());
	});
}

#[test]
fn bridge_tx_validation() {
	new_test_ext().execute_with(|| {
		assert_ok!(RelayerSet::validate_bridge_tx(&3));
		assert_noop!(RelayerSet::validate_bridge_tx(&4), InvalidTransaction::BadSigner);
		RelayerSet::post_dispatch_failed_bridge_tx(&3);
		assert_noop!(RelayerSet::validate_bridge_tx(&3), InvalidTransaction::Future);
		next_block();
		assert_noop!(RelayerSet::validate_bridge_tx(&3), InvalidTransaction::Future);
		next_block();
		assert_ok!(RelayerSet::validate_bridge_tx(&3));
	});
}
