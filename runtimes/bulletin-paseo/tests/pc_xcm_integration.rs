// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Cumulus.
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

//! People Chain → Bulletin Chain `authorize_account` integration tests.
//!
//! Drives `XcmExecutor::prepare_and_execute` directly with messages shaped the
//! way People Chain sends them (sibling parachain origin, `OriginKind::Xcm`,
//! unpaid execution + Transact). Exercises the full receive-side pipeline:
//! barrier, origin conversion, `SafeCallFilter`, `Authorizer = EnsureXcm<
//! IsSiblingParachain>`, and the pallet's `authorize_account` /
//! `refresh_account_authorization` semantics.

#![cfg(test)]

mod common;

use bulletin_paseo_runtime::{
	paseo_constants::locations::PeopleLocation, xcm_config::LocationToAccountId, Runtime,
	RuntimeCall, RuntimeGenesisConfig, RuntimeOrigin, System, TransactionStorage,
};
use common::{advance_block, assert_extrinsic_ok, construct_and_apply_extrinsic};
use frame_support::{assert_ok, traits::Get};
use pallet_bulletin_transaction_storage::{
	AuthorizationExtent, Call as TxStorageCall, Config as TxStorageConfig,
};
use parachains_common::{AccountId, BlockNumber};
use sp_core::Encode;
use sp_keyring::Sr25519Keyring;
use sp_runtime::BuildStorage;
use xcm::latest::{prelude::*, InstructionError};
use xcm_executor::traits::ConvertLocation;

use bulletin_paseo_runtime::xcm_config::XcmConfig;

/// People Chain location on Paseo. Matches `paseo_constants::PeopleLocation`.
fn pc_location() -> Location {
	PeopleLocation::get()
}

fn auth_period() -> BlockNumber {
	<<Runtime as TxStorageConfig>::AuthorizationPeriod as Get<BlockNumber>>::get()
}

fn empty() -> AuthorizationExtent {
	AuthorizationExtent::default()
}

fn extent(
	bytes: u64,
	bytes_allowance: u64,
	transactions: u32,
	transactions_allowance: u32,
) -> AuthorizationExtent {
	AuthorizationExtent {
		bytes,
		bytes_permanent: 0,
		bytes_allowance,
		transactions,
		transactions_allowance,
	}
}

fn extent_of(who: &AccountId) -> AuthorizationExtent {
	TransactionStorage::account_authorization_extent(who.clone())
}

/// Build an XCM message in the shape PC uses: free unpaid execution + Transact.
fn xcm_transact(call: RuntimeCall, kind: OriginKind) -> Xcm<RuntimeCall> {
	Xcm::builder_unsafe()
		.unpaid_execution(Unlimited, None)
		.transact(kind, None, call.encode())
		.build()
}

fn execute_from(origin: Location, message: Xcm<RuntimeCall>) -> Result<(), InstructionError> {
	let mut id = [0u8; 32];
	xcm_executor::XcmExecutor::<XcmConfig>::prepare_and_execute(
		origin,
		message,
		&mut id,
		Weight::MAX,
		Weight::MAX,
	)
	.ensure_complete()
}

fn pc_authorize(who: AccountId, transactions: u32, bytes: u64) -> Result<(), InstructionError> {
	let call = RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
		who,
		transactions,
		bytes,
	});
	execute_from(pc_location(), xcm_transact(call, OriginKind::Xcm))
}

fn pc_refresh(who: AccountId) -> Result<(), InstructionError> {
	let call =
		RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::refresh_account_authorization {
			who,
		});
	execute_from(pc_location(), xcm_transact(call, OriginKind::Xcm))
}

fn new_test_ext() -> sp_io::TestExternalities {
	let mut ext =
		sp_io::TestExternalities::new(RuntimeGenesisConfig::default().build_storage().unwrap());
	ext.execute_with(advance_block);
	ext
}

mod origin_rejections {
	use super::*;

