// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

#![cfg(test)]

use bulletin_westend_runtime::{
	Balances, Executive, Runtime, RuntimeCall, RuntimeGenesisConfig, RuntimeOrigin, System,
	TransactionStorage, TxExtension, UncheckedExtrinsic,
};
use frame_support::{assert_ok, pallet_prelude::Hooks, traits::fungible::Mutate};
use pallet_bulletin_data_renewal::Call as DataRenewalCall;
use pallet_bulletin_transaction_storage::{
	extension::NonceCheckWithQueueBonus, Call as TxStorageCall, Config as TxStorageConfig,
	TransactionRef,
};
use parachains_common::{Balance, Hash, Nonce};
use sp_core::{Encode, Pair};
use sp_keyring::Sr25519Keyring;
use sp_runtime::{
	generic::{Era, SignedPayload},
	traits::TransactionExtension,
	transaction_validity::{
		InvalidTransaction, TransactionPriority, TransactionSource, TransactionValidityError,
	},
	BuildStorage, MultiAddress, MultiSignature, MultiSigner,
};

const SMALL_STORE_BYTES: usize = 64 * 1024;
const LARGE_STORE_BYTES: usize = 2 * 1024 * 1024;
const LARGE_ALLOWANCE_BYTES: u64 = 1024 * 1024 * 1024;
const SMALL_ALLOWANCE_BYTES: u64 = 1024;
const ALLOWANCE_TRANSACTIONS: u32 = 1000;
const FUNDS: Balance = 1_000_000_000_000;
const TRANSFER_VALUE: Balance = 10_000_000_000;
const TIP: Balance = 1_000_000_000;
const SPAMMER_BACKLOG: Nonce = 50;
const PAST_TRANSACTIONS: Nonce = 5;

fn externalities() -> sp_io::TestExternalities {
	let mut genesis = RuntimeGenesisConfig::default();
	genesis.transaction_storage.allowed_authorizers = vec![(
		Sr25519Keyring::BobStash.to_account_id(),
		ALLOWANCE_TRANSACTIONS,
		LARGE_ALLOWANCE_BYTES,
	)];
	sp_io::TestExternalities::new(genesis.build_storage().unwrap())
}

fn next_block() {
	let current = System::block_number();
	<TransactionStorage as Hooks<_>>::on_finalize(current);
	<System as Hooks<_>>::on_finalize(current);
	System::set_block_number(current + 1);
	frame_system::BlockWeight::<Runtime>::kill();
	frame_system::BlockSize::<Runtime>::kill();
	<System as Hooks<_>>::on_initialize(current + 1);
	<TransactionStorage as Hooks<_>>::on_initialize(current + 1);
}

fn account_authorization(account: Sr25519Keyring, bytes: u64) {
	assert_ok!(TransactionStorage::authorize_account(
		RuntimeOrigin::root(),
		account.to_account_id(),
		ALLOWANCE_TRANSACTIONS,
		bytes,
	));
}

fn funding(account: Sr25519Keyring) {
	assert_ok!(Balances::mint_into(&account.to_account_id(), FUNDS));
}

fn tx_extension(nonce: Nonce, tip: Balance) -> TxExtension {
	cumulus_pallet_weight_reclaim::StorageWeightReclaim::<Runtime, _>::from((
		frame_system::AuthorizeCall::<Runtime>::new(),
		frame_system::CheckNonZeroSender::<Runtime>::new(),
		frame_system::CheckSpecVersion::<Runtime>::new(),
		frame_system::CheckTxVersion::<Runtime>::new(),
		frame_system::CheckGenesis::<Runtime>::new(),
		frame_system::CheckEra::<Runtime>::from(Era::immortal()),
		NonceCheckWithQueueBonus::<Runtime>::new(nonce),
		frame_system::CheckWeight::<Runtime>::new(),
		pallet_skip_feeless_payment::SkipCheckIfFeeless::from(
			pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(tip),
		),
		pallet_bulletin_transaction_storage::extension::ValidateAuthorizedCalls::<
			Runtime,
			bulletin_westend_runtime::storage::StorageCallInspector,
			(
				pallet_bulletin_transaction_storage::extension::StorageLeaves<Runtime>,
				pallet_bulletin_data_renewal::extension::RenewalLeaves<Runtime>,
			),
		>::default(),
		pallet_bulletin_transaction_storage::extension::AllowanceBasedPriority::<Runtime>::default(
		),
		frame_metadata_hash_extension::CheckMetadataHash::<Runtime>::new(false),
	))
}

