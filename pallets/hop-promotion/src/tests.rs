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

//! Tests for hop-promotion pallet.

use crate::{mock::*, signing_payload, signing_payload_v2};
use bulletin_transaction_storage_primitives::ValidTransactionParams;
use codec::Encode;
use frame_support::{
	assert_noop, assert_ok,
	traits::{Authorize, IntegrityTest},
};
use sp_core::H256;
use sp_io::hashing::blake2_256;
use sp_keyring::Sr25519Keyring;
use sp_runtime::{
	transaction_validity::{InvalidTransaction, TransactionSource},
	AccountId32, MultiSignature, MultiSigner,
};

const TEST_TIMESTAMP_MS: u64 = 1_700_000_000_000;

fn authorized_origin() -> RuntimeOrigin {
	frame_system::Origin::<Test>::Authorized.into()
}

/// Build a `(signer, signature)` pair where `keyring` signs the payload for `(data, ts)`.
fn signed_by(
	keyring: Sr25519Keyring,
	data: &[u8],
	submit_timestamp: u64,
) -> (MultiSigner, MultiSignature) {
	let payload = signing_payload(&blake2_256(data), submit_timestamp);
	let sig = keyring.sign(&payload);
	(MultiSigner::Sr25519(keyring.public()), MultiSignature::Sr25519(sig))
}

fn dummy_signer_and_sig() -> (MultiSigner, MultiSignature) {
	(
		MultiSigner::Sr25519(Sr25519Keyring::Alice.public()),
		MultiSignature::Sr25519(Default::default()),
	)
}

fn authorize_account(who: AccountId32, transactions: u32, bytes: u64) {
	assert_ok!(pallet_bulletin_transaction_storage::Pallet::<Test>::authorize_account(
		RuntimeOrigin::root(),
		who,
		transactions,
		bytes,
	));
}

fn set_now(ms: u64) {
	pallet_timestamp::Pallet::<Test>::set_timestamp(ms);
}

fn make_promote_call(
	data: Vec<u8>,
	signer: MultiSigner,
	signature: MultiSignature,
	submit_timestamp: u64,
) -> RuntimeCall {
	RuntimeCall::HopPromotion(crate::Call::promote { data, signer, signature, submit_timestamp })
}

/// V2 counterpart of [`signed_by`]: signs the V2 payload that additionally
/// binds the genesis hash and the recipients hash.
fn signed_by_v2(
	keyring: Sr25519Keyring,
	data: &[u8],
	submit_timestamp: u64,
	recipients_hash: &H256,
	genesis_hash: &[u8; 32],
) -> (MultiSigner, MultiSignature) {
	let payload = signing_payload_v2(
		&blake2_256(data),
		submit_timestamp,
		genesis_hash,
		recipients_hash.as_fixed_bytes(),
	);
	let sig = keyring.sign(&payload);
	(MultiSigner::Sr25519(keyring.public()), MultiSignature::Sr25519(sig))
}

fn make_promote_v2_call(
	data: Vec<u8>,
	signer: MultiSigner,
	signature: MultiSignature,
	submit_timestamp: u64,
	recipients_hash: H256,
) -> RuntimeCall {
	RuntimeCall::HopPromotion(crate::Call::promote_v2 {
		signer,
		signature,
		submit_timestamp,
		recipients_hash,
		data,
	})
}

fn current_genesis_hash() -> [u8; 32] {
	*frame_system::Pallet::<Test>::block_hash(0u64).as_fixed_bytes()
}

/// Canonical client-side recipients hash: `blake2_256` of the SCALE-encoded
/// `Vec<MultiSigner>` in submission order, as `sc-hop` must compute it.
fn recipients_hash_of(keys: &[Sr25519Keyring]) -> H256 {
	let raw: Vec<MultiSigner> = keys.iter().map(|k| MultiSigner::Sr25519(k.public())).collect();
	H256::from(blake2_256(&raw.encode()))
}

// ---- Dispatch tests ----

#[test]
fn promote_succeeds_with_valid_data() {
	new_test_ext().execute_with(|| {
		System::run_to_block::<AllPalletsWithSystem>(1);
		frame_system::Pallet::<Test>::set_extrinsic_index(0);
		let data = vec![42u8; 100];
		let (signer, sig) = dummy_signer_and_sig();
		assert_ok!(HopPromotion::promote(authorized_origin(), signer, sig, 0, data));
	});
}

