use bulletin_polkadot_runtime as runtime;
use frame_support::assert_ok;
use frame_support::traits::Hooks;
use pallet_transaction_storage::{Call as TxCall, AuthorizationExtent, BAD_DATA_SIZE};
use frame_support::dispatch::GetDispatchInfo;
use sp_core::{Pair, Encode};
use runtime::{RuntimeOrigin, System, Runtime, BuildStorage, RuntimeCall, UncheckedExtrinsic, TxExtension, SignedPayload, Executive, Hash, Header};
use sp_runtime::generic::Era;
use sp_runtime::traits::Header as _;
use sp_runtime::traits::SaturatedConversion;
use sp_keyring::Sr25519Keyring;
use sp_runtime::ApplyExtrinsicResult;
use pallet_transaction_storage::DEFAULT_MAX_TRANSACTION_SIZE;

fn advance_block() {
	Executive::finalize_block();
	let next_number = System::block_number() + 1;
	let header = Header::new(next_number, Default::default(), Default::default(), Default::default(), Default::default());
	Executive::initialize_block(&header);

	let slot = runtime::Babe::current_slot();
	let now = slot.saturated_into::<u64>() * runtime::SLOT_DURATION;
	runtime::Timestamp::set(RuntimeOrigin::none(), now).unwrap();
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
	let dispatch_info = call.get_dispatch_info();
	let xt = construct_extrinsic(account, call)?;
	let xt_len = xt.encode().len();
	log::info!(
		"Applying extrinsic: class={:?} pays_fee={:?} weight={:?} encoded_len={} bytes",
		dispatch_info.class,
		dispatch_info.pays_fee,
		dispatch_info.total_weight(),
		xt_len
	);
	Executive::apply_extrinsic(xt)
}

#[test]
fn transaction_storage_runtime_sizes() {
	let _ = sp_tracing::try_init_simple();
	sp_io::TestExternalities::new(
		runtime::RuntimeGenesisConfig::default().build_storage().unwrap(),
	)
	.execute_with(|| {
		// Start at block 1
		runtime::System::set_block_number(1);
		runtime::TransactionStorage::on_initialize(1);
		// set timestamp for block 1
		let slot = runtime::Babe::current_slot();
		let now = slot.saturated_into::<u64>() * runtime::SLOT_DURATION;
		runtime::Timestamp::set(RuntimeOrigin::none(), now).unwrap();


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

			advance_block();
		}

		assert_eq!(
			runtime::TransactionStorage::account_authorization_extent(who.clone()),
			AuthorizationExtent { transactions: 0, bytes: 0 },
		);

		// 11 MB should exceed MaxTransactionSize (8 MB) and fail
		let oversize: usize = DEFAULT_MAX_TRANSACTION_SIZE as usize + 1;//11 * 1024 * 1024;
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


