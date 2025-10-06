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
use polkadot_sdk_frame::{
	deps::frame_support,
	prelude::{
		fungible::{Dust, Inspect, InspectHold, MutateHold, Unbalanced, UnbalancedHold},
		DepositConsequence, DispatchError, DispatchResult, Fortitude, Preservation, Provenance,
		WithdrawConsequence, Zero,
	},
	token::{BalanceStatus, WithdrawReasons},
	traits::{
		tokens::imbalance::TryMerge, Currency, Imbalance, ReservableCurrency, SameOrOther,
		SignedImbalance, TryDrop,
	},
};
use scale_info::TypeInfo;

/// Fungible currency implementation that does not support any balance operations.
/// Works only with zero balances.
///
/// Note: This is a workaround to satisfy the `pallet-session::Config::Currency` and
/// `pallet-proxy::Config::Currency` trait requirements.
pub struct NoCurrency<AccountId, HoldReason = ()>(
	core::marker::PhantomData<(AccountId, HoldReason)>,
);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ZeroImbalance<Balance> {
	_phantom: core::marker::PhantomData<Balance>,
}

impl<Balance: Default + Copy> Imbalance<Balance> for ZeroImbalance<Balance> {
	type Opposite = Self;

	fn zero() -> Self {
		Self { _phantom: core::marker::PhantomData }
	}

	fn split(self, _amount: Balance) -> (Self, Self) {
		(Self::zero(), Self::zero())
	}

	fn merge(self, _other: Self) -> Self {
		Self::zero()
	}

	fn subsume(&mut self, _other: Self) {}

	fn offset(self, _other: Self::Opposite) -> SameOrOther<Self, Self::Opposite> {
		SameOrOther::None
	}

	fn peek(&self) -> Balance {
		Balance::default()
	}

	fn drop_zero(self) -> Result<(), Self> {
		Ok(())
	}

	fn extract(&mut self, _amount: Balance) -> Self {
		Self::zero()
	}
}

impl<Balance: Default + Copy> TryDrop for ZeroImbalance<Balance> {
	fn try_drop(self) -> Result<(), Self> {
		Ok(())
	}
}

impl<Balance: Default + Copy> TryMerge for ZeroImbalance<Balance> {
	fn try_merge(self, _other: Self) -> Result<Self, (Self, Self)> {
		Ok(Self::zero())
	}
}

impl<AccountId, HoldReason> Currency<AccountId> for NoCurrency<AccountId, HoldReason> {
	type Balance = u128;
	type PositiveImbalance = ZeroImbalance<u128>;
	type NegativeImbalance = ZeroImbalance<u128>;

	fn total_balance(_who: &AccountId) -> Self::Balance {
		Zero::zero()
	}

	fn can_slash(_who: &AccountId, _value: Self::Balance) -> bool {
		false
	}

	fn total_issuance() -> Self::Balance {
		Zero::zero()
	}

	fn minimum_balance() -> Self::Balance {
		Zero::zero()
	}

	fn burn(_amount: Self::Balance) -> Self::PositiveImbalance {
		ZeroImbalance::zero()
	}

	fn issue(_amount: Self::Balance) -> Self::NegativeImbalance {
		ZeroImbalance::zero()
	}

	fn free_balance(_who: &AccountId) -> Self::Balance {
		Zero::zero()
	}

	fn ensure_can_withdraw(
		_who: &AccountId,
		_amount: Self::Balance,
		_reasons: WithdrawReasons,
		_new_balance: Self::Balance,
	) -> frame_support::pallet_prelude::DispatchResult {
		Ok(())
	}

	fn transfer(
		_source: &AccountId,
		_dest: &AccountId,
		_value: Self::Balance,
		_existence_requirement: frame_support::traits::ExistenceRequirement,
	) -> frame_support::pallet_prelude::DispatchResult {
		Ok(())
	}

	fn slash(_who: &AccountId, _value: Self::Balance) -> (Self::NegativeImbalance, Self::Balance) {
		(ZeroImbalance::zero(), Zero::zero())
	}

	fn deposit_into_existing(
		_who: &AccountId,
		_value: Self::Balance,
	) -> Result<Self::PositiveImbalance, frame_support::pallet_prelude::DispatchError> {
		Ok(ZeroImbalance::zero())
	}

	fn deposit_creating(_who: &AccountId, _value: Self::Balance) -> Self::PositiveImbalance {
		ZeroImbalance::zero()
	}

	fn withdraw(
		_who: &AccountId,
		_value: Self::Balance,
		_reasons: WithdrawReasons,
		_liveness: frame_support::traits::ExistenceRequirement,
	) -> Result<Self::NegativeImbalance, frame_support::pallet_prelude::DispatchError> {
		Ok(ZeroImbalance::zero())
	}

	fn make_free_balance_be(
		_who: &AccountId,
		_balance: Self::Balance,
	) -> frame_support::traits::SignedImbalance<Self::Balance, Self::PositiveImbalance> {
		SignedImbalance::zero()
	}
}

impl<AccountId, HoldReason> ReservableCurrency<AccountId> for NoCurrency<AccountId, HoldReason> {
	fn can_reserve(_who: &AccountId, _value: Self::Balance) -> bool {
		true
	}

	fn reserve(_who: &AccountId, _value: Self::Balance) -> DispatchResult {
		Ok(())
	}

	fn unreserve(_who: &AccountId, _value: Self::Balance) -> Self::Balance {
		Zero::zero()
	}

	fn slash_reserved(_who: &AccountId, _value: Self::Balance) -> (ZeroImbalance<u128>, u128) {
		(ZeroImbalance::zero(), Zero::zero())
	}

	fn repatriate_reserved(
		_slashed: &AccountId,
		_beneficiary: &AccountId,
		_value: Self::Balance,
		_status: BalanceStatus,
	) -> Result<Self::Balance, DispatchError> {
		Ok(Zero::zero())
	}

	fn reserved_balance(_who: &AccountId) -> Self::Balance {
		Zero::zero()
	}
}
