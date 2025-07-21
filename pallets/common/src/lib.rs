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

#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Decode, Encode};
use polkadot_sdk_frame::prelude::{
	fungible::{Dust, Inspect, InspectHold, MutateHold, Unbalanced, UnbalancedHold},
	DepositConsequence, DispatchError, DispatchResult, Fortitude, Preservation, Provenance,
	WithdrawConsequence, Zero,
};
use scale_info::TypeInfo;

/// Fungible currency implementation that does not support any balance operations.
/// Works only with zero balances.
///
/// Note: This is a workaround to satisfy the `pallet-session::Config::Currency` trait requirements.
pub struct NoCurrency<AccountId, HoldReason>(core::marker::PhantomData<(AccountId, HoldReason)>);

impl<AccountId, HoldReason: Encode + Decode + TypeInfo + 'static> Inspect<AccountId>
	for NoCurrency<AccountId, HoldReason>
{
	type Balance = u128;

	fn total_issuance() -> Self::Balance {
		Zero::zero()
	}

	fn minimum_balance() -> Self::Balance {
		Zero::zero()
	}

	fn total_balance(_who: &AccountId) -> Self::Balance {
		Zero::zero()
	}

	fn balance(_who: &AccountId) -> Self::Balance {
		Zero::zero()
	}

	fn reducible_balance(
		_who: &AccountId,
		_preservation: Preservation,
		_force: Fortitude,
	) -> Self::Balance {
		Zero::zero()
	}

	fn can_deposit(
		_who: &AccountId,
		_amount: Self::Balance,
		_provenance: Provenance,
	) -> DepositConsequence {
		DepositConsequence::Success
	}

	fn can_withdraw(
		_who: &AccountId,
		_amount: Self::Balance,
	) -> WithdrawConsequence<Self::Balance> {
		WithdrawConsequence::Success
	}
}

impl<AccountId, HoldReason: Encode + Decode + TypeInfo + 'static> Unbalanced<AccountId>
	for NoCurrency<AccountId, HoldReason>
{
	fn handle_dust(_dust: Dust<AccountId, Self>) {}

	fn write_balance(
		_who: &AccountId,
		_amount: Self::Balance,
	) -> Result<Option<Self::Balance>, DispatchError> {
		Ok(None)
	}

	fn set_total_issuance(_amount: Self::Balance) {}
}

impl<AccountId, HoldReason: Encode + Decode + TypeInfo + 'static> InspectHold<AccountId>
	for NoCurrency<AccountId, HoldReason>
{
	type Reason = HoldReason;

	fn total_balance_on_hold(_who: &AccountId) -> Self::Balance {
		Zero::zero()
	}

	fn balance_on_hold(_reason: &Self::Reason, _who: &AccountId) -> Self::Balance {
		Zero::zero()
	}
}

impl<AccountId, HoldReason: Encode + Decode + TypeInfo + 'static> UnbalancedHold<AccountId>
	for NoCurrency<AccountId, HoldReason>
{
	fn set_balance_on_hold(
		_reason: &Self::Reason,
		_who: &AccountId,
		_amount: Self::Balance,
	) -> DispatchResult {
		Ok(())
	}
}

impl<AccountId, HoldReason: Encode + Decode + TypeInfo + 'static> MutateHold<AccountId>
	for NoCurrency<AccountId, HoldReason>
{
}
