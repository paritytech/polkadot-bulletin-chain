// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Constants for zombienet-sdk tests: metrics, environment variables, timeouts, and test data.

// Prometheus metrics
pub const BEST_BLOCK_METRIC: &str = "block_height{status=\"best\"}";
pub const FINALIZED_BLOCK_METRIC: &str = "block_height{status=\"finalized\"}";
pub const NODE_ROLE_METRIC: &str = "node_roles";
/// 1.0 = syncing, 0.0 = idle
pub const IS_MAJOR_SYNCING_METRIC: &str = "substrate_sub_libp2p_is_major_syncing";

pub const FULLNODE_ROLE_VALUE: f64 = 1.0;
pub const IDLE_VALUE: f64 = 0.0;

// HOP (`sc-hop`) Prometheus metrics. The zombienet metrics parser indexes every series
// both with and without the `chain` label, so labelled series are addressed by their
// remaining labels alone — in the order `sc-hop` declares them.
pub const HOP_POOL_ENTRIES_METRIC: &str = "substrate_hop_pool_entries";
pub const HOP_POOL_BYTES_METRIC: &str = "substrate_hop_pool_bytes";
pub const HOP_POOL_MAX_BYTES_METRIC: &str = "substrate_hop_pool_max_bytes";
pub const HOP_POOL_INSERTED_BYTES_METRIC: &str = "substrate_hop_pool_inserted_bytes_total";
pub const HOP_PROMOTIONS_CONFIRMED_METRIC: &str = "substrate_hop_promotions_confirmed_total";
pub const HOP_PROMOTION_BACKLOG_METRIC: &str = "substrate_hop_promotion_backlog";
pub const HOP_MAINTENANCE_TICKS_METRIC: &str = "substrate_hop_maintenance_ticks_total";
pub const HOP_REMOVED_ACKED_METRIC: &str = "substrate_hop_pool_removed_total{reason=\"acked\"}";
pub const HOP_REMOVED_EXPIRED_PROMOTED_METRIC: &str =
	"substrate_hop_pool_removed_total{reason=\"expired_promoted\"}";
pub const HOP_REMOVED_EXPIRED_UNPROMOTED_METRIC: &str =
	"substrate_hop_pool_removed_total{reason=\"expired_unpromoted\"}";
pub const HOP_SUBMIT_NOT_AUTHORIZED_METRIC: &str =
	"substrate_hop_rpc_errors_total{method=\"hop_submit\",reason=\"not_authorized\"}";
// `claim`/`ack` map `NotRecipient` to `NotFound` so callers cannot probe whether a hash
// exists, so `not_found` is the reason for both an unknown hash and a wrong signer.
pub const HOP_CLAIM_NOT_FOUND_METRIC: &str =
	"substrate_hop_rpc_errors_total{method=\"hop_claim\",reason=\"not_found\"}";
pub const HOP_ACK_NOT_FOUND_METRIC: &str =
	"substrate_hop_rpc_errors_total{method=\"hop_ack\",reason=\"not_found\"}";

// Environment variables
pub const RELAY_BINARY_PATH_ENV: &str = "POLKADOT_RELAY_BINARY_PATH";
pub const DEFAULT_RELAY_BINARY: &str = "polkadot";
pub const PARACHAIN_BINARY_PATH_ENV: &str = "POLKADOT_PARACHAIN_BINARY_PATH";
pub const DEFAULT_PARACHAIN_BINARY: &str = "polkadot-omni-node";
pub const PARACHAIN_CHAIN_SPEC_ENV: &str = "PARACHAIN_CHAIN_SPEC_PATH";
pub const DEFAULT_PARACHAIN_CHAIN_SPEC: &str = "./zombienet/bulletin-westend-spec.json";

// Timeouts (seconds)
pub const NETWORK_READY_TIMEOUT_SECS: u64 = 180;
pub const METRIC_TIMEOUT_SECS: u64 = 60;
pub const BLOCK_PRODUCTION_TIMEOUT_SECS: u64 = 300;
pub const TRANSACTION_TIMEOUT_SECS: u64 = 60;
pub const FINALIZED_TRANSACTION_TIMEOUT_SECS: u64 = 120;
pub const SYNC_TIMEOUT_SECS: u64 = 180;
pub const LOG_TIMEOUT_SECS: u64 = 60;
pub const LOG_ERROR_TIMEOUT_SECS: u64 = 10;

// Test constants
pub const TEST_DATA_SIZE: usize = 2048;
pub const TRANSACTION_STORAGE_COLUMN: &str = "col11";
pub const NODE_LOG_CONFIG: &str = "-lsync=trace,sub-libp2p=trace,litep2p=trace,request-response=trace,transaction-storage=trace,bitswap=trace";
/// For pruning-eviction tests. Deliberately omits `NODE_LOG_CONFIG`'s libp2p/sync trace
/// targets: they emit ~10 MB per node in minutes, truncating the shared-network log files
/// long before the failing test runs. The `db`/`state-db` targets are what shows whether
/// pruning actually fired. (Node uses RocksDB, so a `parity-db` target would never fire.)
pub const PRUNING_NODE_LOG_CONFIG: &str =
	"-ltransaction-storage=trace,bitswap=trace,db=debug,state-db=debug,state-db::pin=debug";

// Parachain network topology (configurable via env vars)
pub const RELAY_CHAIN_ENV: &str = "RELAY_CHAIN";
pub const DEFAULT_RELAY_CHAIN: &str = "westend-local";

pub const PARA_ID_ENV: &str = "PARACHAIN_ID";
pub const DEFAULT_PARA_ID: u32 = 1010;

pub const PARACHAIN_CHAIN_ID_ENV: &str = "PARACHAIN_CHAIN_ID";
pub const DEFAULT_PARACHAIN_CHAIN_ID: &str = "bulletin-westend";

pub const PARACHAIN_TEST_DATA_PATTERN: &[u8] = b"ZOMBIENET_PARACHAIN_TEST_DATA_";

// LDB tool
pub const LDB_PATH_ENV: &str = "ROCKSDB_LDB_PATH";
pub const DEFAULT_LDB_PATH: &str = "rocksdb_ldb";
