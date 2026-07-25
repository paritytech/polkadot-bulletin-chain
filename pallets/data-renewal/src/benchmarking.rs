// This file is part of Substrate.

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

//! Benchmarks for the data-renewal pallet.

// `super::*` re-exports the pallet's own imports (`AuthorizationScope`,
// `BlockTransactions`, `TransactionInfo`, `TransactionRef`,
// `ContentHash`, `RenewalData`, FRAME prelude, ...). Only the benchmark-only extras
// are imported explicitly here.
use super::{Pallet as DataRenewal, *};
use crate::extension::RenewalLeaves;
use alloc::vec;
use bulletin_transaction_storage_primitives::cids::{HashingAlgorithm, RAW_CODEC};
use pallet_bulletin_transaction_storage::{
	self as txs,
	extension::{StorageLeaves, ValidateAuthorizedCalls},
	pallet::Origin,
	Pallet as TransactionStorage,
};
use polkadot_sdk_frame::{
	benchmarking::prelude::*,
	deps::{
		frame_support::dispatch::{DispatchInfo, PostDispatchInfo},
		frame_system::{EventRecord, Pallet as System, RawOrigin},
		sp_runtime::traits::{AsTransactionAuthorizedOrigin, DispatchTransaction, Dispatchable},
	},
	traits::{AsSystemOriginSigner, IsSubType, OriginTrait},
};

type RuntimeCallOf<T> = <T as frame_system::Config>::RuntimeCall;

fn assert_last_event<T: Config>(generic_event: <T as Config>::RuntimeEvent) {
	let events = System::<T>::events();
	let system_event: <T as frame_system::Config>::RuntimeEvent = generic_event.into();
	let EventRecord { event, .. } = &events[events.len() - 1];
	assert_eq!(event, &system_event);
}

/// Advance the block, running both pallets' `on_initialize` / `on_finalize` so the
/// storage pallet flushes `BlockTransactions` into `Transactions[block]`.
fn run_to_block<T: Config>(n: BlockNumberFor<T>) {
	while System::<T>::block_number() < n {
		DataRenewal::<T>::on_finalize(System::<T>::block_number());
		TransactionStorage::<T>::on_finalize(System::<T>::block_number());
		System::<T>::on_finalize(System::<T>::block_number());
		System::<T>::set_block_number(System::<T>::block_number() + One::one());
		System::<T>::on_initialize(System::<T>::block_number());
		TransactionStorage::<T>::on_initialize(System::<T>::block_number());
	}
}

/// Origin rewritten to `Origin::Authorized` for `who`, as the extension would set
/// before dispatching a renewal call.
fn authorized_origin<T: Config>(who: T::AccountId) -> <T as frame_system::Config>::RuntimeOrigin
where
	<T as frame_system::Config>::RuntimeOrigin: From<Origin<T>>,
{
	Origin::<T>::Authorized { who: who.clone(), scope: AuthorizationScope::Account(who) }.into()
}

