// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Constants for zombienet-sdk tests: metrics, environment variables, timeouts, and test data.

// Prometheus metrics
pub const BEST_BLOCK_METRIC: &str = "block_height{status=\"best\"}";

// Environment variables — binary paths
pub const RELAY_BINARY_PATH_ENV: &str = "POLKADOT_RELAY_BINARY_PATH";
pub const DEFAULT_RELAY_BINARY: &str = "polkadot";
pub const PARACHAIN_BINARY_PATH_ENV: &str = "POLKADOT_PARACHAIN_BINARY_PATH";
pub const DEFAULT_PARACHAIN_BINARY: &str = "polkadot-omni-node";
pub const PARACHAIN_CHAIN_SPEC_ENV: &str = "PARACHAIN_CHAIN_SPEC_PATH";
pub const DEFAULT_PARACHAIN_CHAIN_SPEC: &str = "./zombienet/bulletin-westend-spec.json";

// Parachain network topology (configurable via env vars)
pub const RELAY_CHAIN_ENV: &str = "RELAY_CHAIN";
pub const DEFAULT_RELAY_CHAIN: &str = "westend-local";
pub const PARA_ID_ENV: &str = "PARACHAIN_ID";
pub const DEFAULT_PARA_ID: u32 = 2487;
pub const PARACHAIN_CHAIN_ID_ENV: &str = "PARACHAIN_CHAIN_ID";
pub const DEFAULT_PARACHAIN_CHAIN_ID: &str = "bulletin-westend";

// Environment variables — runtime WASM paths
pub const OLD_RUNTIME_WASM_ENV: &str = "OLD_RUNTIME_WASM_PATH";
pub const DEFAULT_OLD_RUNTIME_WASM: &str =
	"./zombienet-sdk-tests/runtimes/old_runtime.compact.compressed.wasm";
pub const BROKEN_RUNTIME_WASM_ENV: &str = "BROKEN_RUNTIME_WASM_PATH";
pub const DEFAULT_BROKEN_RUNTIME_WASM: &str =
	"./zombienet-sdk-tests/runtimes/broken_runtime.compact.compressed.wasm";
pub const FIX_RUNTIME_WASM_ENV: &str = "FIX_RUNTIME_WASM_PATH";
pub const DEFAULT_FIX_RUNTIME_WASM: &str =
	"./zombienet-sdk-tests/runtimes/fix_runtime.compact.compressed.wasm";
pub const NEXT_RUNTIME_WASM_ENV: &str = "NEXT_RUNTIME_WASM_PATH";
pub const DEFAULT_NEXT_RUNTIME_WASM: &str =
	"./zombienet-sdk-tests/runtimes/next_runtime.compact.compressed.wasm";

// Timeouts (seconds)
pub const NETWORK_READY_TIMEOUT_SECS: u64 = 180;
pub const BLOCK_PRODUCTION_TIMEOUT_SECS: u64 = 300;
pub const TRANSACTION_TIMEOUT_SECS: u64 = 60;
pub const FINALIZED_TRANSACTION_TIMEOUT_SECS: u64 = 120;

// Migration test constants
pub const STALL_DETECTION_INTERVAL_SECS: u64 = 2;
pub const STALL_THRESHOLD_SECS: u64 = 30;
pub const STALL_DETECTION_TIMEOUT_SECS: u64 = 300;
pub const RECOVERY_TIMEOUT_SECS: u64 = 180;
pub const TEST_RETENTION_PERIOD: u32 = 30;

// Test data
pub const TEST_DATA_SIZE: usize = 2048;
pub const PARACHAIN_TEST_DATA_PATTERN: &[u8] = b"ZOMBIENET_PARACHAIN_TEST_DATA_";