fn signed_extrinsic(
	sender: Sr25519Keyring,
	call: RuntimeCall,
	nonce: Nonce,
	tip: Balance,
) -> UncheckedExtrinsic {
	frame_system::BlockHash::<Runtime>::insert(0, Hash::default());
	let extension = tx_extension(nonce, tip);
	let payload =
		SignedPayload::new(call.clone(), extension.clone()).expect("invalid extension data");
	let signature = payload.using_encoded(|bytes| sender.pair().sign(bytes));
	UncheckedExtrinsic::new_signed(
		call,
		sender.to_account_id().into(),
		MultiSignature::Sr25519(signature),
		extension,
	)
}

fn general_extrinsic(call: RuntimeCall) -> UncheckedExtrinsic {
	frame_system::BlockHash::<Runtime>::insert(0, Hash::default());
	UncheckedExtrinsic::new_transaction(call, tx_extension(0, 0))
}

fn inclusion(sender: Sr25519Keyring, call: RuntimeCall) {
	let nonce = System::account_nonce(sender.to_account_id());
	let outcome = Executive::apply_extrinsic(signed_extrinsic(sender, call, nonce, 0))
		.expect("invalid transaction");
	assert_ok!(outcome);
}

fn pool_priority(xt: UncheckedExtrinsic, source: TransactionSource) -> TransactionPriority {
	Executive::validate_transaction(source, xt, Hash::default())
		.expect("invalid transaction")
		.priority
}

fn queued_priority(
	sender: Sr25519Keyring,
	call: RuntimeCall,
	queue_position: Nonce,
	tip: Balance,
) -> TransactionPriority {
	let nonce = System::account_nonce(sender.to_account_id()) + queue_position;
	pool_priority(signed_extrinsic(sender, call, nonce, tip), TransactionSource::External)
}

fn store_call(size: usize, fill: u8) -> RuntimeCall {
	RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store { data: vec![fill; size] })
}

fn transfer_call() -> RuntimeCall {
	RuntimeCall::Balances(pallet_balances::Call::<Runtime>::transfer_keep_alive {
		dest: MultiAddress::Id(Sr25519Keyring::Two.to_account_id()),
		value: TRANSFER_VALUE,
	})
}

fn promotion_call(submitter: Sr25519Keyring, data: Vec<u8>) -> RuntimeCall {
	let submit_timestamp = pallet_timestamp::Pallet::<Runtime>::get();
	let payload = pallet_bulletin_hop_promotion::signing_payload(
		&sp_io::hashing::blake2_256(&data),
		submit_timestamp,
	);
	RuntimeCall::HopPromotion(pallet_bulletin_hop_promotion::Call::<Runtime>::promote {
		signer: MultiSigner::Sr25519(submitter.public()),
		signature: MultiSignature::Sr25519(submitter.sign(&payload)),
		submit_timestamp,
		data,
	})
}

#[test]
fn priority_ladder_across_call_types() {
	externalities().execute_with(|| {
		let expired = Sr25519Keyring::One;
		let in_budget = Sr25519Keyring::Eve;
		let over_budget = Sr25519Keyring::Ferdie;
		let renewer = Sr25519Keyring::Dave;
		let authorizer = Sr25519Keyring::BobStash;
		let payer = Sr25519Keyring::Bob;
		let submitter = Sr25519Keyring::Charlie;

		account_authorization(expired, LARGE_ALLOWANCE_BYTES);
		System::set_block_number(
			System::block_number() + <Runtime as TxStorageConfig>::AuthorizationPeriod::get(),
		);
		next_block();
		for account in [in_budget, renewer, submitter] {
			account_authorization(account, LARGE_ALLOWANCE_BYTES);
		}
		account_authorization(over_budget, SMALL_ALLOWANCE_BYTES);
		funding(payer);
		inclusion(renewer, store_call(SMALL_STORE_BYTES, 1));
		let stored_block = System::block_number();
		next_block();
		let preimage = vec![2u8; SMALL_STORE_BYTES];
		assert_ok!(TransactionStorage::authorize_preimage(
			RuntimeOrigin::root(),
			sp_io::hashing::blake2_256(&preimage),
			preimage.len() as u64,
		));

		let cleanup = pool_priority(
			UncheckedExtrinsic::new_bare(RuntimeCall::TransactionStorage(
				TxStorageCall::<Runtime>::remove_expired_account_authorization {
					who: expired.to_account_id(),
				},
			)),
			TransactionSource::External,
		);
		let in_budget_store = queued_priority(in_budget, store_call(SMALL_STORE_BYTES, 3), 0, 0);
		let over_budget_store =
			queued_priority(over_budget, store_call(SMALL_STORE_BYTES, 4), 0, 0);
		let renew = queued_priority(
			renewer,
			RuntimeCall::DataRenewal(DataRenewalCall::<Runtime>::force_renew {
				entry: TransactionRef::Position { block: stored_block, index: 0 },
			}),
			0,
			0,
		);
		let authorize = queued_priority(
			authorizer,
			RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::authorize_account {
				who: Sr25519Keyring::Two.to_account_id(),
				transactions: 1,
				bytes: SMALL_ALLOWANCE_BYTES,
			}),
			0,
			0,
		);
		let preimage_store = pool_priority(
			UncheckedExtrinsic::new_bare(store_call(SMALL_STORE_BYTES, 2)),
			TransactionSource::External,
		);
		let transfer = queued_priority(payer, transfer_call(), 0, 0);
		let promotion = pool_priority(
			general_extrinsic(promotion_call(submitter, vec![5u8; SMALL_STORE_BYTES])),
			TransactionSource::Local,
		);

		assert!(cleanup > in_budget_store, "cleanup must rank above every store");
		assert!(
			in_budget_store > over_budget_store,
			"the allowance boost must lift in-budget stores"
		);
		assert_eq!(renew, over_budget_store, "renew must rank level with an over-budget store");
		assert_eq!(
			authorize, over_budget_store,
			"authorize_account must rank level with an over-budget store"
		);
		assert_eq!(
			preimage_store, over_budget_store,
			"a preimage-authorized store must rank level with an over-budget store"
		);
		assert!(over_budget_store > transfer, "fee-paying calls must rank below every store");
		assert!(transfer > promotion, "HOP promotion must rank lowest");
	});
}

