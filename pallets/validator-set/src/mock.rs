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

//! Mock helpers for Validator Set pallet.

#![cfg(test)]

use crate as pallet_validator_set;
use pallet_session::ShouldEndSession;
use polkadot_sdk_frame::{
	deps::sp_runtime::{impl_opaque_keys, testing::UintAuthorityId, BoundToRuntimeAppPublic},
	prelude::*,
	runtime::prelude::*,
	testing_prelude::*,
	traits::{ConvertInto, OneSessionHandler},
};
use std::cell::Cell;

pub type AccountId = u64;
type Block = frame_system::mocking::MockBlock<Test>;

construct_runtime!(
	pub struct Test {
		System: frame_system,
		ValidatorSet: pallet_validator_set,
		Session: pallet_session,
		Historical: pallet_session::historical,
	}
);

pub struct MockSessionHandler;

impl OneSessionHandler<AccountId> for MockSessionHandler {
	type Key = UintAuthorityId;

	fn on_genesis_session<'a, I>(_validators: I)
	where
		I: Iterator<Item = (&'a AccountId, Self::Key)>,
	{
	}

	fn on_new_session<'a, I>(_changed: bool, _validators: I, _queued_validators: I)
	where
		I: Iterator<Item = (&'a AccountId, Self::Key)>,
	{
	}

	fn on_disabled(_i: u32) {}
}

impl BoundToRuntimeAppPublic for MockSessionHandler {
	type Public = UintAuthorityId;
}

impl_opaque_keys! {
	pub struct MockSessionKeys {
		pub mock: MockSessionHandler,
	}
}

impl From<AccountId> for MockSessionKeys {
	fn from(who: AccountId) -> Self {
		Self { mock: UintAuthorityId(who) }
	}
}

thread_local! {
	static END_SESSION: Cell<bool> = const { Cell::new(false) };
}

pub struct MockShouldEndSession;

impl<T> ShouldEndSession<T> for MockShouldEndSession {
	fn should_end_session(_now: T) -> bool {
		END_SESSION.replace(false)
	}
}

pub fn next_block() {
	System::on_finalize(System::block_number());
	System::set_block_number(System::block_number() + 1);
	System::on_initialize(System::block_number());
	Session::on_initialize(System::block_number());
}

pub fn next_session() {
	END_SESSION.set(true);
	next_block();
	assert!(!END_SESSION.get());
}

pub fn new_test_ext() -> TestExternalities {
	let validators = vec![1, 2, 3];
	let keys = validators.iter().map(|who| (*who, *who, (*who).into())).collect();
	let t = RuntimeGenesisConfig {
		system: Default::default(),
		session: SessionConfig { keys, non_authority_keys: vec![] },
		validator_set: ValidatorSetConfig { initial_validators: validators.try_into().unwrap() },
	}
	.build_storage()
	.unwrap();
	t.into()
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Nonce = u64;
	type Block = Block;
	type BlockHashCount = ConstU64<250>;
}

parameter_types! {
	pub const SetKeysCooldownBlocks: BlockNumberFor<Test> = 2;
}

impl pallet_validator_set::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = ();
	type AddRemoveOrigin = EnsureRoot<AccountId>;
	type MaxAuthorities = ConstU32<6>;
	type SetKeysCooldownBlocks = SetKeysCooldownBlocks;
}

impl pallet_session::Config for Test {
	type ValidatorId = AccountId;
	type ValidatorIdOf = ConvertInto;
	type ShouldEndSession = MockShouldEndSession;
	type NextSessionRotation = ();
	type SessionManager = ValidatorSet;
	type SessionHandler = (MockSessionHandler,);
	type Keys = MockSessionKeys;
	type WeightInfo = ();
	type RuntimeEvent = RuntimeEvent;
	type Currency = pallets_common::NoCurrency<AccountId, RuntimeHoldReason>;
	type KeyDeposit = ();
	type DisablingStrategy = ();
}

impl pallet_session::historical::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type FullIdentification = Self::ValidatorId;
	type FullIdentificationOf = Self::ValidatorIdOf;
}
