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

//! Benchmarks for transaction-storage Pallet


use super::{Pallet as TransactionStorage, *};
use alloc::vec;
use polkadot_sdk_frame::{
	benchmarking::prelude::*,
	deps::frame_system::{EventRecord, Pallet as System, RawOrigin},
};
use sp_transaction_storage_proof::TransactionStorageProof;

// Proof generated from max size storage:
// ```
// let mut transactions = Vec::new();
// let tx_size = DEFAULT_MAX_TRANSACTION_SIZE;
// for _ in 0..DEFAULT_MAX_BLOCK_TRANSACTIONS {
//   transactions.push(vec![0; tx_size]);
// }
// let content_hash = vec![0; 32];
// build_proof(content_hash.as_slice(), transactions).unwrap().encode()
// ```
// while hardforcing target chunk key in `build_proof` to [22, 21, 1, 0].
const PROOF: &str = "\
	0104000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	00000000000000000000000000000000000000000000000000000000000014cd0780ffff8030\
	2eb0a6d2f63b834d15f1e729d1c1004657e3048cf206d697eeb153f61a30ba0080302eb0a6d2\
	f63b834d15f1e729d1c1004657e3048cf206d697eeb153f61a30ba80302eb0a6d2f63b834d15\
	f1e729d1c1004657e3048cf206d697eeb153f61a30ba80302eb0a6d2f63b834d15f1e729d1c1\
	004657e3048cf206d697eeb153f61a30ba80302eb0a6d2f63b834d15f1e729d1c1004657e304\
	8cf206d697eeb153f61a30ba80302eb0a6d2f63b834d15f1e729d1c1004657e3048cf206d697\
	eeb153f61a30ba80302eb0a6d2f63b834d15f1e729d1c1004657e3048cf206d697eeb153f61a\
	30ba80302eb0a6d2f63b834d15f1e729d1c1004657e3048cf206d697eeb153f61a30ba80302e\
	b0a6d2f63b834d15f1e729d1c1004657e3048cf206d697eeb153f61a30ba80302eb0a6d2f63b\
	834d15f1e729d1c1004657e3048cf206d697eeb153f61a30ba80302eb0a6d2f63b834d15f1e7\
	29d1c1004657e3048cf206d697eeb153f61a30ba80302eb0a6d2f63b834d15f1e729d1c10046\
	57e3048cf206d697eeb153f61a30ba80302eb0a6d2f63b834d15f1e729d1c1004657e3048cf2\
	06d697eeb153f61a30ba80302eb0a6d2f63b834d15f1e729d1c1004657e3048cf206d697eeb1\
	53f61a30ba80302eb0a6d2f63b834d15f1e729d1c1004657e3048cf206d697eeb153f61a30ba\
	bd058077778010fd81bc1359802f0b871aeb95e4410a8ec92b93af10ea767a2027cf4734e8de\
	808da338e6b722f7bf2051901bd5bccee5e71d5cf6b1faff338ad7120b0256c28380221ce17f\
	19117affa96e077905fe48a99723a065969c638593b7d9ab57b538438010fd81bc1359802f0b\
	871aeb95e4410a8ec92b93af10ea767a2027cf4734e8de808da338e6b722f7bf2051901bd5bc\
	cee5e71d5cf6b1faff338ad7120b0256c283008010fd81bc1359802f0b871aeb95e4410a8ec9\
	2b93af10ea767a2027cf4734e8de808da338e6b722f7bf2051901bd5bccee5e71d5cf6b1faff\
	338ad7120b0256c28380221ce17f19117affa96e077905fe48a99723a065969c638593b7d9ab\
	57b538438010fd81bc1359802f0b871aeb95e4410a8ec92b93af10ea767a2027cf4734e8de80\
	8da338e6b722f7bf2051901bd5bccee5e71d5cf6b1faff338ad7120b0256c28380221ce17f19\
	117affa96e077905fe48a99723a065969c638593b7d9ab57b53843cd0780ffff804509f59593\
	fd47b1a97189127ba65a5649cfb0346637f9836e155eaf891a939c00804509f59593fd47b1a9\
	7189127ba65a5649cfb0346637f9836e155eaf891a939c804509f59593fd47b1a97189127ba6\
	5a5649cfb0346637f9836e155eaf891a939c804509f59593fd47b1a97189127ba65a5649cfb0\
	346637f9836e155eaf891a939c804509f59593fd47b1a97189127ba65a5649cfb0346637f983\
	6e155eaf891a939c804509f59593fd47b1a97189127ba65a5649cfb0346637f9836e155eaf89\
	1a939c804509f59593fd47b1a97189127ba65a5649cfb0346637f9836e155eaf891a939c8045\
	09f59593fd47b1a97189127ba65a5649cfb0346637f9836e155eaf891a939c804509f59593fd\
	47b1a97189127ba65a5649cfb0346637f9836e155eaf891a939c804509f59593fd47b1a97189\
	127ba65a5649cfb0346637f9836e155eaf891a939c804509f59593fd47b1a97189127ba65a56\
	49cfb0346637f9836e155eaf891a939c804509f59593fd47b1a97189127ba65a5649cfb03466\
	37f9836e155eaf891a939c804509f59593fd47b1a97189127ba65a5649cfb0346637f9836e15\
	5eaf891a939c804509f59593fd47b1a97189127ba65a5649cfb0346637f9836e155eaf891a93\
	9c804509f59593fd47b1a97189127ba65a5649cfb0346637f9836e155eaf891a939ccd0780ff\
	ff8078916e776c64ccea05e958559f015c082d9d06feafa3610fc44a5b2ef543cb818078916e\
	776c64ccea05e958559f015c082d9d06feafa3610fc44a5b2ef543cb818078916e776c64ccea\
	05e958559f015c082d9d06feafa3610fc44a5b2ef543cb818078916e776c64ccea05e958559f\
	015c082d9d06feafa3610fc44a5b2ef543cb818078916e776c64ccea05e958559f015c082d9d\
	06feafa3610fc44a5b2ef543cb81008078916e776c64ccea05e958559f015c082d9d06feafa3\
	610fc44a5b2ef543cb818078916e776c64ccea05e958559f015c082d9d06feafa3610fc44a5b\
	2ef543cb818078916e776c64ccea05e958559f015c082d9d06feafa3610fc44a5b2ef543cb81\
	8078916e776c64ccea05e958559f015c082d9d06feafa3610fc44a5b2ef543cb818078916e77\
	6c64ccea05e958559f015c082d9d06feafa3610fc44a5b2ef543cb818078916e776c64ccea05\
	e958559f015c082d9d06feafa3610fc44a5b2ef543cb818078916e776c64ccea05e958559f01\
	5c082d9d06feafa3610fc44a5b2ef543cb818078916e776c64ccea05e958559f015c082d9d06\
	feafa3610fc44a5b2ef543cb818078916e776c64ccea05e958559f015c082d9d06feafa3610f\
	c44a5b2ef543cb818078916e776c64ccea05e958559f015c082d9d06feafa3610fc44a5b2ef5\
	43cb811044010000\
";
fn proof() -> Vec<u8> {
	array_bytes::hex2bytes_unchecked(PROOF)
}