#[benchmarks(where
	T: Send + Sync,
	RuntimeCallOf<T>: IsSubType<txs::Call<T>>
		+ IsSubType<Call<T>>
		+ From<Call<T>>
		+ Dispatchable<Info = DispatchInfo, PostInfo = PostDispatchInfo>,
	T::RuntimeOrigin: OriginTrait
		+ AsSystemOriginSigner<T::AccountId>
		+ AsTransactionAuthorizedOrigin
		+ From<Origin<T>>
		+ Clone,
	<T::RuntimeOrigin as OriginTrait>::PalletsOrigin: From<Origin<T>> + TryInto<Origin<T>>,
)]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn renew() -> Result<(), BenchmarkError> {
		// Worst-case: `ContentHash` variant pays one extra `TransactionByContentHash` read.
		let caller: T::AccountId = whitelisted_caller();
		let data = vec![0u8; <T as txs::Config>::MaxTransactionSize::get() as usize];
		let content_hash = sp_io::hashing::blake2_256(&data);
		TransactionStorage::<T>::store(RawOrigin::None.into(), data)?;
		run_to_block::<T>(1u32.into());

		#[extrinsic_call]
		_(authorized_origin::<T>(caller.clone()), TransactionRef::ContentHash(content_hash));

		assert_last_event::<T>(
			Event::RenewalEnabled { content_hash, who: caller, recurring: false }.into(),
		);
		Ok(())
	}

	#[benchmark]
	fn force_renew() -> Result<(), BenchmarkError> {
		// Worst-case: `ContentHash` variant pays one extra `TransactionByContentHash` read.
		let data = vec![0u8; <T as txs::Config>::MaxTransactionSize::get() as usize];
		let content_hash = sp_io::hashing::blake2_256(&data);
		TransactionStorage::<T>::store(RawOrigin::None.into(), data)?;
		run_to_block::<T>(1u32.into());

		#[extrinsic_call]
		_(RawOrigin::None, TransactionRef::ContentHash(content_hash));

		assert_last_event::<T>(Event::Renewed { index: 0, content_hash }.into());
		Ok(())
	}

	#[benchmark]
	fn enable_auto_renew() -> Result<(), BenchmarkError> {
		let origin = <T as txs::Config>::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let caller: T::AccountId = whitelisted_caller();
		let data = vec![0u8; <T as txs::Config>::MaxTransactionSize::get() as usize];
		let content_hash = sp_io::hashing::blake2_256(&data);

		TransactionStorage::<T>::authorize_account(
			origin,
			caller.clone(),
			10,
			<T as txs::Config>::MaxTransactionSize::get() as u64 * 10,
		)
		.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;
		TransactionStorage::<T>::store(RawOrigin::None.into(), data)?;
		run_to_block::<T>(1u32.into());

		#[extrinsic_call]
		_(authorized_origin::<T>(caller.clone()), content_hash);

		assert_last_event::<T>(
			Event::RenewalEnabled { content_hash, who: caller, recurring: true }.into(),
		);
		Ok(())
	}

	#[benchmark]
	fn disable_auto_renew() -> Result<(), BenchmarkError> {
		let origin = <T as txs::Config>::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let caller: T::AccountId = whitelisted_caller();
		let data = vec![0u8; <T as txs::Config>::MaxTransactionSize::get() as usize];
		let content_hash = sp_io::hashing::blake2_256(&data);

		TransactionStorage::<T>::authorize_account(
			origin,
			caller.clone(),
			10,
			<T as txs::Config>::MaxTransactionSize::get() as u64 * 10,
		)
		.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;
		TransactionStorage::<T>::store(RawOrigin::None.into(), data)?;
		run_to_block::<T>(1u32.into());
		DataRenewal::<T>::enable_auto_renew(authorized_origin::<T>(caller.clone()), content_hash)
			.map_err(|_| BenchmarkError::Stop("unable to enable auto-renew"))?;
		// `enable_auto_renew` leaves the entry `paid: true`, which blocks signed
		// `disable_auto_renew`. Flip to the post-first-cycle state so the benchmark
		// measures the path real owners actually hit.
		Renewals::<T>::mutate(content_hash, |entry| {
			if let Some(data) = entry {
				data.paid = false;
			}
		});

		#[extrinsic_call]
		_(authorized_origin::<T>(caller.clone()), content_hash);

		assert_last_event::<T>(Event::AutoRenewalDisabled { content_hash, who: caller }.into());
		Ok(())
	}

	/// Per-call cost charged inside the combined extension's signed validation for a
	/// renewal leaf (auth lookup + `bytes_permanent` + `PermanentStorageUsed`).
	#[benchmark]
	fn validate_renew() -> Result<(), BenchmarkError> {
		let data = vec![0u8; <T as txs::Config>::MaxTransactionSize::get() as usize];
		TransactionStorage::<T>::store(RawOrigin::None.into(), data.clone())?;
		run_to_block::<T>(1u32.into());

		let origin = <T as txs::Config>::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let caller: T::AccountId = whitelisted_caller();
		let bytes_allowance = <T as txs::Config>::MaxTransactionSize::get() as u64 * 10;
		TransactionStorage::<T>::authorize_account(origin, caller.clone(), 0, bytes_allowance)
			.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;

		let ext = ValidateAuthorizedCalls::<T, (), (StorageLeaves<T>, RenewalLeaves<T>)>::default();
		let call: RuntimeCallOf<T> = Call::<T>::force_renew {
			entry: TransactionRef::Position { block: BlockNumberFor::<T>::zero(), index: 0 },
		}
		.into();
		let info = DispatchInfo::default();
		let len = 0_usize;

		#[block]
		{
			ext.test_run(RawOrigin::Signed(caller.clone()).into(), &call, &info, len, 0, |_| {
				Ok(().into())
			})
			.unwrap()
			.unwrap();
		}

		// `prepare` charged `data.len()` to the permanent-usage counter.
		let extent = TransactionStorage::<T>::account_authorization_extent(caller);
		assert_eq!(extent.extra.bytes_permanent, data.len() as u64);
		assert_eq!(extent.bytes_allowance, bytes_allowance);
		Ok(())
	}

	/// Drain `n` pending auto-renewals in the mandatory inherent. `paid: false`
	/// exercises the heavier per-cycle `check_authorization` charge path; distinct
	/// callers/hashes per item avoid storage-cache undercharging.
	#[benchmark]
	fn process_pending_renewals(
		n: Linear<0, { <T as txs::Config>::MaxBlockTransactions::get() }>,
	) -> Result<(), BenchmarkError> {
		System::<T>::set_block_number(1u32.into());
		// `do_renew_in_memory` indexes renewals against the current extrinsic.
		System::<T>::set_extrinsic_index(0);

		if n > 0 {
			let mut pending = PendingAutoRenewals::<T>::get();
			for i in 0..n {
				// Unique caller per item so each `check_authorization` hits a distinct
				// `Authorizations` key.
				let caller: T::AccountId = account("rn_caller", i, 0);
				let origin = <T as txs::Config>::Authorizer::try_successful_origin()
					.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
				TransactionStorage::<T>::authorize_account(
					origin,
					caller.clone(),
					10,
					<T as txs::Config>::MaxTransactionSize::get() as u64 * 10,
				)
				.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;

				let content_hash = sp_io::hashing::blake2_256(&i.to_le_bytes());
				let tx_info = TransactionInfo {
					chunk_root: Default::default(),
					size: 1,
					content_hash,
					hashing: HashingAlgorithm::Blake2b256,
					cid_codec: RAW_CODEC,
					extrinsic_index: 0,
					block_chunks: 0,
					meta: EntryKind::Store,
				};
				let renewal_data = RenewalData { account: caller, recurring: true, paid: false };
				pending
					.try_push((content_hash, tx_info, renewal_data))
					.map_err(|_| BenchmarkError::Stop("unable to push pending renewal"))?;
			}
			PendingAutoRenewals::<T>::put(&pending);
		}

		#[extrinsic_call]
		_(RawOrigin::None);

		assert!(PendingAutoRenewals::<T>::get().is_empty());
		assert_eq!(BlockTransactions::<T>::get().len() as u32, n);
		Ok(())
	}

	impl_benchmark_test_suite!(DataRenewal, crate::mock::new_test_ext(), crate::mock::Test);
}
