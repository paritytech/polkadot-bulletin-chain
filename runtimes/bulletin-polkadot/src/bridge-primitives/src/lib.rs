// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Polkadot.

// Polkadot is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Polkadot is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Polkadot.  If not, see <http://www.gnu.org/licenses/>.

//! Module with configuration which reflects Bulletin runtime setup
//! (AccountId, Headers, Hashes...)

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

// TODO: move here the stuff from
// bp-polkadot-bulletin = { git = "https://github.com/paritytech/polkadot-sdk.git", rev = "a64eb1fb02d4012948cba024fca2f27d94732e52", default-features = false }
