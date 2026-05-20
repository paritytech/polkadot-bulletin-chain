// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

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