#[test]
fn in_budget_store_above_over_budget_store_at_any_queue_position() {
	externalities().execute_with(|| {
		let in_budget = Sr25519Keyring::Eve;
		let over_budget = Sr25519Keyring::Ferdie;
		account_authorization(in_budget, LARGE_ALLOWANCE_BYTES);
		account_authorization(over_budget, SMALL_ALLOWANCE_BYTES);

		let in_budget_far_back =
			queued_priority(in_budget, store_call(SMALL_STORE_BYTES, 0), Nonce::MAX / 2, 0);
		let over_budget_first =
			queued_priority(over_budget, store_call(SMALL_STORE_BYTES, 0), 0, 0);

		assert!(in_budget_far_back > over_budget_first);
	});
}

#[test]
fn queue_position_order_without_size_effect() {
	externalities().execute_with(|| {
		let large_sender = Sr25519Keyring::Eve;
		let small_sender = Sr25519Keyring::Ferdie;
		for sender in [large_sender, small_sender] {
			account_authorization(sender, LARGE_ALLOWANCE_BYTES);
		}

		let large_first = queued_priority(large_sender, store_call(LARGE_STORE_BYTES, 0), 0, 0);
		let large_second = queued_priority(large_sender, store_call(LARGE_STORE_BYTES, 0), 1, 0);
		let small_first = queued_priority(small_sender, store_call(SMALL_STORE_BYTES, 0), 0, 0);
		let small_second = queued_priority(small_sender, store_call(SMALL_STORE_BYTES, 0), 1, 0);

		assert_eq!(large_first, small_first);
		assert_eq!(large_second, small_second);
		assert!(small_second < small_first);
	});
}

#[test]
fn newcomer_first_transaction_above_spammer_backlog() {
	externalities().execute_with(|| {
		let spammer = Sr25519Keyring::Eve;
		let newcomer = Sr25519Keyring::Ferdie;
		for account in [spammer, newcomer] {
			account_authorization(account, LARGE_ALLOWANCE_BYTES);
		}

		let newcomer_first = queued_priority(newcomer, store_call(SMALL_STORE_BYTES, 0), 0, 0);

		assert_eq!(
			queued_priority(spammer, store_call(SMALL_STORE_BYTES, 0), 0, 0),
			newcomer_first
		);
		for queue_position in 1..=SPAMMER_BACKLOG {
			assert!(
				queued_priority(spammer, store_call(SMALL_STORE_BYTES, 0), queue_position, 0) <
					newcomer_first
			);
		}
	});
}

#[test]
fn past_transactions_without_effect_on_next_transaction() {
	externalities().execute_with(|| {
		next_block();
		let veteran = Sr25519Keyring::Eve;
		let newcomer = Sr25519Keyring::Ferdie;
		for account in [veteran, newcomer] {
			account_authorization(account, LARGE_ALLOWANCE_BYTES);
		}
		for fill in 0..PAST_TRANSACTIONS as u8 {
			inclusion(veteran, store_call(SMALL_STORE_BYTES, fill));
		}

		assert_eq!(System::account_nonce(veteran.to_account_id()), PAST_TRANSACTIONS);
		assert_eq!(
			queued_priority(veteran, store_call(SMALL_STORE_BYTES, 9), 0, 0),
			queued_priority(newcomer, store_call(SMALL_STORE_BYTES, 9), 0, 0),
		);
	});
}

