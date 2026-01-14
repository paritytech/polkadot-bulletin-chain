// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! HOP (Hand-Off Protocol) implementation.
//!
//! Provides ephemeral short-term data storage for 24 hours with RPC upload and Bitswap retrieval.
//! Data is automatically promoted to Bulletin Chain middle-term storage on timeout,
//! or dropped if user doesn't have enough allowance.

pub mod cli;
pub mod pool;
pub mod rpc;
pub mod types;

pub use cli::HopParams;
pub use pool::HopDataPool;
pub use types::PoolStatus;
