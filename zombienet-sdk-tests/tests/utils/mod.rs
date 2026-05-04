// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Utilities for zombienet-sdk tests

/// Log macros that prefix messages with test name for parallel test runs.
/// Usage: `test_log!(TEST, "message {}", arg);`
#[macro_export]
macro_rules! test_log {
	($test_name:expr, $($arg:tt)*) => {
		log::info!("[{}] {}", $test_name, format!($($arg)*))
	};
}

#[macro_export]
macro_rules! test_warn {
	($test_name:expr, $($arg:tt)*) => {
		log::warn!("[{}] {}", $test_name, format!($($arg)*))
	};
}

#[macro_export]
macro_rules! test_error {
	($test_name:expr, $($arg:tt)*) => {
		log::error!("[{}] {}", $test_name, format!($($arg)*))
	};
}

pub mod bitswap;
pub mod config;
pub mod crypto;
pub mod ldb;
pub mod network;
pub mod sync;
pub mod tx;

pub use bitswap::*;
pub use config::*;
pub use crypto::*;
pub use ldb::*;
pub use network::*;
pub use sync::*;
pub use tx::*;