#[test]
fn promote_rejects_empty_data() {
	new_test_ext().execute_with(|| {
		System::run_to_block::<AllPalletsWithSystem>(1);
		frame_system::Pallet::<Test>::set_extrinsic_index(0);
		let (signer, sig) = dummy_signer_and_sig();
		assert_noop!(
			HopPromotion::promote(authorized_origin(), signer, sig, 0, vec![]),
			pallet_bulletin_transaction_storage::Error::<Test>::BadDataSize,
		);
	});
}

#[test]
fn promote_rejects_oversized_data() {
	new_test_ext().execute_with(|| {
		System::run_to_block::<AllPalletsWithSystem>(1);
		frame_system::Pallet::<Test>::set_extrinsic_index(0);
		let (signer, sig) = dummy_signer_and_sig();
		assert_noop!(
			HopPromotion::promote(
				authorized_origin(),
				signer,
				sig,
				0,
				vec![0u8; TEST_MAX_TRANSACTION_SIZE as usize + 1],
			),
			pallet_bulletin_transaction_storage::Error::<Test>::BadDataSize,
		);
	});
}

#[test]
fn promote_rejects_non_authorized_origins() {
	new_test_ext().execute_with(|| {
		System::run_to_block::<AllPalletsWithSystem>(1);
		let data = vec![42u8; 100];
		let (signer, sig) = dummy_signer_and_sig();
		assert_noop!(
			HopPromotion::promote(
				RuntimeOrigin::none(),
				signer.clone(),
				sig.clone(),
				0,
				data.clone(),
			),
			sp_runtime::traits::BadOrigin,
		);
		assert_noop!(
			HopPromotion::promote(
				RuntimeOrigin::signed(Sr25519Keyring::Alice.to_account_id()),
				signer.clone(),
				sig.clone(),
				0,
				data.clone(),
			),
			sp_runtime::traits::BadOrigin,
		);
		assert_noop!(
			HopPromotion::promote(RuntimeOrigin::root(), signer, sig, 0, data),
			sp_runtime::traits::BadOrigin,
		);
	});
}

// ---- Authorize closure: source / data size ----

#[test]
fn authorize_rejects_external_source() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		let data = vec![1u8; 100];
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, TEST_TIMESTAMP_MS);
		let call = make_promote_call(data, signer, sig, TEST_TIMESTAMP_MS);
		assert_eq!(
			call.authorize(TransactionSource::External),
			Some(Err(InvalidTransaction::Call.into())),
		);
	});
}

#[test]
fn authorize_rejects_empty_data() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &[], TEST_TIMESTAMP_MS);
		let call = make_promote_call(vec![], signer, sig, TEST_TIMESTAMP_MS);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(pallet_bulletin_transaction_storage::BAD_DATA_SIZE.into())),
		);
	});
}

#[test]
fn authorize_rejects_oversized_data() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		let data = vec![0u8; TEST_MAX_TRANSACTION_SIZE as usize + 1];
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, TEST_TIMESTAMP_MS);
		let call = make_promote_call(data, signer, sig, TEST_TIMESTAMP_MS);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(pallet_bulletin_transaction_storage::BAD_DATA_SIZE.into())),
		);
	});
}

// ---- Authorize closure: signature, account, timestamp ----

#[test]
fn authorize_accepts_valid_signature_and_active_auth() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, TEST_TIMESTAMP_MS);
		let call = make_promote_call(data, signer, sig, TEST_TIMESTAMP_MS);
		assert!(matches!(call.authorize(TransactionSource::Local), Some(Ok(_))));
	});
}

#[test]
fn authorize_rejects_bad_signature() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		// Sign different data, then submit with the original data.
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &[7u8; 50], TEST_TIMESTAMP_MS);
		let call = make_promote_call(data, signer, sig, TEST_TIMESTAMP_MS);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadProof.into())),
		);
	});
}

#[test]
fn authorize_rejects_signer_mismatch() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		// Bob signs, but the call advertises Alice as the signer.
		let bob_sig =
			Sr25519Keyring::Bob.sign(&signing_payload(&blake2_256(&data), TEST_TIMESTAMP_MS));
		let call = make_promote_call(
			data,
			MultiSigner::Sr25519(Sr25519Keyring::Alice.public()),
			MultiSignature::Sr25519(bob_sig),
			TEST_TIMESTAMP_MS,
		);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadProof.into())),
		);
	});
}

