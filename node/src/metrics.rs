//! Bulletin Chain Prometheus metrics.

use std::sync::Arc;

use codec::{Decode, Encode};
use futures::StreamExt;
use sc_client_api::{BlockchainEvents, StorageProvider};
use sp_core::storage::StorageKey;
use sp_runtime::traits::Header;
use substrate_prometheus_endpoint::{register, Gauge, PrometheusError, Registry, U64};

use crate::{node_primitives::Block, service::FullClient};

/// Bulletin-specific Prometheus metrics.
#[derive(Clone)]
pub struct BulletinMetrics {
	/// Number of store transactions in the latest block.
	pub block_store_transactions: Gauge<U64>,
	/// Bytes stored in the latest block.
	pub block_store_bytes: Gauge<U64>,
	/// Number of renew transactions in the latest block.
	pub block_renew_transactions: Gauge<U64>,
	/// Bytes renewed in the latest block.
	pub block_renew_bytes: Gauge<U64>,
	/// Number of registered validators.
	pub registered_validators: Gauge<U64>,
	/// Whether proof generation failed on the last attempt (0 = ok, 1 = failed).
	pub proof_generation_failed: Gauge<U64>,
	/// Outbound bridge messages pending relay.
	pub bridge_outbound_pending: Gauge<U64>,
	/// Latest generated outbound nonce.
	pub bridge_outbound_generated_nonce: Gauge<U64>,
	/// Latest received (confirmed) outbound nonce.
	pub bridge_outbound_received_nonce: Gauge<U64>,
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
			block_renew_transactions: register(
				Gauge::new(
					"bulletin_block_renew_transactions",
					"Number of data renew transactions in the latest block",
				)?,
				registry,
			)?,
			block_renew_bytes: register(
				Gauge::new(
					"bulletin_block_renew_bytes",
					"Bytes of data renewed in the latest block",
				)?,
				registry,
			)?,
			registered_validators: register(
				Gauge::new(
					"bulletin_registered_validators",
					"Number of registered validators in the validator set",
				)?,
				registry,
			)?,
			proof_generation_failed: register(
				Gauge::new(
					"bulletin_proof_generation_failed",
					"Whether proof generation failed on the last attempt (0 = ok, 1 = failed)",
				)?,
				registry,
			)?,
			bridge_outbound_pending: register(
				Gauge::new(
					"bulletin_bridge_outbound_pending",
					"Number of outbound bridge messages waiting to be relayed",
				)?,
				registry,
			)?,
			bridge_outbound_generated_nonce: register(
				Gauge::new(
					"bulletin_bridge_outbound_latest_generated_nonce",
					"Latest generated outbound bridge message nonce",
				)?,
				registry,
			)?,
			bridge_outbound_received_nonce: register(
				Gauge::new(
					"bulletin_bridge_outbound_latest_received_nonce",
					"Latest received (confirmed delivered) outbound bridge message nonce",
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

/// Compute the raw storage key for a simple StorageValue (pallet_prefix, storage_name).
fn storage_value_key(pallet: &[u8], name: &[u8]) -> StorageKey {
	let mut key = Vec::with_capacity(32);
	key.extend_from_slice(&sp_core::hashing::twox_128(pallet));
	key.extend_from_slice(&sp_core::hashing::twox_128(name));
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

/// Read a SCALE-encoded u32 StorageValue.
fn read_u32_storage(
	client: &FullClient,
	block_hash: <Block as sp_runtime::traits::Block>::Hash,
	key: &StorageKey,
) -> Option<u32> {
	let data = client.storage(block_hash, key).ok()??;
	u32::decode(&mut &data.0[..]).ok()
}

/// Compute the raw storage key for `BridgePolkadotMessages::OutboundLanes` with lane ID
/// `[0,0,0,0]`.
fn outbound_lane_data_key() -> StorageKey {
	let mut key = Vec::with_capacity(32 + 16 + 4);
	key.extend_from_slice(&sp_core::hashing::twox_128(b"BridgePolkadotMessages"));
	key.extend_from_slice(&sp_core::hashing::twox_128(b"OutboundLanes"));
	// Blake2_128Concat hasher: blake2_128(encode(lane_id)) ++ encode(lane_id)
	let lane_id: [u8; 4] = [0, 0, 0, 0];
	let encoded = lane_id.encode();
	key.extend_from_slice(&sp_core::hashing::blake2_128(&encoded));
	key.extend_from_slice(&encoded);
	StorageKey(key)
}

/// Read the OutboundLaneData from raw storage bytes.
/// Returns (oldest_unpruned_nonce, latest_received_nonce, latest_generated_nonce).
fn read_outbound_lane_data(
	client: &FullClient,
	block_hash: <Block as sp_runtime::traits::Block>::Hash,
	key: &StorageKey,
) -> Option<(u64, u64, u64)> {
	let data = client.storage(block_hash, key).ok()??;
	// OutboundLaneData SCALE encoding: 3 × u64 (little-endian) + 1 byte LaneState
	if data.0.len() < 24 {
		return None;
	}
	let oldest_unpruned = u64::from_le_bytes(data.0[0..8].try_into().ok()?);
	let latest_received = u64::from_le_bytes(data.0[8..16].try_into().ok()?);
	let latest_generated = u64::from_le_bytes(data.0[16..24].try_into().ok()?);
	Some((oldest_unpruned, latest_received, latest_generated))
}

/// Read a SCALE-encoded u64 StorageValue.
fn read_u64_storage(
	client: &FullClient,
	block_hash: <Block as sp_runtime::traits::Block>::Hash,
	key: &StorageKey,
) -> Option<u64> {
	let data = client.storage(block_hash, key).ok()??;
	u64::decode(&mut &data.0[..]).ok()
}

/// Spawn a background task that updates Bulletin metrics on each imported block.
pub fn spawn_metrics_task(
	task_manager: &sc_service::TaskManager,
	client: Arc<FullClient>,
	metrics: BulletinMetrics,
) {
	let num_validators_key = storage_value_key(b"ValidatorSet", b"NumValidators");
	let renew_count_key = storage_value_key(b"TransactionStorage", b"BlockRenewCount");
	let renew_bytes_key = storage_value_key(b"TransactionStorage", b"BlockRenewBytes");
	let outbound_lane_key = outbound_lane_data_key();

	let task = async move {
		let mut stream = client.import_notification_stream();
		while let Some(notification) = stream.next().await {
			if !notification.is_new_best {
				continue;
			}
			let block_hash = notification.hash;
			let block_number = *notification.header.number();

			// Store transactions and bytes (combined store + renew from Transactions map).
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

			// Renew transactions and bytes.
			metrics
				.block_renew_transactions
				.set(read_u32_storage(&client, block_hash, &renew_count_key).unwrap_or(0) as u64);
			metrics
				.block_renew_bytes
				.set(read_u64_storage(&client, block_hash, &renew_bytes_key).unwrap_or(0));

			// Registered validators.
			metrics
				.registered_validators
				.set(read_u32_storage(&client, block_hash, &num_validators_key).unwrap_or(0) as u64);

			// Bridge outbound lane metrics (may not exist on Westend parachain runtime).
			if let Some((_, latest_received, latest_generated)) =
				read_outbound_lane_data(&client, block_hash, &outbound_lane_key)
			{
				metrics.bridge_outbound_generated_nonce.set(latest_generated);
				metrics.bridge_outbound_received_nonce.set(latest_received);
				metrics
					.bridge_outbound_pending
					.set(latest_generated.saturating_sub(latest_received));
			}
		}
	};

	task_manager.spawn_handle().spawn("bulletin-metrics", None, task);
}
