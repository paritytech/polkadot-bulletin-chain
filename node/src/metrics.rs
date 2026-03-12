//! Bulletin Chain Prometheus metrics.

use std::sync::Arc;

use codec::{Decode, Encode};
use futures::StreamExt;
use sc_client_api::{BlockchainEvents, StorageProvider};
use sp_core::storage::StorageKey;
use sp_runtime::traits::Header;
use substrate_prometheus_endpoint::{register, Counter, Gauge, PrometheusError, Registry, U64};

use crate::{node_primitives::Block, service::FullClient};

/// Bulletin-specific Prometheus metrics.
#[derive(Clone)]
pub struct BulletinMetrics {
	/// Number of store transactions in the latest block.
	pub block_store_transactions: Gauge<U64>,
	/// Bytes stored in the latest block.
	pub block_store_bytes: Gauge<U64>,
	/// Number of storage proof generation failures on this validator.
	pub proof_generation_failures: Counter<U64>,
}

impl BulletinMetrics {
	pub fn register(registry: &Registry) -> Result<Self, PrometheusError> {
		Ok(Self {
			block_store_transactions: register(
				Gauge::new(
					"bulletin_block_store_transactions",
					"Number of data store transactions in the latest block",
				)?,
				registry,
			)?,
			block_store_bytes: register(
				Gauge::new(
					"bulletin_block_store_bytes",
					"Bytes of data stored in the latest block",
				)?,
				registry,
			)?,
			proof_generation_failures: register(
				Counter::new(
					"bulletin_proof_generation_failures_total",
					"Number of storage proof generation failures on this validator",
				)?,
				registry,
			)?,
		})
	}
}

/// Compute the raw storage key for `TransactionStorage::Transactions` at a given block number.
fn transactions_storage_key(block_number: u32) -> StorageKey {
	let mut key = Vec::with_capacity(32 + 16 + 4);
	key.extend_from_slice(&sp_core::hashing::twox_128(b"TransactionStorage"));
	key.extend_from_slice(&sp_core::hashing::twox_128(b"Transactions"));
	// Blake2_128Concat hasher: blake2_128(encode(key)) ++ encode(key)
	let encoded = block_number.encode();
	key.extend_from_slice(&sp_core::hashing::blake2_128(&encoded));
	key.extend_from_slice(&encoded);
	StorageKey(key)
}

/// Read the stored transaction count and total bytes for a block from on-chain storage.
fn read_block_transactions(
	client: &FullClient,
	block_hash: <Block as sp_runtime::traits::Block>::Hash,
	block_number: u32,
) -> Option<(u64, u64)> {
	let key = transactions_storage_key(block_number);
	let data = client.storage(block_hash, &key).ok()??;
	let transactions =
		Vec::<pallet_transaction_storage::TransactionInfo>::decode(&mut &data.0[..]).ok()?;
	let count = transactions.len() as u64;
	let bytes = transactions.iter().map(|t| t.size as u64).sum();
	Some((count, bytes))
}

/// Spawn a background task that updates Bulletin metrics on each imported block.
pub fn spawn_metrics_task(
	task_manager: &sc_service::TaskManager,
	client: Arc<FullClient>,
	metrics: BulletinMetrics,
) {
	let task = async move {
		let mut stream = client.import_notification_stream();
		while let Some(notification) = stream.next().await {
			if !notification.is_new_best {
				continue;
			}
			let block_hash = notification.hash;
			let block_number = *notification.header.number();

			match read_block_transactions(&client, block_hash, block_number) {
				Some((count, bytes)) => {
					metrics.block_store_transactions.set(count);
					metrics.block_store_bytes.set(bytes);
				},
				None => {
					metrics.block_store_transactions.set(0);
					metrics.block_store_bytes.set(0);
				},
			}
		}
	};

	task_manager.spawn_handle().spawn("bulletin-metrics", None, task);
}