#[test]
fn over_budget_signers_round_robin() {
	externalities().execute_with(|| {
		let first_spammer = Sr25519Keyring::Eve;
		let second_spammer = Sr25519Keyring::Ferdie;
		for spammer in [first_spammer, second_spammer] {
			account_authorization(spammer, SMALL_ALLOWANCE_BYTES);
		}
		let priority = |spammer, queue_position| {
			queued_priority(spammer, store_call(SMALL_STORE_BYTES, 0), queue_position, 0)
		};

		assert_eq!(priority(first_spammer, 0), priority(second_spammer, 0));
		assert_eq!(priority(first_spammer, 1), priority(second_spammer, 1));
		assert!(priority(first_spammer, 1) < priority(second_spammer, 0));
	});
}

#[test]
fn store_after_allowance_exhaustion_below_in_budget_store() {
	externalities().execute_with(|| {
		next_block();
		let exhausted = Sr25519Keyring::Eve;
		let in_budget = Sr25519Keyring::Ferdie;
		account_authorization(exhausted, SMALL_STORE_BYTES as u64);
		account_authorization(in_budget, LARGE_ALLOWANCE_BYTES);

		assert_eq!(
			queued_priority(exhausted, store_call(SMALL_STORE_BYTES, 1), 0, 0),
			queued_priority(in_budget, store_call(SMALL_STORE_BYTES, 1), 0, 0),
		);

		inclusion(exhausted, store_call(SMALL_STORE_BYTES, 2));

		assert!(
			queued_priority(exhausted, store_call(SMALL_STORE_BYTES, 3), 0, 0) <
				queued_priority(in_budget, store_call(SMALL_STORE_BYTES, 3), Nonce::MAX / 2, 0)
		);
	});
}

#[test]
fn store_tip_without_effect() {
	externalities().execute_with(|| {
		let sender = Sr25519Keyring::Eve;
		account_authorization(sender, LARGE_ALLOWANCE_BYTES);

		assert_eq!(
			queued_priority(sender, store_call(SMALL_STORE_BYTES, 0), 0, TIP),
			queued_priority(sender, store_call(SMALL_STORE_BYTES, 0), 0, 0),
		);
	});
}

#[test]
fn transfer_tip_above_queue_position() {
	externalities().execute_with(|| {
		let tipping_payer = Sr25519Keyring::Bob;
		let payer = Sr25519Keyring::Charlie;
		for account in [tipping_payer, payer] {
			funding(account);
		}

		let untipped_first = queued_priority(payer, transfer_call(), 0, 0);

		assert!(queued_priority(tipping_payer, transfer_call(), 1, TIP) > untipped_first);
		assert!(queued_priority(payer, transfer_call(), 1, 0) < untipped_first);
	});
}

#[test]
fn stale_nonce_rejection() {
	externalities().execute_with(|| {
		let sender = Sr25519Keyring::Eve;
		account_authorization(sender, LARGE_ALLOWANCE_BYTES);
		inclusion(sender, store_call(SMALL_STORE_BYTES, 1));

		assert_eq!(
			Executive::validate_transaction(
				TransactionSource::External,
				signed_extrinsic(sender, store_call(SMALL_STORE_BYTES, 2), 0, 0),
				Hash::default(),
			),
			Err(TransactionValidityError::Invalid(InvalidTransaction::Stale)),
		);
	});
}

#[test]
fn check_nonce_wire_format_compatibility() {
	let nonce: Nonce = 7;
	assert_eq!(
		NonceCheckWithQueueBonus::<Runtime>::new(nonce).encode(),
		frame_system::CheckNonce::<Runtime>::from(nonce).encode(),
	);
	assert_eq!(
		<NonceCheckWithQueueBonus<Runtime> as TransactionExtension<RuntimeCall>>::IDENTIFIER,
		<frame_system::CheckNonce<Runtime> as TransactionExtension<RuntimeCall>>::IDENTIFIER,
	);
	assert_eq!(
		scale_info::meta_type::<NonceCheckWithQueueBonus<Runtime>>(),
		scale_info::meta_type::<frame_system::CheckNonce<Runtime>>(),
	);
}

#[test]
fn check_nonce_weight_equality() {
	let call = RuntimeCall::System(frame_system::Call::remark { remark: vec![] });
	assert_eq!(
		NonceCheckWithQueueBonus::<Runtime>::new(0).weight(&call),
		frame_system::CheckNonce::<Runtime>::from(0).weight(&call),
	);
}