fn assert_last_event<T: Config>(generic_event: <T as Config>::RuntimeEvent) {
	let events = System::<T>::events();
	let system_event: <T as frame_system::Config>::RuntimeEvent = generic_event.into();
	let EventRecord { event, .. } = &events[events.len() - 1];
	assert_eq!(event, &system_event);
}

pub fn run_to_block<T: Config>(n: frame_system::pallet_prelude::BlockNumberFor<T>) {
	while System::<T>::block_number() < n {
		TransactionStorage::<T>::on_finalize(System::<T>::block_number());
		System::<T>::on_finalize(System::<T>::block_number());
		System::<T>::set_block_number(System::<T>::block_number() + One::one());
		System::<T>::on_initialize(System::<T>::block_number());
		TransactionStorage::<T>::on_initialize(System::<T>::block_number());
	}
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn store(l: Linear<{ 1 }, { T::MaxTransactionSize::get() }>) -> Result<(), BenchmarkError> {
		let data = vec![0u8; l as usize];
		let content_hash = sp_io::hashing::blake2_256(&data);

		#[extrinsic_call]
		_(RawOrigin::None, data);

		assert!(!BlockTransactions::<T>::get().is_empty());
		assert_last_event::<T>(Event::Stored { index: 0, content_hash }.into());
		Ok(())
	}

	#[benchmark]
	fn renew() -> Result<(), BenchmarkError> {
		let data = vec![0u8; T::MaxTransactionSize::get() as usize];
		let content_hash = sp_io::hashing::blake2_256(&data);
		TransactionStorage::<T>::store(RawOrigin::None.into(), data)?;
		run_to_block::<T>(1u32.into());

		#[extrinsic_call]
		_(RawOrigin::None, BlockNumberFor::<T>::zero(), 0);

		assert_last_event::<T>(Event::Renewed { index: 0, content_hash }.into());
		Ok(())
	}

	#[benchmark]
	fn check_proof() -> Result<(), BenchmarkError> {
		run_to_block::<T>(1u32.into());
		for _ in 0..T::MaxBlockTransactions::get() {
			TransactionStorage::<T>::store(
				RawOrigin::None.into(),
				vec![0u8; T::MaxTransactionSize::get() as usize],
			)?;
		}
		run_to_block::<T>(crate::Pallet::<T>::retention_period() + BlockNumberFor::<T>::one());
		let encoded_proof = proof();
		let proof = TransactionStorageProof::decode(&mut &*encoded_proof).unwrap();

		#[extrinsic_call]
		_(RawOrigin::None, proof);

		assert_last_event::<T>(Event::ProofChecked.into());
		Ok(())
	}

	#[benchmark]
	fn authorize_account() -> Result<(), BenchmarkError> {
		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let who: T::AccountId = whitelisted_caller();
		let transactions = 10;
		let bytes: u64 = 1024 * 1024;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, who.clone(), transactions, bytes);

		assert_last_event::<T>(Event::AccountAuthorized { who, transactions, bytes }.into());
		Ok(())
	}

	#[benchmark]
	fn refresh_account_authorization() -> Result<(), BenchmarkError> {
		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let who: T::AccountId = whitelisted_caller();
		let transactions = 10;
		let bytes: u64 = 1024 * 1024;
		let origin2 = origin.clone();
		TransactionStorage::<T>::authorize_account(
			origin2 as T::RuntimeOrigin,
			who.clone(),
			transactions,
			bytes,
		)
		.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, who.clone());

		assert_last_event::<T>(Event::AccountAuthorizationRefreshed { who }.into());
		Ok(())
	}

	#[benchmark]
	fn authorize_preimage() -> Result<(), BenchmarkError> {
		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let content_hash = [0u8; 32];
		let max_size: u64 = 1024 * 1024;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, content_hash, max_size);

		assert_last_event::<T>(Event::PreimageAuthorized { content_hash, max_size }.into());
		Ok(())
	}

	#[benchmark]
	fn refresh_preimage_authorization() -> Result<(), BenchmarkError> {
		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let content_hash = [0u8; 32];
		let max_size: u64 = 1024 * 1024;
		let origin2 = origin.clone();
		TransactionStorage::<T>::authorize_preimage(
			origin2 as T::RuntimeOrigin,
			content_hash,
			max_size,
		)
		.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, content_hash);

		assert_last_event::<T>(Event::PreimageAuthorizationRefreshed { content_hash }.into());
		Ok(())
	}

	#[benchmark]
	fn remove_expired_account_authorization() -> Result<(), BenchmarkError> {
		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let who: T::AccountId = whitelisted_caller();
		TransactionStorage::<T>::authorize_account(origin, who.clone(), 1, 1)
			.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;

		let period = T::AuthorizationPeriod::get();
		let now = System::<T>::block_number();
		run_to_block::<T>(now + period);

		#[extrinsic_call]
		_(RawOrigin::None, who.clone());

		assert_last_event::<T>(Event::ExpiredAccountAuthorizationRemoved { who }.into());
		Ok(())
	}

	#[benchmark]
	fn remove_expired_preimage_authorization() -> Result<(), BenchmarkError> {
		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let content_hash = [0; 32];
		TransactionStorage::<T>::authorize_preimage(origin, content_hash, 1)
			.map_err(|_| BenchmarkError::Stop("unable to authorize preimage"))?;

		let period = T::AuthorizationPeriod::get();
		let now = System::<T>::block_number();
		run_to_block::<T>(now + period);

		#[extrinsic_call]
		_(RawOrigin::None, content_hash);

		assert_last_event::<T>(Event::ExpiredPreimageAuthorizationRemoved { content_hash }.into());
		Ok(())
	}

	impl_benchmark_test_suite!(TransactionStorage, crate::mock::new_test_ext(), crate::mock::Test);
}