	// XCM `Transact` reports a successful `Outcome` even when the inner call's
	// dispatch fails with `BadOrigin` or returns a runtime error. The signal
	// that the rejection happened is therefore the absence of any storage
	// mutation, which is what these tests assert.

	#[test]
	fn relay_chain_origin_cannot_authorize() {
		new_test_ext().execute_with(|| {
			let target: AccountId = Sr25519Keyring::Ferdie.to_account_id();
			let call =
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
					who: target.clone(),
					transactions: 5,
					bytes: 1_000,
				});

			assert_ok!(execute_from(Location::parent(), xcm_transact(call, OriginKind::Xcm)));

			assert_eq!(extent_of(&target), empty(), "relay-chain origin must not authorize");
		});
	}

	#[test]
	fn sibling_with_sovereign_origin_kind_cannot_authorize() {
		new_test_ext().execute_with(|| {
			let target: AccountId = Sr25519Keyring::Ferdie.to_account_id();
			let call =
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
					who: target.clone(),
					transactions: 5,
					bytes: 1_000,
				});

			assert_ok!(execute_from(
				pc_location(),
				xcm_transact(call, OriginKind::SovereignAccount)
			));

			assert_eq!(
				extent_of(&target),
				empty(),
				"sibling with OriginKind::SovereignAccount must not authorize",
			);

			let sovereign = LocationToAccountId::convert_location(&pc_location())
				.expect("sibling sovereign account must derive");
			assert_eq!(
				extent_of(&sovereign),
				empty(),
				"derived sibling sovereign must not gain authorization",
			);
		});
	}

	#[test]
	fn random_local_origin_cannot_authorize() {
		new_test_ext().execute_with(|| {
			let target: AccountId = Sr25519Keyring::Ferdie.to_account_id();
			let stranger_loc =
				Location::new(0, [Junction::AccountId32 { network: None, id: [0x42u8; 32] }]);
			let call =
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
					who: target.clone(),
					transactions: 5,
					bytes: 1_000,
				});

			assert_ok!(execute_from(stranger_loc, xcm_transact(call, OriginKind::Xcm)));

			// `XcmPassthrough` resolves the origin to `pallet_xcm::Origin::Xcm(stranger)`,
			// which does not match `IsSiblingParachain` and is not in `TestAccounts`,
			// so the inner `authorize_account` dispatch is rejected with `BadOrigin`.
			assert_eq!(extent_of(&target), empty());
		});
	}

	#[test]
	fn authorize_with_zero_bytes_fails() {
		new_test_ext().execute_with(|| {
			let who: AccountId = Sr25519Keyring::Alice.to_account_id();

			assert_ok!(pc_authorize(who.clone(), 1, 0));

			assert_eq!(extent_of(&who), empty());
			assert!(!TransactionStorage::account_has_active_authorization(&who));
		});
	}
}

// SafeCallFilter.
//
// Storage-mutating calls must not reach dispatch over XCM, even when the
// origin is otherwise valid. The filter inspects through `Utility::batch*`.
mod safe_call_filter {
	use super::*;

