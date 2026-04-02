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
use crate::extension::ValidateStorageCalls;
use alloc::vec;
use polkadot_sdk_frame::{
	benchmarking::prelude::*,
	deps::{
		frame_support::dispatch::{DispatchInfo, PostDispatchInfo},
		frame_system::{EventRecord, Pallet as System, RawOrigin},
		sp_runtime::traits::{AsTransactionAuthorizedOrigin, DispatchTransaction, Dispatchable},
	},
	traits::{AsSystemOriginSigner, IsSubType, OriginTrait},
};
use sp_transaction_storage_proof::TransactionStorageProof;

/// Helper trait for benchmarking. The runtime must provide a pre-computed storage proof
/// that matches its `MaxTransactionSize` and `MaxBlockTransactions` configuration.
pub trait BenchmarkHelper<T: Config> {
	/// Returns an encoded `TransactionStorageProof` for a block full of
	/// `MaxBlockTransactions` zero-filled transactions of `MaxTransactionSize` bytes,
	/// built with `random_hash` as randomness.
	fn check_proof_encoded(random_hash: &[u8]) -> Vec<u8>;
}

/// Default [`BenchmarkHelper`] for runtimes using [`DEFAULT_MAX_TRANSACTION_SIZE`] and
/// [`DEFAULT_MAX_BLOCK_TRANSACTIONS`]. Regenerate with `gen_check_proof` runtime test if these
/// change.
pub struct DefaultCheckProofHelper;

const DEFAULT_CHECK_PROOF: &str = "\
	0104000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000000000000000000\
	0000000000000000000000000000000000000000000000000000000000000ccd0780ffff0080\
	f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825\
	c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83\
	a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a373\
	3464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b\
	5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d305\
	5c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e31\
	3ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771\
	032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc\
	9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0\
	f8a3733464780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464\
	780a2b5bb2e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2\
	e5d3055c04a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04\
	a28e313ad980f771032825c1fc9bea83a6e0f8a3733464780a2b5bb2e5d3055c04a28e313ad9\
	ad03803333008041038b346937eae08686bc2166a94e8ebcad3aac044655f5e016556efab645\
	178010fd81bc1359802f0b871aeb95e4410a8ec92b93af10ea767a2027cf4734e8de8041038b\
	346937eae08686bc2166a94e8ebcad3aac044655f5e016556efab645178010fd81bc1359802f\
	0b871aeb95e4410a8ec92b93af10ea767a2027cf4734e8de8041038b346937eae08686bc2166\
	a94e8ebcad3aac044655f5e016556efab645178010fd81bc1359802f0b871aeb95e4410a8ec9\
	2b93af10ea767a2027cf4734e8de8041038b346937eae08686bc2166a94e8ebcad3aac044655\
	f5e016556efab64517084000\
";

impl<T: Config> BenchmarkHelper<T> for DefaultCheckProofHelper {
	fn check_proof_encoded(random_hash: &[u8]) -> Vec<u8> {
		assert_eq!(
			T::MaxTransactionSize::get(),
			DEFAULT_MAX_TRANSACTION_SIZE,
			"DefaultCheckProofHelper requires MaxTransactionSize == DEFAULT_MAX_TRANSACTION_SIZE ({})",
			DEFAULT_MAX_TRANSACTION_SIZE,
		);
		assert_eq!(
			T::MaxBlockTransactions::get(),
			DEFAULT_MAX_BLOCK_TRANSACTIONS,
			"DefaultCheckProofHelper requires MaxBlockTransactions == DEFAULT_MAX_BLOCK_TRANSACTIONS ({})",
			DEFAULT_MAX_BLOCK_TRANSACTIONS,
		);
		assert_eq!(
			random_hash, &[0u8; 32],
			"DefaultCheckProofHelper proof was built with [0u8; 32]"
		);
		array_bytes::hex2bytes_unchecked(DEFAULT_CHECK_PROOF)
	}
}

type RuntimeCallOf<T> = <T as frame_system::Config>::RuntimeCall;

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

