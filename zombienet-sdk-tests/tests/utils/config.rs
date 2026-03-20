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
pub const VALIDATOR_ROLE_VALUE: f64 = 4.0;
pub const IDLE_VALUE: f64 = 0.0;

// Environment variables
pub const BINARY_PATH_ENV: &str = "POLKADOT_BULLETIN_BINARY_PATH";
pub const DEFAULT_BINARY: &str = "./target/release/polkadot-bulletin-chain";
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
pub const CHAIN_ID: &str = "local_testnet";
pub const TRANSACTION_STORAGE_COLUMN: &str = "col11";
pub const NODE_LOG_CONFIG: &str = "-lsync=trace,sub-libp2p=trace,litep2p=trace,request-response=trace,transaction-storage=trace,bitswap=trace";

// Parachain network topology (configurable via env vars)
pub const RELAY_CHAIN_ENV: &str = "RELAY_CHAIN";
pub const DEFAULT_RELAY_CHAIN: &str = "westend-local";

pub const PARA_ID_ENV: &str = "PARACHAIN_ID";
pub const DEFAULT_PARA_ID: u32 = 2487;

pub const PARACHAIN_CHAIN_ID_ENV: &str = "PARACHAIN_CHAIN_ID";
pub const DEFAULT_PARACHAIN_CHAIN_ID: &str = "bulletin-westend";

pub const SOLO_TEST_DATA_PATTERN: &[u8] = b"ZOMBIENET_TEST_DATA_";
pub const PARACHAIN_TEST_DATA_PATTERN: &[u8] = b"ZOMBIENET_PARACHAIN_TEST_DATA_";

// LDB tool
pub const LDB_PATH_ENV: &str = "ROCKSDB_LDB_PATH";
pub const DEFAULT_LDB_PATH: &str = "rocksdb_ldb";
