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

//! `authorize_account` semantics: happy path, additivity, replacement on
//! expiry, and per-account scoping. Thin wrappers around
//! `bulletin-runtimes-test-utils`.

use super::*;

#[test]
fn happy_path_from_sibling() {
	utils::xcm_authorize_happy_path::<Runtime, XcmConfig>(
		pc_location(),
		new_test_ext,
		advance_block,
	);
}

#[test]
fn additive_within_window() {
	utils::xcm_authorize_is_additive_within_window::<Runtime, XcmConfig>(
		pc_location(),
		new_test_ext,
		advance_block,
	);
}

#[test]
fn replaces_after_expiry() {
	utils::xcm_authorize_replaces_after_expiry::<Runtime, XcmConfig>(
		pc_location(),
		new_test_ext,
		advance_block,
	);
}

#[test]
fn account_scopes_are_independent() {
	utils::xcm_account_scopes_are_independent::<Runtime, XcmConfig>(
		pc_location(),
		new_test_ext,
		advance_block,
	);
}
