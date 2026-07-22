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

//! Tests for `NoCurrency`, `ZeroImbalance` and the call inspectors.

use crate::{
	inspect_proxy_wrapper, inspect_sudo_wrapper, inspect_utility_wrapper, mock::*,
	proxy_inner_calls, sudo_inner_calls, utility_inner_calls, NoCurrency, ZeroImbalance,
};
use polkadot_sdk_frame::{
	deps::frame_system,
	prelude::{
		fungible::{Inspect, InspectHold, Unbalanced, UnbalancedHold},
		DepositConsequence, Fortitude, Preservation, Provenance, Weight, WithdrawConsequence, H256,
	},
	token::{BalanceStatus, WithdrawReasons},
	traits::{
		tokens::imbalance::TryMerge, Currency, ExistenceRequirement, Imbalance, ReservableCurrency,
		SameOrOther, SignedImbalance, TryDrop,
	},
};

type Cur = NoCurrency<u64, ()>;
type Z = ZeroImbalance<u128>;

#[test]
fn no_currency_currency_is_zero_and_noop() {
	assert_eq!(<Cur as Currency<u64>>::total_balance(&1), 0);
	assert_eq!(<Cur as Currency<u64>>::total_issuance(), 0);
	assert_eq!(<Cur as Currency<u64>>::minimum_balance(), 0);
	assert_eq!(<Cur as Currency<u64>>::free_balance(&1), 0);
	assert!(!<Cur as Currency<u64>>::can_slash(&1, 100));

	assert_eq!(<Cur as Currency<u64>>::burn(100).peek(), 0);
	assert_eq!(<Cur as Currency<u64>>::issue(100).peek(), 0);
	assert_eq!(<Cur as Currency<u64>>::total_issuance(), 0);

	assert_eq!(
		<Cur as Currency<u64>>::ensure_can_withdraw(&1, 100, WithdrawReasons::all(), 0),
		Ok(())
	);
	assert_eq!(
		<Cur as Currency<u64>>::transfer(&1, &2, 100, ExistenceRequirement::AllowDeath),
		Ok(())
	);
	assert_eq!(<Cur as Currency<u64>>::total_balance(&2), 0);

	let (imbalance, remainder) = <Cur as Currency<u64>>::slash(&1, 100);
	assert_eq!(imbalance.peek(), 0);
	assert_eq!(remainder, 0);

	assert_eq!(<Cur as Currency<u64>>::deposit_into_existing(&1, 100).unwrap().peek(), 0);
	assert_eq!(<Cur as Currency<u64>>::deposit_creating(&1, 100).peek(), 0);
	assert_eq!(
		<Cur as Currency<u64>>::withdraw(
			&1,
			100,
			WithdrawReasons::all(),
			ExistenceRequirement::AllowDeath
		)
		.unwrap()
		.peek(),
		0
	);

	match <Cur as Currency<u64>>::make_free_balance_be(&1, 100) {
		SignedImbalance::Positive(p) => assert_eq!(p.peek(), 0),
		SignedImbalance::Negative(_) => panic!("expected positive zero imbalance"),
	}
	assert_eq!(<Cur as Currency<u64>>::free_balance(&1), 0);
}

#[test]
fn no_currency_reservable_is_zero_and_noop() {
	assert!(<Cur as ReservableCurrency<u64>>::can_reserve(&1, 100));
	assert_eq!(<Cur as ReservableCurrency<u64>>::reserve(&1, 100), Ok(()));
	assert_eq!(<Cur as ReservableCurrency<u64>>::reserved_balance(&1), 0);
	assert_eq!(<Cur as ReservableCurrency<u64>>::unreserve(&1, 100), 0);

	let (imbalance, remainder) = <Cur as ReservableCurrency<u64>>::slash_reserved(&1, 100);
	assert_eq!(imbalance.peek(), 0);
	assert_eq!(remainder, 0);

	assert_eq!(
		<Cur as ReservableCurrency<u64>>::repatriate_reserved(&1, &2, 100, BalanceStatus::Free),
		Ok(0)
	);
}

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