#[test]
fn authorize_rejects_unauthorized_account() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		// Note: no authorize_account for Alice.
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, TEST_TIMESTAMP_MS);
		let call = make_promote_call(data, signer, sig, TEST_TIMESTAMP_MS);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadSigner.into())),
		);
	});
}

#[test]
fn authorize_accepts_fully_consumed_unexpired_authorization() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);
		frame_system::Pallet::<Test>::set_extrinsic_index(0);

		let alice = Sr25519Keyring::Alice.to_account_id();
		let data = vec![1u8; 100];
		// Authorize exactly enough for one store call, then spend it.
		authorize_account(alice.clone(), 1, data.len() as u64);
		let store_call =
			pallet_bulletin_transaction_storage::Call::<Test>::store { data: data.clone() };
		assert_ok!(pallet_bulletin_transaction_storage::Pallet::<Test>::pre_dispatch_signed(
			&alice,
			&store_call,
		));

		// Allowance is fully spent but the entry is still in storage and unexpired,
		// so HOP promotion is still permitted.
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, TEST_TIMESTAMP_MS);
		let call = make_promote_call(data, signer, sig, TEST_TIMESTAMP_MS);
		assert!(matches!(call.authorize(TransactionSource::Local), Some(Ok(_))));
	});
}

#[test]
fn authorize_rejects_expired_account_authorization() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		// Slot expiry runs on the relay clock: move it past the mock's
		// `DefaultAuthorizationWindow` (10 relay blocks from relay block 1).
		run_to_block(20);
		MockRelayBlockNumber::set(&20);

		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, TEST_TIMESTAMP_MS);
		let call = make_promote_call(data, signer, sig, TEST_TIMESTAMP_MS);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadSigner.into())),
		);
	});
}

#[test]
fn authorize_rejects_timestamp_too_old() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let stale_ts = TEST_TIMESTAMP_MS - TEST_SUBMIT_TIMESTAMP_TOLERANCE_MS - 1;
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, stale_ts);
		let call = make_promote_call(data, signer, sig, stale_ts);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::Stale.into())),
		);
	});
}

#[test]
fn authorize_rejects_timestamp_too_far_in_future() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let future_ts = TEST_TIMESTAMP_MS + TEST_SUBMIT_TIMESTAMP_TOLERANCE_MS + 1;
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, future_ts);
		let call = make_promote_call(data, signer, sig, future_ts);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::Stale.into())),
		);
	});
}

#[test]
fn authorize_accepts_timestamp_at_window_boundary() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let edge_ts = TEST_TIMESTAMP_MS - TEST_SUBMIT_TIMESTAMP_TOLERANCE_MS;
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, edge_ts);
		let call = make_promote_call(data, signer, sig, edge_ts);
		assert!(matches!(call.authorize(TransactionSource::Local), Some(Ok(_))));
	});
}

#[test]
fn authorize_rejects_signature_for_different_timestamp() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		// Sign for one timestamp, submit with another (both within window).
		let signed_ts = TEST_TIMESTAMP_MS;
		let claimed_ts = TEST_TIMESTAMP_MS - 1_000;
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, signed_ts);
		let call = make_promote_call(data, signer, sig, claimed_ts);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadProof.into())),
		);
	});
}

#[test]
fn authorize_valid_transaction_properties() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, TEST_TIMESTAMP_MS);
		let call = make_promote_call(data.clone(), signer, sig, TEST_TIMESTAMP_MS);
		let (valid_tx, weight) = call.authorize(TransactionSource::Local).unwrap().unwrap();
		let params = PromoteTxParams::get();
		assert_eq!(valid_tx.priority, params.priority);
		assert_eq!(valid_tx.longevity, params.longevity);
		assert!(!valid_tx.propagate);
		assert_eq!(weight, frame_support::weights::Weight::zero());
		let hash = sp_io::hashing::blake2_256(&data);
		let expected_tag = (params.tag_prefix, hash).encode();
		assert!(valid_tx.provides.contains(&expected_tag));
	});
}

