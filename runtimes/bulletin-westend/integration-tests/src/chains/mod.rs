// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

//! Emulated chain definitions for XCM integration tests.
//!
//! These modules define the Westend relay chain and Asset Hub Westend parachain
//! for use with the xcm-emulator framework. They are inlined here (rather than
//! depending on the upstream `westend-emulated-chain` and
//! `asset-hub-westend-emulated-chain` crates) because those crates are not
//! published to crates.io.

pub mod asset_hub_westend;
pub mod westend;

pub use asset_hub_westend::AssetHubWestend;
pub use westend::Westend;