	#[test]
	fn sibling_xcm_store_is_blocked() {
		new_test_ext().execute_with(|| {
			let who: AccountId = Sr25519Keyring::Alice.to_account_id();
			assert_ok!(TransactionStorage::authorize_account(
				RuntimeOrigin::root(),
				who.clone(),
				1,
				1_000
			));

			let store_call = RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
				data: vec![0u8; 100],
			});

			assert!(execute_from(pc_location(), xcm_transact(store_call, OriginKind::Xcm)).is_err());
			assert_eq!(extent_of(&who), extent(0, 1_000, 0, 1));
		});
	}

	#[test]
	fn sibling_xcm_batch_with_store_is_entirely_blocked() {
		new_test_ext().execute_with(|| {
			let target: AccountId = Sr25519Keyring::Bob.to_account_id();
			let store_target: AccountId = Sr25519Keyring::Alice.to_account_id();

			let authorize_call =
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
					who: target.clone(),
					transactions: 5,
					bytes: 1_000,
				});
			let store_call = RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
				data: vec![0u8; 50],
			});
			let batch_call = RuntimeCall::Utility(pallet_utility::Call::batch {
				calls: vec![authorize_call, store_call],
			});

			assert!(execute_from(pc_location(), xcm_transact(batch_call, OriginKind::Xcm)).is_err());
			assert_eq!(
				extent_of(&target),
				empty(),
				"the inner authorize_account must NOT have executed alongside a filtered store",
			);
			assert_eq!(extent_of(&store_target), empty());
		});
	}

	#[test]
	fn sibling_xcm_batch_of_only_authorize_calls_succeeds() {
		new_test_ext().execute_with(|| {
			let alice: AccountId = Sr25519Keyring::Alice.to_account_id();
			let bob: AccountId = Sr25519Keyring::Bob.to_account_id();

			let authorize_alice =
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
					who: alice.clone(),
					transactions: 5,
					bytes: 1_000,
				});
			let authorize_bob =
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
					who: bob.clone(),
					transactions: 10,
					bytes: 2_000,
				});
			let batch_call = RuntimeCall::Utility(pallet_utility::Call::batch {
				calls: vec![authorize_alice, authorize_bob],
			});

			assert_ok!(execute_from(pc_location(), xcm_transact(batch_call, OriginKind::Xcm)));
			assert_eq!(extent_of(&alice), extent(0, 1_000, 0, 5));
			assert_eq!(extent_of(&bob), extent(0, 2_000, 0, 10));
		});
	}
}

mod authorize_semantics {
	use super::*;

	#[test]
	fn happy_path_from_sibling() {
		new_test_ext().execute_with(|| {
			let who: AccountId = Sr25519Keyring::Alice.to_account_id();

			assert_ok!(pc_authorize(who.clone(), 10, 1_000_000));

			assert_eq!(extent_of(&who), extent(0, 1_000_000, 0, 10));
			assert!(TransactionStorage::account_has_active_authorization(&who));
		});
	}

	// Authorizations are additive within an unexpired window.
	// Each people chain claim adds to the existing allowance
	// and does NOT push expiry forward.
	// Consumed counters are preserved.
	#[test]
	fn additive_within_window() {
		new_test_ext().execute_with(|| {
			let account = Sr25519Keyring::Alice;
			let who: AccountId = account.to_account_id();

			assert_ok!(pc_authorize(who.clone(), 5, 1_000));
			assert_eq!(extent_of(&who), extent(0, 1_000, 0, 5));

			advance_block();
			assert_extrinsic_ok(construct_and_apply_extrinsic(
				Some(account.pair()),
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
					data: vec![0u8; 200],
				}),
			));
			assert_eq!(extent_of(&who), extent(200, 1_000, 1, 5));

			assert_ok!(pc_authorize(who.clone(), 3, 500));
			assert_eq!(extent_of(&who), extent(200, 1_500, 1, 8));

			assert_ok!(pc_authorize(who.clone(), 2, 250));
			assert_eq!(extent_of(&who), extent(200, 1_750, 1, 10));

			// Expiry must not have been pushed forward by additive claims.
			let now = System::block_number();
			System::set_block_number(now + auth_period());
			assert_eq!(extent_of(&who), empty(), "additive claims must not have extended expiry",);
		});
	}

	// Replace after expiry.
	#[test]
	fn replaces_after_expiry() {
		new_test_ext().execute_with(|| {
			let account = Sr25519Keyring::Alice;
			let who: AccountId = account.to_account_id();

			assert_ok!(pc_authorize(who.clone(), 5, 1_000));

			advance_block();
			assert_extrinsic_ok(construct_and_apply_extrinsic(
				Some(account.pair()),
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
					data: vec![0u8; 400],
				}),
			));
			assert_eq!(extent_of(&who), extent(400, 1_000, 1, 5));

			let now = System::block_number();
			System::set_block_number(now + auth_period() + 1);
			assert_eq!(extent_of(&who), empty(), "extent must read empty once expired");

			assert_ok!(pc_authorize(who.clone(), 1, 100));
			assert_eq!(extent_of(&who), extent(0, 100, 0, 1));
		});
	}

	#[test]
	fn account_scopes_are_independent() {
		new_test_ext().execute_with(|| {
			let alice: AccountId = Sr25519Keyring::Alice.to_account_id();
			let bob: AccountId = Sr25519Keyring::Bob.to_account_id();

			assert_ok!(pc_authorize(alice.clone(), 5, 1_000));
			assert_ok!(pc_authorize(bob.clone(), 10, 2_000));

			assert_eq!(extent_of(&alice), extent(0, 1_000, 0, 5));
			assert_eq!(extent_of(&bob), extent(0, 2_000, 0, 10));

			let now = System::block_number();
			System::set_block_number(now + auth_period() + 1);
			assert_ok!(TransactionStorage::remove_expired_account_authorization(
				RuntimeOrigin::none(),
				alice.clone(),
			));
			assert_eq!(extent_of(&alice), empty());
			// Bob's entry is still in storage — re-authorize lands as a
			// fresh entry rather than failing.
			assert_ok!(pc_authorize(bob.clone(), 1, 50));
			assert_eq!(extent_of(&bob), extent(0, 50, 0, 1));
		});
	}
}

