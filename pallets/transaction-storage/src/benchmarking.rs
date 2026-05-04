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
	fn encoded_check_proof(random_hash: &[u8]) -> Vec<u8>;
}

/// Default [`BenchmarkHelper`] for runtimes using [`DEFAULT_MAX_TRANSACTION_SIZE`] and
/// [`DEFAULT_MAX_BLOCK_TRANSACTIONS`]. Regenerate with `gen_default_check_proof` test if these
/// change.
pub struct DefaultCheckProofHelper;

/// Hex-encoded [`TransactionStorageProof`] for the default configuration
/// ([`DEFAULT_MAX_TRANSACTION_SIZE`] / [`DEFAULT_MAX_BLOCK_TRANSACTIONS`], randomness `[0u8; 32]`).
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
	fn encoded_check_proof(random_hash: &[u8]) -> Vec<u8> {
		assert_eq!(
			T::MaxTransactionSize::get(),
			DEFAULT_MAX_TRANSACTION_SIZE,
			"DefaultCheckProofHelper requires MaxTransactionSize == DEFAULT_MAX_TRANSACTION_SIZE ({DEFAULT_MAX_TRANSACTION_SIZE})",
		);
		assert_eq!(
			T::MaxBlockTransactions::get(),
			DEFAULT_MAX_BLOCK_TRANSACTIONS,
			"DefaultCheckProofHelper requires MaxBlockTransactions == DEFAULT_MAX_BLOCK_TRANSACTIONS ({DEFAULT_MAX_BLOCK_TRANSACTIONS})",
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
	fn renew_content_hash() -> Result<(), BenchmarkError> {
		let data = vec![0u8; T::MaxTransactionSize::get() as usize];
		let content_hash = sp_io::hashing::blake2_256(&data);
		TransactionStorage::<T>::store(RawOrigin::None.into(), data)?;
		run_to_block::<T>(1u32.into());

		#[extrinsic_call]
		_(RawOrigin::None, content_hash);

		assert_last_event::<T>(Event::Renewed { index: 0, content_hash }.into());
		Ok(())
	}

	#[benchmark]
	fn authorize_account() -> Result<(), BenchmarkError> {
		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let who: T::AccountId = whitelisted_caller();
		let transactions: u32 = 10;
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
		let bytes: u64 = 1024 * 1024;
		let origin2 = origin.clone();
		TransactionStorage::<T>::authorize_account(
			origin2 as T::RuntimeOrigin,
			who.clone(),
			0,
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
		TransactionStorage::<T>::authorize_account(origin, who.clone(), 0, 1)
			.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;

		// `AuthorizationPeriod` is ~14 days of blocks on real runtimes; iterating
		// `on_initialize`/`on_finalize` for each is ~1.3M no-op iterations per step.
		// The dispatchable only compares `block_number >= expiration`, so we can jump
		// the system block number directly without running intermediate block hooks.
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
		let bytes_allowance = l as u64 * 10;
		TransactionStorage::<T>::authorize_account(
			origin as T::RuntimeOrigin,
			caller.clone(),
			0,
			bytes_allowance,
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

		// prepare added `l` bytes to the used counter
		let extent = TransactionStorage::<T>::account_authorization_extent(caller);
		assert_eq!(extent.bytes, l as u64);
		assert_eq!(extent.bytes_allowance, bytes_allowance);
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
		let bytes_allowance = T::MaxTransactionSize::get() as u64 * 10;
		TransactionStorage::<T>::authorize_account(
			origin as T::RuntimeOrigin,
			caller.clone(),
			0,
			bytes_allowance,
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

		// prepare added `MaxTransactionSize` bytes to the used counter
		let extent = TransactionStorage::<T>::account_authorization_extent(caller);
		assert_eq!(extent.bytes, T::MaxTransactionSize::get() as u64);
		assert_eq!(extent.bytes_allowance, bytes_allowance);
		Ok(())
	}

	#[benchmark]
	fn enable_auto_renew() -> Result<(), BenchmarkError> {
		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let caller: T::AccountId = whitelisted_caller();
		let data = vec![0u8; T::MaxTransactionSize::get() as usize];
		let content_hash = sp_io::hashing::blake2_256(&data);

		// Authorize account and store data
		TransactionStorage::<T>::authorize_account(
			origin as T::RuntimeOrigin,
			caller.clone(),
			10,
			T::MaxTransactionSize::get() as u64 * 10,
		)
		.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;
		TransactionStorage::<T>::store(RawOrigin::None.into(), data)?;
		run_to_block::<T>(1u32.into());

		#[extrinsic_call]
		_(RawOrigin::Signed(caller.clone()), content_hash);

		assert_last_event::<T>(Event::AutoRenewalEnabled { content_hash, who: caller }.into());
		Ok(())
	}

	#[benchmark]
	fn disable_auto_renew() -> Result<(), BenchmarkError> {
		let origin = T::Authorizer::try_successful_origin()
			.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
		let caller: T::AccountId = whitelisted_caller();
		let data = vec![0u8; T::MaxTransactionSize::get() as usize];
		let content_hash = sp_io::hashing::blake2_256(&data);

		// Authorize, store, advance, then enable auto-renew
		TransactionStorage::<T>::authorize_account(
			origin as T::RuntimeOrigin,
			caller.clone(),
			10,
			T::MaxTransactionSize::get() as u64 * 10,
		)
		.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;
		TransactionStorage::<T>::store(RawOrigin::None.into(), data)?;
		run_to_block::<T>(1u32.into());
		TransactionStorage::<T>::enable_auto_renew(
			RawOrigin::Signed(caller.clone()).into(),
			content_hash,
		)
		.map_err(|_| BenchmarkError::Stop("unable to enable auto-renew"))?;

		#[extrinsic_call]
		_(RawOrigin::Signed(caller.clone()), content_hash);

		assert_last_event::<T>(Event::AutoRenewalDisabled { content_hash, who: caller }.into());
		Ok(())
	}

	/// Worst-case benchmark for the composite mandatory inherent: a storage proof to
	/// verify AND `n` pending auto-renewals to drain in the same block. The intercept
	/// captures the proof-check + drain-dispatch overhead; the slope captures per-item
	/// renewal cost. The dispatchable always declares `apply_block_inherents(MAX)`,
	/// so blocks where only one branch has work are conservatively over-charged.
	#[benchmark]
	fn apply_block_inherents(
		n: Linear<0, { T::MaxBlockTransactions::get() }>,
	) -> Result<(), BenchmarkError> {
		// Override the default retention period (DEFAULT_RETENTION_PERIOD = ~14 days
		// of blocks) with a tiny value so `run_to_block` only iterates ~10 blocks of
		// `on_initialize`/`on_finalize` per benchmark step. The cost of the inherent
		// itself does not depend on the retention period — it only governs which
		// block's payload the proof verifies.
		const BENCH_RETENTION: u32 = 10;
		RetentionPeriod::<T>::put(BlockNumberFor::<T>::from(BENCH_RETENTION));

		// Step 1: prime block 1 with `MaxBlockTransactions` entries. Going through
		// `store()` 512 times costs ~12 minutes per benchmark step because each call
		// does a `blake2_256_ordered_root` over ~8K chunks of zero-data. Optimization:
		// call `store()` once to populate column TRANSACTION + capture the canonical
		// `TransactionInfo`, then clone that entry into `BlockTransactions` 511 more
		// times with updated cumulative `block_chunks`. The proof verification only
		// reads `Transactions[target]` (and the chunk_root field of each entry), so
		// every entry must carry the correct chunk_root — but the heavy Merkle root
		// computation only needs to happen once.
		run_to_block::<T>(1u32.into());
		TransactionStorage::<T>::store(
			RawOrigin::None.into(),
			vec![0u8; T::MaxTransactionSize::get() as usize],
		)?;
		let template = BlockTransactions::<T>::get()
			.first()
			.cloned()
			.ok_or(BenchmarkError::Stop("first store did not populate BlockTransactions"))?;
		let chunks_per_tx = template.block_chunks;
		BlockTransactions::<T>::mutate(|txns| -> Result<(), BenchmarkError> {
			for i in 1..T::MaxBlockTransactions::get() {
				let mut next = template.clone();
				next.block_chunks = chunks_per_tx.saturating_mul(i + 1);
				txns.try_push(next)
					.map_err(|_| BenchmarkError::Stop("BlockTransactions overflow"))?;
			}
			Ok(())
		})?;

		// Step 2: advance to the proof-check block (1 + RetentionPeriod). `run_to_block`
		// stops after on_initialize of the target block, so on_finalize of the target
		// block has NOT run yet — the dispatchable will satisfy its proof + pending
		// invariants before that ever happens. The first `run_to_block` step here also
		// finalizes block 1, moving `BlockTransactions` → `Transactions[1]`.
		run_to_block::<T>(crate::Pallet::<T>::retention_period() + BlockNumberFor::<T>::one());

		// Step 3: pre-populate `n` PendingAutoRenewals entries. The drain loop calls
		// `do_renew` for each, which pushes a `TransactionInfo` into `BlockTransactions`,
		// updates `TransactionByContentHash`, and bumps the column-TRANSACTION refcount
		// via `transaction_index::renew`. Synthetic content hashes are sufficient — none
		// of those operations validate against existing storage.
		if n > 0 {
			let origin = T::Authorizer::try_successful_origin()
				.map_err(|_| BenchmarkError::Stop("unable to compute origin"))?;
			let caller: T::AccountId = whitelisted_caller();

			TransactionStorage::<T>::authorize_account(
				origin as T::RuntimeOrigin,
				caller.clone(),
				n * 10,
				T::MaxTransactionSize::get() as u64 * n as u64 * 10,
			)
			.map_err(|_| BenchmarkError::Stop("unable to authorize account"))?;

			let mut pending = PendingAutoRenewals::<T>::get();
			for i in 0..n {
				let content_hash = sp_io::hashing::blake2_256(&i.to_le_bytes());
				let tx_info = TransactionInfo {
					chunk_root: Default::default(),
					size: 1,
					content_hash,
					hashing: HashingAlgorithm::Blake2b256,
					cid_codec: RAW_CODEC,
					block_chunks: 0,
				};
				let renewal_data = AutoRenewalData { account: caller.clone() };
				pending
					.try_push((content_hash, tx_info, renewal_data))
					.map_err(|_| BenchmarkError::Stop("unable to push pending renewal"))?;
			}
			PendingAutoRenewals::<T>::put(&pending);
		}

		// Step 4: pin ParentHash to T::Hash::default() — the proof returned by
		// `T::BenchmarkHelper::encoded_check_proof` was built against random_hash =
		// `T::Hash::default()`. The runtime's `random_chunk(parent_hash, total_chunks)`
		// must use the same value to pick the chunk the proof was built for.
		let random_hash = T::Hash::default();
		frame_support::storage::unhashed::put(
			&sp_io::hashing::twox_128(b"System")
				.iter()
				.chain(sp_io::hashing::twox_128(b"ParentHash").iter())
				.copied()
				.collect::<alloc::vec::Vec<u8>>(),
			&random_hash,
		);
		let encoded = T::BenchmarkHelper::encoded_check_proof(random_hash.as_ref());
		let proof = TransactionStorageProof::decode(&mut encoded.as_slice()).unwrap();

		#[extrinsic_call]
		_(RawOrigin::None, Some(proof));

		assert!(PendingAutoRenewals::<T>::get().is_empty());
		// Proof check ran (event order varies depending on `n` — drains emit later events).
		let proof_checked: <T as frame_system::Config>::RuntimeEvent =
			<T as Config>::RuntimeEvent::from(Event::<T>::ProofChecked).into();
		assert!(
			System::<T>::events().iter().any(|r| r.event == proof_checked),
			"ProofChecked event must be emitted",
		);
		Ok(())
	}

	/// Worst-case benchmark for the `Hooks::on_initialize` expiry sweep.
	///
	/// Each iteration of the per-tx loop reads `TransactionByContentHash` and
	/// `AutoRenewals` once; on the cleanup path it also writes
	/// `TransactionByContentHash`. Half of the prepared items have auto-renewal
	/// registered so both branches of the discriminator are exercised across `n`.
	///
	/// Setup uses the same store-once-clone-rest trick as `apply_block_inherents`:
	/// one real `store()` to populate column TRANSACTION + capture the canonical
	/// `TransactionInfo`, then `n - 1` direct clones into `BlockTransactions`. The
	/// hot path being measured is on_initialize, not the setup, so this is sound.
	#[benchmark]
	fn on_initialize_with_expiry(
		n: Linear<0, { T::MaxBlockTransactions::get() }>,
	) -> Result<(), BenchmarkError> {
		// Override retention period so the obsolete-target arithmetic is small and
		// `run_to_block` doesn't iterate ~200K block hooks per benchmark step.
		const BENCH_RETENTION: u32 = 10;
		RetentionPeriod::<T>::put(BlockNumberFor::<T>::from(BENCH_RETENTION));

		// Block 1: prime BlockTransactions with `n` entries via the
		// store-once-clone-rest pattern. (No-op when `n == 0`.)
		run_to_block::<T>(1u32.into());
		if n > 0 {
			TransactionStorage::<T>::store(
				RawOrigin::None.into(),
				vec![0u8; T::MaxTransactionSize::get() as usize],
			)?;
			let template = BlockTransactions::<T>::get()
				.first()
				.cloned()
				.ok_or(BenchmarkError::Stop("first store did not populate BlockTransactions"))?;
			let chunks_per_tx = template.block_chunks;
			BlockTransactions::<T>::mutate(|txns| -> Result<(), BenchmarkError> {
				for i in 1..n {
					let mut next = template.clone();
					next.block_chunks = chunks_per_tx.saturating_mul(i + 1);
					txns.try_push(next)
						.map_err(|_| BenchmarkError::Stop("BlockTransactions overflow"))?;
				}
				Ok(())
			})?;

			// One insert is enough: every cloned entry shares `template.content_hash`,
			// so the on_initialize loop takes the push-to-pending branch for all `n`.
			let caller: T::AccountId = whitelisted_caller();
			AutoRenewals::<T>::insert(template.content_hash, AutoRenewalData { account: caller });
		}

		// Finalize block 1 → BlockTransactions becomes Transactions[1].
		run_to_block::<T>(2u32.into());

		// Jump to the block AFTER the obsolete target so on_initialize takes
		// `Transactions[1]` on the next call. The hook reads `obsolete = n - RP - 1`,
		// so we pre-set the block number to `RP + 2` (= 12 with BENCH_RETENTION=10),
		// because the harness's `#[block]` invocation will run on_initialize for
		// `System::block_number()`.
		System::<T>::set_block_number(BlockNumberFor::<T>::from(BENCH_RETENTION + 2u32));

		// The block under measurement.
		#[block]
		{
			TransactionStorage::<T>::on_initialize(System::<T>::block_number());
		}

		// Sanity: Transactions[1] was taken (no longer in storage) iff n > 0.
		if n > 0 {
			assert!(
				Transactions::<T>::get(BlockNumberFor::<T>::from(1u32)).is_none(),
				"on_initialize should have taken Transactions[1]",
			);
		}
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
			"DEFAULT_CHECK_PROOF hex for tx_size={DEFAULT_MAX_TRANSACTION_SIZE}, \
			max_block_transactions={DEFAULT_MAX_BLOCK_TRANSACTIONS}:",
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
