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

//! Tests for the bulletin-utility pallet.

use crate::mock::*;
use polkadot_sdk_frame::deps::{
	frame_support::{assert_ok, dispatch::CheckIfFeeless},
	frame_system,
};

const SIGNER: u64 = 1;

fn feeless() -> RuntimeCall {
	RuntimeCall::Dummy(dummy::Call::feeless_noop {})
}

fn paid() -> RuntimeCall {
	RuntimeCall::Dummy(dummy::Call::paid_noop {})
}

fn origin() -> RuntimeOrigin {
	RuntimeOrigin::signed(SIGNER)
}

fn batch(calls: Vec<RuntimeCall>) -> RuntimeCall {
	RuntimeCall::BulletinUtility(crate::Call::batch { calls })
}

fn batch_all(calls: Vec<RuntimeCall>) -> RuntimeCall {
	RuntimeCall::BulletinUtility(crate::Call::batch_all { calls })
}

fn force_batch(calls: Vec<RuntimeCall>) -> RuntimeCall {
	RuntimeCall::BulletinUtility(crate::Call::force_batch { calls })
}

#[test]
fn batch_of_feeless_calls_is_feeless() {
	for build in [batch, batch_all, force_batch] {
		assert!(build(vec![feeless(), feeless()]).is_feeless(&origin()));
	}
}

#[test]
fn batch_with_a_paid_call_is_not_feeless() {
	for build in [batch, batch_all, force_batch] {
		assert!(!build(vec![feeless(), paid()]).is_feeless(&origin()));
		assert!(!build(vec![paid()]).is_feeless(&origin()));
	}
}

#[test]
fn empty_batch_is_not_feeless() {
	for build in [batch, batch_all, force_batch] {
		assert!(!build(vec![]).is_feeless(&origin()));
	}
}

#[test]
fn nested_feeless_batch_is_feeless() {
	let inner = batch(vec![feeless(), feeless()]);
	assert!(batch(vec![inner]).is_feeless(&origin()));

	let inner_paid = batch(vec![feeless(), paid()]);
	assert!(!batch(vec![inner_paid]).is_feeless(&origin()));
}

#[test]
fn batch_delegates_to_pallet_utility() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		assert_ok!(BulletinUtility::batch(origin(), vec![feeless(), feeless()]));
		System::assert_has_event(RuntimeEvent::Utility(pallet_utility::Event::BatchCompleted));
	});
}

#[test]
fn batch_all_delegates_to_pallet_utility() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		assert_ok!(BulletinUtility::batch_all(origin(), vec![feeless(), feeless()]));
		System::assert_has_event(RuntimeEvent::Utility(pallet_utility::Event::BatchCompleted));
	});
}

#[test]
fn force_batch_delegates_to_pallet_utility() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		assert_ok!(BulletinUtility::force_batch(origin(), vec![feeless(), feeless()]));
		System::assert_has_event(RuntimeEvent::Utility(pallet_utility::Event::BatchCompleted));
	});
}

#[test]
fn batch_all_blocks_nested_batch_all() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let inner = batch_all(vec![feeless()]);
		let err = BulletinUtility::batch_all(origin(), vec![inner]).unwrap_err().error;
		assert_eq!(err, frame_system::Error::<Test>::CallFiltered.into());
	});
}

#[test]
fn batch_allows_nesting() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let inner = batch(vec![feeless()]);
		assert_ok!(BulletinUtility::batch(origin(), vec![inner]));
	});
}