#[benchmarks(where
	T: Send + Sync,
	RuntimeCallOf<T>: IsSubType<Call<T>> + From<Call<T>> + Dispatchable<Info = DispatchInfo, PostInfo = PostDispatchInfo>,
	T::RuntimeOrigin: OriginTrait + AsSystemOriginSigner<T::AccountId> + AsTransactionAuthorizedOrigin + From<Origin<T>> + Clone,
	<T::RuntimeOrigin as OriginTrait>::PalletsOrigin: From<Origin<T>> + TryInto<Origin<T>>,
)]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn store(l: Linear<{ 1 }, { T::MaxTransactionSize::get() }>) -> Result<(), BenchmarkError> {
		let data = vec![0u8; l as usize];
		let content_hash = sp_io::hashing::blake2_256(&data);
		let cid = calculate_cid(
			&data,
			CidConfig { codec: RAW_CODEC, hashing: HashingAlgorithm::Blake2b256 },
		)
		.unwrap()
		.to_bytes();

		#[extrinsic_call]
		_(RawOrigin::None, data);

		assert!(!BlockTransactions::<T>::get().is_empty());
		assert_last_event::<T>(Event::Stored { index: 0, content_hash, cid }.into());
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
		// Advance to block 2 so on_finalize(1) commits BlockTransactions into Transactions storage,
		// then jump directly to the target block — no need to iterate the remaining blocks since
		// on_initialize cleanup targets block n - period - 1 (i.e. block 0), preserving block 1.
		run_to_block::<T>(2u32.into());
		System::<T>::set_block_number(
			crate::Pallet::<T>::retention_period() + BlockNumberFor::<T>::one(),
		);
		// The pre-computed proof was built with T::Hash::default() as randomness.
		// Pin ParentHash to the same value so chunk selection matches.
		let random_hash = T::Hash::default();
		frame_support::storage::unhashed::put(
			&sp_io::hashing::twox_128(b"System")
				.iter()
				.chain(sp_io::hashing::twox_128(b"ParentHash").iter())
				.copied()
				.collect::<alloc::vec::Vec<u8>>(),
			&random_hash,
		);
		let encoded = T::BenchmarkHelper::check_proof_encoded(random_hash.as_ref());
		let proof = TransactionStorageProof::decode(&mut encoded.as_slice()).unwrap();

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
		System::<T>::set_block_number(now + period);

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
		System::<T>::set_block_number(now + period);

		#[extrinsic_call]
		_(RawOrigin::None, content_hash);

		assert_last_event::<T>(Event::ExpiredPreimageAuthorizationRemoved { content_hash }.into());
		Ok(())
	}

	#[benchmark]
	fn validate_store(
		l: Linear<{ 1 }, { T::MaxTransactionSize::get() }>,
	) -> Result<(), BenchmarkError> {
		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let caller: T::AccountId = whitelisted_caller();
		let data = vec![0u8; l as usize];
		let transactions = 10;
		let bytes = l as u64 * 10;
		TransactionStorage::<T>::authorize_account(
			origin as T::RuntimeOrigin,
			caller.clone(),
			transactions,
			bytes,
		)
		.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;

		let ext = ValidateStorageCalls::<T>::default();
		let call: RuntimeCallOf<T> = Call::<T>::store { data }.into();
		let info = DispatchInfo::default();
		let len = 0_usize;

		// test_run exercises validate + prepare + post_dispatch without executing the
		// extrinsic itself (the closure substitutes for the actual dispatch).
		#[block]
		{
			ext.test_run(RawOrigin::Signed(caller.clone()).into(), &call, &info, len, 0, |_| {
				Ok(().into())
			})
			.unwrap()
			.unwrap();
		}

		// prepare consumed one transaction worth of authorization
		let extent = TransactionStorage::<T>::account_authorization_extent(caller);
		assert_eq!(extent.transactions, transactions - 1);
		Ok(())
	}

	#[benchmark]
	fn validate_renew() -> Result<(), BenchmarkError> {
		let data = vec![0u8; T::MaxTransactionSize::get() as usize];
		TransactionStorage::<T>::store(RawOrigin::None.into(), data.clone())?;
		run_to_block::<T>(1u32.into());

		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let caller: T::AccountId = whitelisted_caller();
		let transactions = 10;
		let bytes = T::MaxTransactionSize::get() as u64 * 10;
		TransactionStorage::<T>::authorize_account(
			origin as T::RuntimeOrigin,
			caller.clone(),
			transactions,
			bytes,
		)
		.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;

		let ext = ValidateStorageCalls::<T>::default();
		let call: RuntimeCallOf<T> =
			Call::<T>::renew { block: BlockNumberFor::<T>::zero(), index: 0 }.into();
		let info = DispatchInfo::default();
		let len = 0_usize;

		// test_run exercises validate + prepare + post_dispatch without executing the
		// extrinsic itself (the closure substitutes for the actual dispatch).
		#[block]
		{
			ext.test_run(RawOrigin::Signed(caller.clone()).into(), &call, &info, len, 0, |_| {
				Ok(().into())
			})
			.unwrap()
			.unwrap();
		}

		// prepare consumed one transaction worth of authorization
		let extent = TransactionStorage::<T>::account_authorization_extent(caller);
		assert_eq!(extent.transactions, transactions - 1);
		Ok(())
	}

	impl_benchmark_test_suite!(TransactionStorage, crate::mock::new_test_ext(), crate::mock::Test);
}

#[cfg(test)]
mod tests {
	use super::*;
	use codec::Encode;
	use sp_transaction_storage_proof::registration::build_proof;

	/// Builds the proof that `DefaultCheckProofHelper` should return for the default config.
	fn generate_default_check_proof() -> Vec<u8> {
		let tx_size = DEFAULT_MAX_TRANSACTION_SIZE as usize;
		let transactions: Vec<Vec<u8>> =
			(0..DEFAULT_MAX_BLOCK_TRANSACTIONS).map(|_| vec![0u8; tx_size]).collect();
		let proof = build_proof(&[0u8; 32], transactions).unwrap().unwrap();
		proof.encode()
	}

	/// Generates the DEFAULT_CHECK_PROOF hex for `DefaultCheckProofHelper`. Run with:
	/// `cargo test -p pallet-transaction-storage -- --nocapture --ignored gen_default_check_proof`
	#[test]
	#[ignore]
	fn gen_default_check_proof() {
		let encoded = generate_default_check_proof();
		let hex: String = encoded.iter().map(|b| format!("{b:02x}")).collect();
		println!(
			"DEFAULT_CHECK_PROOF hex for tx_size={}, max_block_transactions={}:",
			DEFAULT_MAX_TRANSACTION_SIZE, DEFAULT_MAX_BLOCK_TRANSACTIONS,
		);
		println!("{hex}");
	}

	#[test]
	fn default_check_proof_integrity() {
		let expected = generate_default_check_proof();
		let stored = array_bytes::hex2bytes_unchecked(DEFAULT_CHECK_PROOF);
		assert_eq!(
			stored, expected,
			"DEFAULT_CHECK_PROOF is stale — regenerate with: \
			 cargo test -p pallet-transaction-storage -- --nocapture --ignored gen_default_check_proof"
		);
	}
}
