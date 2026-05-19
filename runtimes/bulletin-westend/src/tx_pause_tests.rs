// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Tests for the `pallet_tx_pause` wiring: renew calls are pausable and gated by
//! `BaseCallFilter`; everything else is whitelisted and rejected by
//! `ensure_can_pause`.

use crate::{
	AccountId, Runtime, RuntimeCall, RuntimeOrigin, TransactionStorage, TxPauseMaxNameLen,
};
type TxPauseWhitelist = bulletin_pallets_common::TxPauseWhitelist<Runtime, TransactionStorage>;
use codec::Encode;
use frame_support::{
	assert_noop, assert_ok,
	traits::{Contains, GetCallMetadata},
	BoundedVec,
};
use pallet_bulletin_transaction_storage as storage_pallet;
use pallet_tx_pause::{Error as TxPauseError, RuntimeCallNameOf};
use sp_io::TestExternalities;
use sp_runtime::BuildStorage;

fn new_test_ext() -> TestExternalities {
	let storage = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	TestExternalities::new(storage)
}

fn name(pallet: &[u8], call: &[u8]) -> RuntimeCallNameOf<Runtime> {
	let max = TxPauseMaxNameLen::get() as usize;
	(
		BoundedVec::try_from(pallet.to_vec())
			.unwrap_or_else(|_| panic!("pallet name '{pallet:?}' > MaxNameLen={max}")),
		BoundedVec::try_from(call.to_vec())
			.unwrap_or_else(|_| panic!("call name '{call:?}' > MaxNameLen={max}")),
	)
}

const RENEW_CALLS: &[&[u8]] =
	&[b"renew", b"renew_content_hash", b"enable_auto_renew", b"disable_auto_renew"];

fn sample_renew_call() -> RuntimeCall {
	RuntimeCall::TransactionStorage(storage_pallet::Call::renew { block: 0, index: 0 })
}

#[test]
fn renew_call_metadata_fits_in_max_name_len() {
	let max = TxPauseMaxNameLen::get() as usize;
	let call = sample_renew_call();
	let meta = call.get_call_metadata();
	assert!(meta.pallet_name.len() <= max, "pallet '{}' > MaxNameLen", meta.pallet_name);
	assert!(meta.function_name.len() <= max, "call '{}' > MaxNameLen", meta.function_name);
	assert_eq!(meta.pallet_name, "TransactionStorage");
	for call in RENEW_CALLS {
		assert!(call.len() <= max);
	}
}

#[test]
fn whitelist_blocks_every_non_renew_call() {
	for pallet in [&b"System"[..], b"Sudo", b"Balances", b"TxPause", b"PolkadotXcm", b"Utility"] {
		assert!(
			TxPauseWhitelist::contains(&name(pallet, b"some_call")),
			"pallet {:?} must be whitelisted",
			core::str::from_utf8(pallet).unwrap()
		);
	}
	for call in [&b"store"[..], b"add_authorizer", b"apply_block_inherents"] {
		assert!(
			TxPauseWhitelist::contains(&name(b"TransactionStorage", call)),
			"TransactionStorage.{} must be whitelisted",
			core::str::from_utf8(call).unwrap()
		);
	}
}

#[test]
fn whitelist_allows_renew_family_to_be_paused() {
	for call in RENEW_CALLS {
		assert!(
			!TxPauseWhitelist::contains(&name(b"TransactionStorage", call)),
			"TransactionStorage.{} must be pausable",
			core::str::from_utf8(call).unwrap()
		);
	}
}

#[test]
fn pause_renew_via_root_filters_dispatch() {
	new_test_ext().execute_with(|| {
		let call = sample_renew_call();
		assert!(
			<Runtime as frame_system::Config>::BaseCallFilter::contains(&call),
			"renew should dispatch when not paused"
		);

		assert_ok!(pallet_tx_pause::Pallet::<Runtime>::pause(
			RuntimeOrigin::root(),
			name(b"TransactionStorage", b"renew"),
		));

		assert!(
			!<Runtime as frame_system::Config>::BaseCallFilter::contains(&call),
			"BaseCallFilter must reject renew after pause"
		);

		assert_ok!(pallet_tx_pause::Pallet::<Runtime>::unpause(
			RuntimeOrigin::root(),
			name(b"TransactionStorage", b"renew"),
		));
		assert!(<Runtime as frame_system::Config>::BaseCallFilter::contains(&call));
	});
}

#[test]
fn pause_non_renew_is_rejected_as_unpausable() {
	new_test_ext().execute_with(|| {
		for (pallet, call) in [
			(&b"System"[..], &b"set_storage"[..]),
			(b"Sudo", b"sudo"),
			(b"Balances", b"transfer_keep_alive"),
			(b"TransactionStorage", b"store"),
		] {
			assert_noop!(
				pallet_tx_pause::Pallet::<Runtime>::pause(
					RuntimeOrigin::root(),
					name(pallet, call),
				),
				TxPauseError::<Runtime>::Unpausable
			);
		}
	});
}

#[test]
fn pause_requires_root_origin() {
	new_test_ext().execute_with(|| {
		let signed: RuntimeOrigin =
			frame_system::RawOrigin::Signed(AccountId::new([1u8; 32])).into();
		assert!(pallet_tx_pause::Pallet::<Runtime>::pause(
			signed,
			name(b"TransactionStorage", b"renew"),
		)
		.is_err());
	});
}

#[test]
fn paused_call_name_round_trip_fits_storage() {
	// Belt-and-braces: encode a `RuntimeCallNameOf` for every renew variant so the
	// `BoundedVec<u8, MaxNameLen>` conversion can't silently regress if MaxNameLen shrinks.
	let max = TxPauseMaxNameLen::get() as usize;
	for call in RENEW_CALLS {
		let key = name(b"TransactionStorage", call);
		let encoded = key.encode();
		assert!(!encoded.is_empty());
		assert!(call.len() <= max);
	}
}
