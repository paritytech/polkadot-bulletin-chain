use bulletin_polkadot_runtime as runtime;
use frame_support::{assert_noop, assert_ok};
use frame_support::traits::{Hooks, OnIdle};
use pallet_transaction_storage::{Call as TxCall, AuthorizationExtent, Error as TxError, BAD_DATA_SIZE};
use sp_core::{sr25519, Pair, Encode};
use runtime::{RuntimeOrigin, TransactionStorage, System, Runtime, BuildStorage, RuntimeCall, UncheckedExtrinsic, TxExtension, SignedPayload, Executive, Hash};
use sp_runtime::generic::Era;
use sp_keyring::Sr25519Keyring;
use sp_runtime::ApplyExtrinsicResult;
use crate::runtime::AllPalletsWithSystem;
use crate::runtime::Weight;
use sp_transaction_storage_proof::TransactionStorageProof;

pub fn run_to_block(n: u32, f: impl Fn() -> Option<TransactionStorageProof>) {
	while System::block_number() < n {
		if let Some(proof) = f() {
			TransactionStorage::check_proof(RuntimeOrigin::none(), proof).unwrap();
		}
		TransactionStorage::on_finalize(System::block_number());
		System::on_finalize(System::block_number());
		System::set_block_number(System::block_number() + 1);
		System::on_initialize(System::block_number());
		TransactionStorage::on_initialize(System::block_number());
	}
}

fn construct_extrinsic(
	sender: sp_core::sr25519::Pair,
	call: RuntimeCall,
) -> Result<UncheckedExtrinsic, sp_runtime::transaction_validity::TransactionValidityError> {

	let account_id = sp_runtime::AccountId32::from(sender.public());
	frame_system::BlockHash::<Runtime>::insert(0, Hash::default());
	let tx_ext: TxExtension = (
		frame_system::CheckNonZeroSender::<Runtime>::new(),
		frame_system::CheckSpecVersion::<Runtime>::new(),
		frame_system::CheckTxVersion::<Runtime>::new(),
		frame_system::CheckGenesis::<Runtime>::new(),
		frame_system::CheckEra::<Runtime>::from(Era::immortal()),
		frame_system::CheckNonce::<Runtime>::from(
			frame_system::Pallet::<Runtime>::account(&account_id).nonce,
		)
		.into(),
		frame_system::CheckWeight::<Runtime>::new(),
		runtime::ValidateSigned,
		runtime::BridgeRejectObsoleteHeadersAndMessages,
	)
		.into();
	let payload = SignedPayload::new(call.clone(), tx_ext.clone())?;
	let signature = payload.using_encoded(|e| sender.sign(e));
	Ok(UncheckedExtrinsic::new_signed(
		call,
		account_id.into(),
		runtime::Signature::Sr25519(signature),
		tx_ext,
	))
}

fn construct_and_apply_extrinsic(
	account: sp_core::sr25519::Pair,
	call: RuntimeCall,
) -> ApplyExtrinsicResult {
	let xt = construct_extrinsic(account, call)?;
	Executive::apply_extrinsic(xt)
}

#[test]
fn transaction_storage_runtime_sizes() {
	sp_io::TestExternalities::new(
		runtime::RuntimeGenesisConfig::default().build_storage().unwrap(),
	)
	.execute_with(|| {
		// Start at block 1
		runtime::System::set_block_number(1);
		runtime::TransactionStorage::on_initialize(1);

		let mut block_number: u32 = 1;

		let who: runtime::AccountId = sp_keyring::Sr25519Keyring::Alice.to_account_id();
		let sizes: [usize; 5] = [
			2000,            // 2 KB
			1 * 1024 * 1024, // 1 MB
			4 * 1024 * 1024, // 4 MB
			6 * 1024 * 1024, // 6 MB
			8 * 1024 * 1024, // 8 MB
		];
		let total_bytes: u64 = sizes.iter().map(|s| *s as u64).sum();

		assert_ok!(runtime::TransactionStorage::authorize_account(
			runtime::RuntimeOrigin::root(),
			who.clone(),
			sizes.len() as u32,
			total_bytes,
		));
		assert_eq!(
			runtime::TransactionStorage::account_authorization_extent(who.clone()),
			AuthorizationExtent { transactions: sizes.len() as u32, bytes: total_bytes },
		);

		let alice_pair = Sr25519Keyring::Alice.pair();
		for size in sizes {
			let call = RuntimeCall::TransactionStorage(TxCall::<runtime::Runtime>::store { data: vec![0u8; size] });
			let res = construct_and_apply_extrinsic(alice_pair.clone(), call);
			assert!(res.is_ok(), "Failed at size={} bytes: {:?}", size, res);

			// End current block and start the next one so each tx is in a separate block
			block_number += 1;
			run_to_block(block_number, || None);
		}

		assert_eq!(
			runtime::TransactionStorage::account_authorization_extent(who.clone()),
			AuthorizationExtent { transactions: 0, bytes: 0 },
		);

		// 11 MB should exceed MaxTransactionSize (8 MB) and fail
		let oversize: usize = 11 * 1024 * 1024;
		assert_ok!(runtime::TransactionStorage::authorize_account(
			runtime::RuntimeOrigin::root(),
			who.clone(),
			1,
			oversize as u64,
		));
		let too_big_call = RuntimeCall::TransactionStorage(TxCall::<runtime::Runtime>::store { data: vec![0u8; oversize] });
		let res = construct_and_apply_extrinsic(alice_pair, too_big_call);
		assert_eq!(
			res,
			Err(BAD_DATA_SIZE.into())
		);
	});
}