/// The passing direction is covered by the runtime-generated integrity test.
#[test]
#[should_panic(expected = "must be strictly below store priority")]
fn integrity_test_rejects_promote_priority_at_or_above_store() {
	new_test_ext().execute_with(|| {
		StoreTxParams::set(ValidTransactionParams {
			priority: PromoteTxParams::get().priority,
			..StoreTxParams::get()
		});
		HopPromotion::integrity_test();
	});
}

#[test]
#[should_panic(expected = "PromoteTxParams and StoreTxParams must not share the tag prefix")]
fn integrity_test_rejects_promote_prefix_shared_with_store() {
	new_test_ext().execute_with(|| {
		StoreTxParams::set(ValidTransactionParams {
			tag_prefix: PromoteTxParams::get().tag_prefix,
			..StoreTxParams::get()
		});
		HopPromotion::integrity_test();
	});
}

// ---- V2 authorize closure ----

#[test]
fn authorize_v2_accepts_valid_signature_with_recipients_and_genesis_hash() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let recipients_hash = recipients_hash_of(&[Sr25519Keyring::Bob, Sr25519Keyring::Charlie]);
		let genesis = current_genesis_hash();
		let (signer, sig) = signed_by_v2(
			Sr25519Keyring::Alice,
			&data,
			TEST_TIMESTAMP_MS,
			&recipients_hash,
			&genesis,
		);
		let call = make_promote_v2_call(data, signer, sig, TEST_TIMESTAMP_MS, recipients_hash);
		assert!(matches!(call.authorize(TransactionSource::Local), Some(Ok(_))));
	});
}

#[test]
fn authorize_v2_rejects_empty_recipients_hash() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		// Even a valid signature over the empty recipients list must be refused:
		// `sc-hop` rejects zero-recipient submissions, and the chain mirrors it.
		let recipients_hash = recipients_hash_of(&[]);
		let genesis = current_genesis_hash();
		let (signer, sig) = signed_by_v2(
			Sr25519Keyring::Alice,
			&data,
			TEST_TIMESTAMP_MS,
			&recipients_hash,
			&genesis,
		);
		let call = make_promote_v2_call(data, signer, sig, TEST_TIMESTAMP_MS, recipients_hash);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(crate::NO_RECIPIENTS.into())),
		);
	});
}

#[test]
fn authorize_v2_rejects_wrong_genesis_hash() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let recipients_hash = recipients_hash_of(&[Sr25519Keyring::Bob]);
		// Sign against a fake genesis (simulating a different chain), submit on this chain.
		let fake_genesis = [0xAB; 32];
		let (signer, sig) = signed_by_v2(
			Sr25519Keyring::Alice,
			&data,
			TEST_TIMESTAMP_MS,
			&recipients_hash,
			&fake_genesis,
		);
		let call = make_promote_v2_call(data, signer, sig, TEST_TIMESTAMP_MS, recipients_hash);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadProof.into())),
		);
	});
}

#[test]
fn authorize_v2_rejects_tampered_recipients_added() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let genesis = current_genesis_hash();
		let signed_hash = recipients_hash_of(&[Sr25519Keyring::Bob]);
		let (signer, sig) =
			signed_by_v2(Sr25519Keyring::Alice, &data, TEST_TIMESTAMP_MS, &signed_hash, &genesis);
		// Submit with the hash of a list that has an extra recipient appended.
		let tampered = recipients_hash_of(&[Sr25519Keyring::Bob, Sr25519Keyring::Charlie]);
		let call = make_promote_v2_call(data, signer, sig, TEST_TIMESTAMP_MS, tampered);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadProof.into())),
		);
	});
}

#[test]
fn authorize_v2_rejects_tampered_recipients_reordered() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let genesis = current_genesis_hash();
		let signed_hash = recipients_hash_of(&[Sr25519Keyring::Bob, Sr25519Keyring::Charlie]);
		let (signer, sig) =
			signed_by_v2(Sr25519Keyring::Alice, &data, TEST_TIMESTAMP_MS, &signed_hash, &genesis);
		// Swap order on submission — same set, different SCALE encoding, different hash.
		let tampered = recipients_hash_of(&[Sr25519Keyring::Charlie, Sr25519Keyring::Bob]);
		let call = make_promote_v2_call(data, signer, sig, TEST_TIMESTAMP_MS, tampered);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadProof.into())),
		);
	});
}

