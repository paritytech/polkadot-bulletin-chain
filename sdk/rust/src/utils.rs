// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Utility functions for Bulletin SDK.
//!
//! This module is intentionally minimal. For generic utilities, use:
//! - `hex` crate for hex encoding/decoding
//! - `sp_core::crypto::Ss58Codec` for SS58 address conversion
//! - `sp_io::hashing` for hashing functions
//! - `backoff` or `tokio-retry` crates for retry logic
//!
//! For fee estimation, use subxt's `payment_queryInfo` RPC.
