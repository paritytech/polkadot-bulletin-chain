// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Transaction submitter implementations for different blockchain client libraries.
//!
//! This module provides concrete implementations of the [`TransactionSubmitter`](crate::submit::TransactionSubmitter)
//! trait for various blockchain interaction libraries:
//!
//! - [`SubxtSubmitter`] - Uses the `subxt` library for type-safe blockchain interaction
//!
//! ## Creating Custom Submitters
//!
//! You can implement your own submitter for any blockchain client library by implementing
//! the `TransactionSubmitter` trait:
//!
//! ```ignore
//! use bulletin_sdk_rust::submit::{TransactionSubmitter, TransactionReceipt};
//! use async_trait::async_trait;
//!
//! pub struct MyCustomSubmitter {
//!     // Your client fields
//! }
//!
//! #[async_trait]
//! impl TransactionSubmitter for MyCustomSubmitter {
//!     async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt> {
//!         // Your implementation
//!     }
//!     // ... implement other methods
//! }
//! ```

#[cfg(feature = "std")]
pub mod subxt_submitter;

#[cfg(feature = "std")]
pub mod mock_submitter;

#[cfg(feature = "std")]
pub use subxt_submitter::SubxtSubmitter;

#[cfg(feature = "std")]
pub use mock_submitter::MockSubmitter;
