// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the client-side transaction indexing pipeline.
//!
//! These tests exercise the full `IndexOperation` → `apply_index_ops()` →
//! `block_indexed_body()` / `indexed_transaction()` code path used by
//! `pallet-transaction-storage` at the client (database) level.
//!
//! The pattern follows polkadot-sdk's own `sc-client-db` tests
//! (`indexed_data_block_body`, `renew_transaction_storage`).

use codec::Encode;
use pallet_transaction_storage::cids::{CidConfig, HashingAlgorithm};
use sc_client_api::{
	backend::{Backend as BackendApi, BlockImportOperation},
	blockchain::Backend as BlockchainBackend,
};
use sc_client_db::{Backend, BlocksPruning};
use sp_core::H256;
use sp_runtime::{
	testing::{Block as RawBlock, Header, MockCallU64, TestXt},
	StateVersion, Storage,
};
use sp_state_machine::IndexOperation;

// ---------------------------------------------------------------------------
// Custom call enum reflecting the actual pallet call indices (0, 9) and the
// runtime pallet index (40) used in bulletin-polkadot-runtime.
// ---------------------------------------------------------------------------

/// Mirrors the runtime-level call dispatch encoding (pallet index 40).
#[derive(
	Clone,
	Debug,
	PartialEq,
	Eq,
	Encode,
	codec::Decode,
	codec::DecodeWithMemTracking,
	scale_info::TypeInfo,
)]
enum RuntimeCall {
	#[codec(index = 40)]
	TransactionStorage(TransactionStorageCall),
}

/// Mirrors `pallet_transaction_storage::Call` variant encoding.
#[derive(
	Clone,
	Debug,
	PartialEq,
	Eq,
	Encode,
	codec::Decode,
	codec::DecodeWithMemTracking,
	scale_info::TypeInfo,
)]
enum TransactionStorageCall {
	/// `store` — call_index 0, stores data with default CID config (Blake2b-256).
	#[codec(index = 0)]
	Store { data: Vec<u8> },
	/// `store_with_cid_config` — call_index 9, stores data with explicit CID config.
	#[codec(index = 9)]
	StoreWithCidConfig { cid: CidConfig, data: Vec<u8> },
}

// Block types -----------------------------------------------------------------

type SimpleXt = TestXt<MockCallU64, ()>;
type SimpleBlock = RawBlock<SimpleXt>;

type PalletXt = TestXt<RuntimeCall, ()>;
type PalletBlock = RawBlock<PalletXt>;

// ---------------------------------------------------------------------------
// Generic block insertion helper
// ---------------------------------------------------------------------------

