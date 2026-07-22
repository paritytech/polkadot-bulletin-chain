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

//! Tests for `NoCurrency` and the utility call inspectors.

use crate::{inspect_utility_wrapper, mock::*, utility_inner_calls, NoCurrency};
use polkadot_sdk_frame::{
	deps::frame_system,
	prelude::{
		fungible::{Inspect, InspectHold, Unbalanced, UnbalancedHold},
		DepositConsequence, Fortitude, Preservation, Provenance, Weight, WithdrawConsequence,
	},
};

type Cur = NoCurrency<u64, ()>;

#[test]
fn no_currency_fungible_is_zero_and_noop() {
	assert_eq!(<Cur as Inspect<u64>>::total_issuance(), 0);
	assert_eq!(<Cur as Inspect<u64>>::minimum_balance(), 0);
	assert_eq!(<Cur as Inspect<u64>>::total_balance(&1), 0);
	assert_eq!(<Cur as Inspect<u64>>::balance(&1), 0);
	assert_eq!(
		<Cur as Inspect<u64>>::reducible_balance(&1, Preservation::Expendable, Fortitude::Force),
		0
	);
	assert_eq!(
		<Cur as Inspect<u64>>::can_deposit(&1, 100, Provenance::Minted),
		DepositConsequence::Success
	);
	assert_eq!(<Cur as Inspect<u64>>::can_withdraw(&1, 100), WithdrawConsequence::Success);

	assert_eq!(<Cur as Unbalanced<u64>>::write_balance(&1, 100), Ok(None));
	<Cur as Unbalanced<u64>>::set_total_issuance(100);
	assert_eq!(<Cur as Inspect<u64>>::total_issuance(), 0);

	assert_eq!(<Cur as InspectHold<u64>>::total_balance_on_hold(&1), 0);
	assert_eq!(<Cur as InspectHold<u64>>::balance_on_hold(&(), &1), 0);
	assert_eq!(<Cur as UnbalancedHold<u64>>::set_balance_on_hold(&(), &1, 100), Ok(()));
	assert_eq!(<Cur as InspectHold<u64>>::balance_on_hold(&(), &1), 0);
}

fn remark() -> RuntimeCall {
	RuntimeCall::System(frame_system::Call::remark { remark: vec![] })
}

fn root_origin() -> Box<OriginCaller> {
	Box::new(OriginCaller::system(frame_system::RawOrigin::Root))
}

#[test]
fn utility_batch_variants_return_all_inner_calls() {
	let calls = vec![remark(), remark()];
	for call in [
		pallet_utility::Call::<Test>::batch { calls: calls.clone() },
		pallet_utility::Call::<Test>::batch_all { calls: calls.clone() },
		pallet_utility::Call::<Test>::force_batch { calls: calls.clone() },
	] {
		assert_eq!(inspect_utility_wrapper(&call), Some(calls.iter().collect()));
	}
}

#[test]
fn utility_single_call_variants_return_inner_call() {
	let inner = remark();
	let variants: Vec<pallet_utility::Call<Test>> = vec![
		pallet_utility::Call::as_derivative { index: 0, call: Box::new(inner.clone()) },
		pallet_utility::Call::dispatch_as {
			as_origin: root_origin(),
			call: Box::new(inner.clone()),
		},
		pallet_utility::Call::with_weight { call: Box::new(inner.clone()), weight: Weight::zero() },
		pallet_utility::Call::dispatch_as_fallible {
			as_origin: root_origin(),
			call: Box::new(inner.clone()),
		},
	];
	for call in &variants {
		assert_eq!(inspect_utility_wrapper(call), Some(vec![&inner]));
	}
}

#[test]
fn utility_if_else_returns_both_branches() {
	let main = remark();
	let fallback = RuntimeCall::System(frame_system::Call::remark_with_event { remark: vec![1] });
	let call = pallet_utility::Call::<Test>::if_else {
		main: Box::new(main.clone()),
		fallback: Box::new(fallback.clone()),
	};
	assert_eq!(inspect_utility_wrapper(&call), Some(vec![&main, &fallback]));
}

#[test]
fn utility_empty_batch_is_not_a_wrapper() {
	for call in [
		pallet_utility::Call::<Test>::batch { calls: vec![] },
		pallet_utility::Call::<Test>::batch_all { calls: vec![] },
		pallet_utility::Call::<Test>::force_batch { calls: vec![] },
	] {
		assert!(utility_inner_calls(&call).is_empty());
		assert_eq!(inspect_utility_wrapper(&call), None);
	}
}

#[test]
fn inspectors_do_not_recurse() {
	let inner_batch = RuntimeCall::Utility(pallet_utility::Call::batch { calls: vec![remark()] });
	let outer = pallet_utility::Call::<Test>::batch { calls: vec![inner_batch.clone()] };
	// One layer only: the inner batch is returned as-is, not expanded. Recursion
	// (and its depth limit) is the caller's responsibility.
	assert_eq!(utility_inner_calls(&outer), vec![&inner_batch]);
}
