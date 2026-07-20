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

//! Runtime-configurable parameters managed by `pallet-parameters`.
//!
//! Replaces the `parameter_types! { pub storage … }` items previously used in
//! `storage.rs` and `xcm_config.rs`. Governance updates each value via a
//! typed `Parameters::set_parameter` call, which decodes runtime-side and
//! emits an `Updated { key, old_value, new_value }` event for audit.

use crate::{Runtime, RuntimeOrigin};
use alloc::vec;
use frame_support::{
	dynamic_params::{dynamic_pallet_params, dynamic_params},
	traits::{ConstU32, EnsureOriginWithArg},
	BoundedVec,
};

/// Upper bound on the number of paraIds in either parachain allowlist.
/// Picked well above the realistic count of trusted governance / authorizer
/// parachains so growth doesn't require a runtime upgrade. Cheap to raise
/// later (`BoundedVec` storage encoding is length-prefixed).
pub const MAX_ALLOWED_PARACHAIN_IDS: u32 = 16;

/// Cap on the total bytes committed to permanent storage (via `renew`) across
/// all authorizations on this chain. Default 1.7 TiB.
pub const DEFAULT_MAX_PERMANENT_STORAGE_SIZE: u64 = 17 * 1024 * 1024 * 1024 * 1024 / 10;

#[dynamic_params(RuntimeParameters, pallet_parameters::Parameters::<Runtime>)]
pub mod dynamic_params {
	use super::*;

	/// Storage-pallet knobs.
	#[dynamic_pallet_params]
	#[codec(index = 0)]
	pub mod storage {
		/// See [`super::DEFAULT_MAX_PERMANENT_STORAGE_SIZE`].
		#[codec(index = 0)]
		pub static MaxPermanentStorageSize: u64 = DEFAULT_MAX_PERMANENT_STORAGE_SIZE;

		/// Validity window of a storage authorization, in blocks. Default 14 days.
		#[codec(index = 1)]
		pub static AuthorizationPeriod: crate::BlockNumber = 14 * crate::DAYS;
	}

	/// XCM origin allowlists.
	#[dynamic_pallet_params]
	#[codec(index = 1)]
	pub mod xcm {
		/// Sibling paraIds allowed to dispatch storage authorizations via XCM.
		/// Defaults to the People chain paraIds on previewnet / paseo-next-v2.
		#[codec(index = 0)]
		pub static AllowedParachainIds: BoundedVec<u32, ConstU32<MAX_ALLOWED_PARACHAIN_IDS>> =
			BoundedVec::truncate_from(vec![1502, 5140]);

		/// Sibling paraIds whose origins are treated as governance (granted
		/// Root via XCM). Defaults to the paseo-next-v2 governance chain.
		#[codec(index = 1)]
		pub static GovernanceParachainIds: BoundedVec<u32, ConstU32<MAX_ALLOWED_PARACHAIN_IDS>> =
			BoundedVec::truncate_from(vec![1500]);
	}
}

/// Routes parameter updates by key. All keys currently require Root, matching
/// the trust surface of `system.set_storage` that the previous `pub storage`
/// items used. Wired this way so future per-key tightening (e.g. Fellowship
/// for `GovernanceParachainIds`) is a localized change to this `match`.
pub struct DynamicParameterOrigin;
impl EnsureOriginWithArg<RuntimeOrigin, RuntimeParametersKey> for DynamicParameterOrigin {
	type Success = ();

	fn try_origin(
		origin: RuntimeOrigin,
		key: &RuntimeParametersKey,
	) -> Result<Self::Success, RuntimeOrigin> {
		use RuntimeParametersKey::*;
		match key {
			Storage(_) | Xcm(_) => frame_system::ensure_root(origin.clone()),
		}
		.map_err(|_| origin)
	}

	#[cfg(feature = "runtime-benchmarks")]
	fn try_successful_origin(_key: &RuntimeParametersKey) -> Result<RuntimeOrigin, ()> {
		Ok(RuntimeOrigin::root())
	}
}

/// Default used by `runtime-benchmarks` to materialize a `RuntimeParameters`
/// value. Picks an arbitrary parameter (the storage cap) with its default.
#[cfg(feature = "runtime-benchmarks")]
impl Default for RuntimeParameters {
	fn default() -> Self {
		RuntimeParameters::Storage(dynamic_params::storage::Parameters::MaxPermanentStorageSize(
			dynamic_params::storage::MaxPermanentStorageSize,
			None,
		))
	}
}