/// Insert a block into the test backend with optional index operations.
///
/// Mirrors the helper used in polkadot-sdk's `sc-client-db` tests.
fn insert_block<B>(
	backend: &Backend<B>,
	number: u64,
	parent_hash: H256,
	body: Vec<B::Extrinsic>,
	transaction_index: Option<Vec<IndexOperation>>,
) -> H256
where
	B: sp_runtime::traits::Block<Hash = H256, Header = Header>,
{
	let mut header = Header {
		number,
		parent_hash,
		state_root: Default::default(),
		digest: Default::default(),
		extrinsics_root: Default::default(),
	};

	let block_hash = if number == 0 { Default::default() } else { parent_hash };
	let mut op = backend.begin_operation().unwrap();
	backend.begin_state_operation(&mut op, block_hash).unwrap();

	if let Some(index) = transaction_index {
		op.update_transaction_index(index).unwrap();
	}

	// Provide a minimal state so the block can be committed.
	header.state_root = op.reset_storage(Storage::default(), StateVersion::V1).unwrap().into();

	op.set_block_data(
		header.clone(),
		Some(body),
		None,
		None,
		sc_client_api::backend::NewBlockState::Best,
	)
	.unwrap();

	backend.commit_operation(op).unwrap();

	sp_runtime::traits::Header::hash(&header)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test that indexed transactions are stored and retrievable using
/// `pallet_transaction_storage::Call::store` (Blake2b-256, call_index 0) and
/// `Call::store_with_cid_config` (SHA2-256, call_index 9), exercising the
/// client-side indexing pipeline with realistic extrinsic encodings.
#[test]
fn indexed_transaction_stored_and_retrievable() {
	let backend = Backend::<PalletBlock>::new_test_with_tx_storage(BlocksPruning::KeepAll, 0);

	let data: Vec<u8> = b"hello bulletin".to_vec();

	// Build extrinsics that mirror real runtime calls — same payload, different calls.
	let xt0 = PalletXt::new_transaction(
		RuntimeCall::TransactionStorage(TransactionStorageCall::Store { data: data.clone() }),
		(),
	);
	let xt1 = PalletXt::new_transaction(
		RuntimeCall::TransactionStorage(TransactionStorageCall::StoreWithCidConfig {
			cid: CidConfig { codec: 0x55, hashing: HashingAlgorithm::Sha2_256 },
			data: data.clone(),
		}),
		(),
	);

	// Verify that the raw data sits at the tail of the encoded extrinsic (same
	// invariant the pallet relies on when calling `transaction_index::index`).
	let encoded0 = xt0.encode();
	let encoded1 = xt1.encode();
	assert_eq!(&encoded0[encoded0.len() - data.len()..], data.as_slice());
	assert_eq!(&encoded1[encoded1.len() - data.len()..], data.as_slice());

	// Hash using the same algorithms the pallet would pick for each call.
	let blake2_hash = sp_crypto_hashing::blake2_256(&data);
	let sha2_hash = sp_crypto_hashing::sha2_256(&data);

	let index = vec![
		IndexOperation::Insert {
			extrinsic: 0,
			hash: blake2_hash.to_vec(),
			size: data.len() as u32,
		},
		IndexOperation::Insert { extrinsic: 1, hash: sha2_hash.to_vec(), size: data.len() as u32 },
	];

	insert_block(&backend, 0, Default::default(), vec![xt0, xt1], Some(index));

	let bc = backend.blockchain();

	// Retrieve by Blake2b-256 hash (store).
	let retrieved0 = bc.indexed_transaction(H256::from(blake2_hash)).unwrap().unwrap();
	assert_eq!(retrieved0, data);

	// Retrieve by SHA2-256 hash (store_with_cid_config) — the DB stores data
	// keyed by the raw hash bytes regardless of which algorithm produced them.
	let retrieved1 = bc.indexed_transaction(H256::from(sha2_hash)).unwrap().unwrap();
	assert_eq!(retrieved1, data);

	// Same data stored via different calls/hashes yields identical indexed content.
	assert_eq!(retrieved0, retrieved1);
}

/// Test that indexed transactions are pruned after block finalization when
/// `BlocksPruning::Some(n)` is configured, matching the retention behaviour
/// that `pallet-transaction-storage` relies on.
#[test]
fn indexed_transaction_pruned_after_finalization() {
	// Keep only the last 1 finalized block's data.
	let backend = Backend::<SimpleBlock>::new_test_with_tx_storage(BlocksPruning::Some(1), 10);

	let xt = SimpleXt::new_transaction(0.into(), ());
	let encoded = xt.encode();
	let data = &encoded[1..];
	let content_hash = sp_crypto_hashing::blake2_256(data);

	let index = vec![IndexOperation::Insert {
		extrinsic: 0,
		hash: content_hash.to_vec(),
		size: data.len() as u32,
	}];

	let hash0 = insert_block(&backend, 0, Default::default(), vec![xt.clone()], Some(index));

	let bc = backend.blockchain();
	assert!(bc.indexed_transaction(H256::from(content_hash)).unwrap().is_some());

	// Insert a second block and finalize it — this should prune block 0's data.
	let hash1 = insert_block(&backend, 1, hash0, vec![], None);
	backend.finalize_block(hash1, None).unwrap();

	assert_eq!(bc.body(hash0).unwrap(), None);
	assert!(bc.indexed_transaction(H256::from(content_hash)).unwrap().is_none());
}

/// Test that `IndexOperation::Renew` keeps indexed data alive across
/// finalization cycles, matching the renewal mechanism used by
/// `pallet-transaction-storage::renew`.
#[test]
fn renewed_transaction_survives_pruning() {
	// Keep only the last 2 finalized blocks' data.
	let backend = Backend::<SimpleBlock>::new_test_with_tx_storage(BlocksPruning::Some(2), 10);

	let xt = SimpleXt::new_transaction(0.into(), ());
	let encoded = xt.encode();
	let data = &encoded[1..];
	let content_hash = sp_crypto_hashing::blake2_256(data);

	let mut blocks = Vec::new();
	let mut prev_hash = Default::default();

	for i in 0..10u64 {
		let mut index = Vec::new();
		if i == 0 {
			index.push(IndexOperation::Insert {
				extrinsic: 0,
				hash: content_hash.to_vec(),
				size: data.len() as u32,
			});
		} else if i < 5 {
			// Keep renewing through block 4.
			index.push(IndexOperation::Renew { extrinsic: 0, hash: content_hash.to_vec() });
		}
		// Blocks 5+ stop renewing.

		let hash = insert_block(
			&backend,
			i,
			prev_hash,
			vec![SimpleXt::new_transaction(i.into(), ())],
			Some(index),
		);
		blocks.push(hash);
		prev_hash = hash;
	}

	// Finalize blocks one by one and check indexed data availability.
	for i in 1..10 {
		let mut op = backend.begin_operation().unwrap();
		backend.begin_state_operation(&mut op, blocks[4]).unwrap();
		op.mark_finalized(blocks[i], None).unwrap();
		backend.commit_operation(op).unwrap();

		let bc = backend.blockchain();
		if i < 6 {
			assert!(
				bc.indexed_transaction(H256::from(content_hash)).unwrap().is_some(),
				"data should still be available after finalizing block {i}"
			);
		} else {
			assert!(
				bc.indexed_transaction(H256::from(content_hash)).unwrap().is_none(),
				"data should be pruned after finalizing block {i}"
			);
		}
	}
}
