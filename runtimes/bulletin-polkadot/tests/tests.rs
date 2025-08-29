use bulletin_polkadot_runtime as runtime;
use frame_support::{assert_noop, assert_ok};
use pallet_transaction_storage::{Call as TxCall, AuthorizationExtent, Error as TxError, BAD_DATA_SIZE};

#[test]
fn transaction_storage_runtime_sizes() {
	sp_io::TestExternalities::new(
		runtime::RuntimeGenesisConfig::default().build_storage().unwrap(),
	)
	.execute_with(|| {
		// Start at block 1
		runtime::System::set_block_number(1);
		runtime::TransactionStorage::on_initialize(1);

		let who: runtime::AccountId = Default::default();
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

		for size in sizes {
			let call = TxCall::<runtime::Runtime>::store { data: vec![0u8; size] };
			assert_ok!(runtime::TransactionStorage::pre_dispatch_signed(&who, &call));
			assert_ok!(runtime::RuntimeCall::TransactionStorage(call)
				.dispatch(runtime::RuntimeOrigin::none()));
		}

		// All authorization consumed
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
		let too_big_call = TxCall::<runtime::Runtime>::store { data: vec![0u8; oversize] };
		assert_noop!(
			runtime::TransactionStorage::pre_dispatch_signed(&who, &too_big_call),
			BAD_DATA_SIZE,
		);
		assert_noop!(
			runtime::RuntimeCall::TransactionStorage(too_big_call)
				.dispatch(runtime::RuntimeOrigin::none()),
			TxError::<runtime::Runtime>::BadDataSize,
		);
	});
}