#[test]
fn zero_imbalance_is_always_zero() {
	assert_eq!(Z::zero().peek(), 0);

	let (a, b) = Z::zero().split(100);
	assert_eq!(a.peek(), 0);
	assert_eq!(b.peek(), 0);

	assert_eq!(Z::zero().merge(Z::zero()).peek(), 0);

	let mut subsumed = Z::zero();
	subsumed.subsume(Z::zero());
	assert_eq!(subsumed.peek(), 0);

	assert!(matches!(Z::zero().offset(Z::zero()), SameOrOther::None));
	assert_eq!(Z::zero().drop_zero(), Ok(()));

	let mut extracted = Z::zero();
	assert_eq!(extracted.extract(100).peek(), 0);
	assert_eq!(extracted.peek(), 0);

	assert_eq!(Z::zero().try_drop(), Ok(()));
	assert_eq!(Z::zero().try_merge(Z::zero()), Ok(Z::zero()));
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
		let expected: Vec<&RuntimeCall> = calls.iter().collect();
		assert_eq!(utility_inner_calls(&call), expected);
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
		assert_eq!(utility_inner_calls(call), vec![&inner]);
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
	assert_eq!(utility_inner_calls(&call), vec![&main, &fallback]);
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
fn sudo_wrapping_variants_return_inner_call() {
	let inner = remark();
	let variants: Vec<pallet_sudo::Call<Test>> = vec![
		pallet_sudo::Call::sudo { call: Box::new(inner.clone()) },
		pallet_sudo::Call::sudo_unchecked_weight {
			call: Box::new(inner.clone()),
			weight: Weight::zero(),
		},
		pallet_sudo::Call::sudo_as { who: 1, call: Box::new(inner.clone()) },
	];
	for call in &variants {
		assert_eq!(sudo_inner_calls(call), vec![&inner]);
		assert_eq!(inspect_sudo_wrapper(call), Some(vec![&inner]));
	}
}

#[test]
fn sudo_non_wrapping_variants_have_no_inner_calls() {
	for call in
		[pallet_sudo::Call::<Test>::set_key { new: 1 }, pallet_sudo::Call::<Test>::remove_key {}]
	{
		assert!(sudo_inner_calls(&call).is_empty());
		assert_eq!(inspect_sudo_wrapper(&call), None);
	}
}

#[test]
fn proxy_wrapping_variants_return_inner_call() {
	let inner = remark();
	let variants: Vec<pallet_proxy::Call<Test>> = vec![
		pallet_proxy::Call::proxy {
			real: 1,
			force_proxy_type: None,
			call: Box::new(inner.clone()),
		},
		pallet_proxy::Call::proxy_announced {
			delegate: 1,
			real: 2,
			force_proxy_type: Some(()),
			call: Box::new(inner.clone()),
		},
	];
	for call in &variants {
		assert_eq!(proxy_inner_calls(call), vec![&inner]);
		assert_eq!(inspect_proxy_wrapper(call), Some(vec![&inner]));
	}
}

#[test]
fn proxy_non_wrapping_variants_have_no_inner_calls() {
	let variants: Vec<pallet_proxy::Call<Test>> = vec![
		pallet_proxy::Call::add_proxy { delegate: 1, proxy_type: (), delay: 0 },
		pallet_proxy::Call::remove_proxy { delegate: 1, proxy_type: (), delay: 0 },
		pallet_proxy::Call::remove_proxies {},
		pallet_proxy::Call::create_pure { proxy_type: (), delay: 0, index: 0 },
		pallet_proxy::Call::kill_pure {
			spawner: 1,
			proxy_type: (),
			index: 0,
			height: 0,
			ext_index: 0,
		},
		pallet_proxy::Call::announce { real: 1, call_hash: H256::zero() },
		pallet_proxy::Call::remove_announcement { real: 1, call_hash: H256::zero() },
		pallet_proxy::Call::reject_announcement { delegate: 1, call_hash: H256::zero() },
		pallet_proxy::Call::poke_deposit {},
	];
	for call in &variants {
		assert!(proxy_inner_calls(call).is_empty());
		assert_eq!(inspect_proxy_wrapper(call), None);
	}
}

#[test]
fn nested_wrappers_unwrap_one_layer_at_a_time() {
	let target = remark();
	let sudo_call = RuntimeCall::Sudo(pallet_sudo::Call::sudo { call: Box::new(target.clone()) });
	let proxy_call = RuntimeCall::Proxy(pallet_proxy::Call::proxy {
		real: 1,
		force_proxy_type: None,
		call: Box::new(sudo_call.clone()),
	});
	let batch =
		pallet_utility::Call::<Test>::batch { calls: vec![proxy_call.clone(), target.clone()] };

	let level1 = inspect_utility_wrapper(&batch).unwrap();
	assert_eq!(level1, vec![&proxy_call, &target]);

	let RuntimeCall::Proxy(inner_proxy) = level1[0] else { panic!("expected proxy call") };
	let level2 = inspect_proxy_wrapper(inner_proxy).unwrap();
	assert_eq!(level2, vec![&sudo_call]);

	let RuntimeCall::Sudo(inner_sudo) = level2[0] else { panic!("expected sudo call") };
	assert_eq!(inspect_sudo_wrapper(inner_sudo), Some(vec![&target]));
}

#[test]
fn inspectors_do_not_recurse() {
	let inner_batch = RuntimeCall::Utility(pallet_utility::Call::batch { calls: vec![remark()] });
	let outer = pallet_utility::Call::<Test>::batch { calls: vec![inner_batch.clone()] };
	// One layer only: the inner batch is returned as-is, not expanded. Recursion
	// (and its depth limit) is the caller's responsibility.
	assert_eq!(utility_inner_calls(&outer), vec![&inner_batch]);
}