#[test]
fn authorize_v2_rejects_v1_payload_format() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let recipients_hash = recipients_hash_of(&[Sr25519Keyring::Bob]);
		// Sign V1 payload (no genesis/recipients), then submit as V2 — verify must fail.
		let (signer, sig) = signed_by(Sr25519Keyring::Alice, &data, TEST_TIMESTAMP_MS);
		let call = make_promote_v2_call(data, signer, sig, TEST_TIMESTAMP_MS, recipients_hash);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadProof.into())),
		);
	});
}

#[test]
fn authorize_v2_rejects_bad_signature() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		authorize_account(Sr25519Keyring::Alice.to_account_id(), 1, data.len() as u64);

		let recipients_hash = recipients_hash_of(&[Sr25519Keyring::Bob]);
		let genesis = current_genesis_hash();
		// Sign for different data, submit with the canonical data — same V2 pattern as the V1 case.
		let (signer, sig) = signed_by_v2(
			Sr25519Keyring::Alice,
			&[7u8; 50],
			TEST_TIMESTAMP_MS,
			&recipients_hash,
			&genesis,
		);
		let call = make_promote_v2_call(data, signer, sig, TEST_TIMESTAMP_MS, recipients_hash);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadProof.into())),
		);
	});
}

#[test]
fn authorize_v2_rejects_unauthorized_account() {
	new_test_ext().execute_with(|| {
		set_now(TEST_TIMESTAMP_MS);
		System::run_to_block::<AllPalletsWithSystem>(1);

		let data = vec![1u8; 100];
		// No `authorize_account` — must fail with BadSigner before signature verify.
		let recipients_hash = recipients_hash_of(&[Sr25519Keyring::Bob]);
		let genesis = current_genesis_hash();
		let (signer, sig) = signed_by_v2(
			Sr25519Keyring::Alice,
			&data,
			TEST_TIMESTAMP_MS,
			&recipients_hash,
			&genesis,
		);
		let call = make_promote_v2_call(data, signer, sig, TEST_TIMESTAMP_MS, recipients_hash);
		assert_eq!(
			call.authorize(TransactionSource::Local),
			Some(Err(InvalidTransaction::BadSigner.into())),
		);
	});
}

#[test]
fn promote_v2_succeeds_with_authorized_origin() {
	new_test_ext().execute_with(|| {
		System::run_to_block::<AllPalletsWithSystem>(1);
		frame_system::Pallet::<Test>::set_extrinsic_index(0);
		let data = vec![42u8; 100];
		let (signer, sig) = dummy_signer_and_sig();
		let recipients_hash = recipients_hash_of(&[Sr25519Keyring::Bob]);
		assert_ok!(HopPromotion::promote_v2(
			authorized_origin(),
			signer,
			sig,
			0,
			recipients_hash,
			data,
		));
	});
}

/// Golden test vector pinning the V2 signing-payload byte layout. The same
/// vector must be replicated in `sc-hop` when the SDK side lands, so both
/// implementations provably produce identical bytes.
#[test]
fn signing_payload_v2_matches_test_vector() {
	let data = b"hop v2 test vector";
	let submit_timestamp: u64 = 1_700_000_000_000;
	let genesis_hash = [0x11u8; 32];
	// Recipients: well-known sr25519 keyring publics, in submission order.
	let recipients = vec![
		MultiSigner::Sr25519(Sr25519Keyring::Bob.public()),
		MultiSigner::Sr25519(Sr25519Keyring::Charlie.public()),
	];
	let recipients_hash = blake2_256(&recipients.encode());

	let payload =
		signing_payload_v2(&blake2_256(data), submit_timestamp, &genesis_hash, &recipients_hash);
	let expected = "0xd28b02d1bcab9cd7d7d683a3f8f051544a172d04faf3e94beeed37fed32495e1";
	assert_eq!(sp_core::bytes::to_hex(&payload, false), expected);
}

/// Pins the precomputed `EMPTY_RECIPIENTS_HASH` to the hash of the actual
/// SCALE encoding of an empty recipients list.
#[test]
fn empty_recipients_hash_matches_empty_list_encoding() {
	assert_eq!(crate::EMPTY_RECIPIENTS_HASH, blake2_256(&Vec::<MultiSigner>::new().encode()));
	assert_eq!(*recipients_hash_of(&[]).as_fixed_bytes(), crate::EMPTY_RECIPIENTS_HASH);
}
