// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Utilities for zombienet-sdk tests

use tracing_subscriber::EnvFilter;

/// Idempotent `tracing-subscriber` init. Honors `RUST_LOG`; defaults to `info`.
pub fn init_logging() {
	let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
	let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// Log macros that prefix messages with test name for parallel test runs.
/// Usage: `test_log!(TEST, "message {}", arg);`
#[macro_export]
macro_rules! test_log {
	($test_name:expr, $($arg:tt)*) => {
		tracing::info!("[{}] {}", $test_name, format!($($arg)*))
	};
}

#[macro_export]
macro_rules! test_warn {
	($test_name:expr, $($arg:tt)*) => {
		tracing::warn!("[{}] {}", $test_name, format!($($arg)*))
	};
}

#[macro_export]
macro_rules! test_error {
	($test_name:expr, $($arg:tt)*) => {
		tracing::error!("[{}] {}", $test_name, format!($($arg)*))
	};
}

pub mod bitswap;
pub mod config;
pub mod crypto;
pub mod events;
pub mod hop_rpc;
pub mod ldb;
pub mod network;
pub mod sync;
pub mod tx;

pub use bitswap::*;
pub use config::*;
pub use crypto::*;
pub use events::*;
pub use hop_rpc::*;
pub use ldb::*;
pub use network::*;
pub use sync::*;
pub use tx::*;
