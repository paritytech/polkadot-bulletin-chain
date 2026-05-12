// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Cumulus.
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

//! `SafeCallFilter`: storage-mutating calls must not reach dispatch over XCM.
//! Thin wrappers around `bulletin-runtimes-test-utils`.

use super::*;

#[test]
fn sibling_xcm_store_is_blocked() {
	utils::xcm_store_is_blocked::<Runtime, XcmConfig>(pc_location(), new_test_ext, advance_block);
}

#[test]
fn sibling_xcm_batch_with_store_is_entirely_blocked() {
	utils::xcm_batch_with_store_is_entirely_blocked::<Runtime, XcmConfig>(
		pc_location(),
		new_test_ext,
		advance_block,
	);
}

#[test]
fn sibling_xcm_batch_of_only_authorize_calls_succeeds() {
	utils::xcm_batch_of_only_authorize_calls_succeeds::<Runtime, XcmConfig>(
		pc_location(),
		new_test_ext,
		advance_block,
	);
}