mod refresh {
	use super::*;

	#[test]
	fn extends_only_expiration() {
		new_test_ext().execute_with(|| {
			let who: AccountId = Sr25519Keyring::Alice.to_account_id();

			assert_ok!(pc_authorize(who.clone(), 5, 1_000));
			assert_eq!(extent_of(&who), extent(0, 1_000, 0, 5));

			let half = auth_period() / 2;
			let now = System::block_number();
			System::set_block_number(now + half);

			assert_ok!(pc_refresh(who.clone()));

			assert_eq!(extent_of(&who), extent(0, 1_000, 0, 5));

			// A block past the *original* expiry must still be active because
			// refresh extended the window.
			System::set_block_number(now + auth_period());
			assert!(
				TransactionStorage::account_has_active_authorization(&who),
				"refresh must have extended expiry past the original window",
			);
		});
	}

	#[test]
	fn without_prior_authorize_fails() {
		new_test_ext().execute_with(|| {
			let who: AccountId = Sr25519Keyring::Alice.to_account_id();

			// XCM completes; the inner `refresh_account_authorization` returns
			// `Error::AccountNotAuthorized` which is reported via runtime events,
			// not as an XCM-level instruction error.
			assert_ok!(pc_refresh(who.clone()));

			assert_eq!(extent_of(&who), empty());
			assert!(!TransactionStorage::account_has_active_authorization(&who));
		});
	}
}

mod end_to_end {
	use super::*;

	#[test]
	fn authorize_then_user_stores_and_renews() {
		new_test_ext().execute_with(|| {
			let account = Sr25519Keyring::Alice;
			let who: AccountId = account.to_account_id();

			// PC issues a 2-tx, 4_000-byte allowance via XCM.
			assert_ok!(pc_authorize(who.clone(), 2, 4_000));
			assert_eq!(extent_of(&who), extent(0, 4_000, 0, 2));

			// Authorized user stores 1_000 bytes (feeless, boost-tier).
			advance_block();
			let stored_block = System::block_number();
			assert_extrinsic_ok(construct_and_apply_extrinsic(
				Some(account.pair()),
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
					data: vec![0u8; 1_000],
				}),
			));
			assert_eq!(extent_of(&who), extent(1_000, 4_000, 1, 2));

			// Same user renews against the just-stored block/index. `renew`
			// charges the per-window permanent quota (`bytes_permanent`).
			advance_block();
			assert_extrinsic_ok(construct_and_apply_extrinsic(
				Some(account.pair()),
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::renew {
					block: stored_block,
					index: 0,
				}),
			));
			assert_eq!(
				extent_of(&who),
				AuthorizationExtent {
					bytes: 1_000,
					bytes_permanent: 1_000,
					bytes_allowance: 4_000,
					transactions: 2,
					transactions_allowance: 2,
				},
			);
		});
	}
}
