// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Utilities for zombienet-sdk tests

/// Log macro that prefixes messages with test name for parallel test runs.
/// Usage: `test_log!(TEST, "message {}", arg);`
#[macro_export]
macro_rules! test_log {
	($test_name:expr, $($arg:tt)*) => {
		log::info!("[{}] {}", $test_name, format!($($arg)*))
	};
}

pub mod chainspec;
pub mod config;
pub mod crypto;
pub mod network;
pub mod subxt_config;
pub mod sync;
pub mod tx;

pub use chainspec::*;
pub use config::*;
pub use crypto::*;
pub use network::*;
pub use subxt_config::*;
pub use sync::*;
pub use tx::*;
