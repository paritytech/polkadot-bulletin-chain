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

use crate::{Config, RetentionPeriod, LOG_TARGET};
use core::marker::PhantomData;
use polkadot_sdk_frame::{
	prelude::{BlockNumberFor, Weight},
	traits::{Get, OnRuntimeUpgrade, Zero},
};

/// Runtime migration that sets the `RetentionPeriod` storage item to a
/// non-zero `NewValue` value **only if it is currently zero**.
///
/// Idempotent migration: safe to run multiple times
pub struct SetRetentionPeriodIfZero<T, NewValue>(PhantomData<(T, NewValue)>);
impl<T: Config, NewValue: Get<BlockNumberFor<T>>> OnRuntimeUpgrade
	for SetRetentionPeriodIfZero<T, NewValue>
{
	fn on_runtime_upgrade() -> Weight {
		let mut weight = T::DbWeight::get().reads(1);

		// If zero, let's reset.
		if RetentionPeriod::<T>::get().is_zero() {
			RetentionPeriod::<T>::set(NewValue::get());
			weight.saturating_accrue(T::DbWeight::get().writes(1));

			tracing::warn!(
				target: LOG_TARGET,
				new_value = ?NewValue::get(),
				"[SetRetentionPeriodIfZero] RetentionPeriod was zero, resetting to:",
			);
		}

		weight
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(
		_state: alloc::vec::Vec<u8>,
	) -> Result<(), polkadot_sdk_frame::deps::sp_runtime::DispatchError> {
		polkadot_sdk_frame::prelude::ensure!(
			!RetentionPeriod::<T>::get().is_zero(),
			"must be migrate to the `NewValue`."
		);

		tracing::info!(target: LOG_TARGET, "SetRetentionPeriodIfZero is OK!");
		Ok(())
	}
}
