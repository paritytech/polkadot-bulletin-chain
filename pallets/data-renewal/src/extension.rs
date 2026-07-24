// This file is part of Substrate.

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

//! This pallet's [`LeafValidator`] plug-in for the storage pallet's
//! [`ValidateAuthorizedCalls`] extension, plus the renewal-side validators it
//! delegates to.
//!
//! The runtime wires `ValidateAuthorizedCalls<Runtime, Inspector, (StorageLeaves,
//! RenewalLeaves)>`: one walk over the call tree offers every leaf to each
//! validator; [`RenewalLeaves`] claims this pallet's dispatchables. All four are
//! direct-only (rejected at wrapper depth > 0) — their pool tags and prepaid
//! charges assume one leaf per extrinsic.

use crate::{Call, Config, Pallet, WeightInfo as _};
use core::marker::PhantomData;
use pallet_bulletin_transaction_storage::{
	self as txs, extension::LeafValidator, AuthorizationScope,
};
use polkadot_sdk_frame::{deps::*, prelude::*};

type RuntimeCallOf<T> = <T as frame_system::Config>::RuntimeCall;

/// Output of [`Pallet::validate_renewal_signed`] / [`Pallet::check_renewal_signed`]:
/// the pool-side `ValidTransaction` plus the [`AuthorizationScope`] consumed (if
/// any), to be carried via an `Origin::Authorized` rewrite by the extension.
pub type RenewalValidation<T> =
	(ValidTransaction, Option<AuthorizationScope<<T as frame_system::Config>::AccountId>>);

/// [`LeafValidator`] for this pallet's calls. All four renewal dispatchables are
/// direct-only: their pool `provides` tags and the prepaid charge accounting
/// assume exactly one leaf per extrinsic.
pub struct RenewalLeaves<T>(PhantomData<T>);

impl<T: Config> LeafValidator<T> for RenewalLeaves<T>
where
	RuntimeCallOf<T>: IsSubType<txs::Call<T>> + IsSubType<Call<T>>,
{
	fn validate_leaf(
		who: &T::AccountId,
		call: &RuntimeCallOf<T>,
		depth: u32,
	) -> Result<txs::extension::LeafValidation<T>, TransactionValidityError> {
		let Some(inner_call) = <_ as IsSubType<Call<T>>>::is_sub_type(call) else {
			return Ok(None);
		};
		if depth > 0 {
			return Err(InvalidTransaction::Call.into());
		}
		Pallet::<T>::validate_renewal_signed(who, inner_call).map(Some)
	}

	fn prepare_leaf(
		who: &T::AccountId,
		call: &RuntimeCallOf<T>,
		depth: u32,
	) -> Result<bool, TransactionValidityError> {
		let Some(inner_call) = <_ as IsSubType<Call<T>>>::is_sub_type(call) else {
			return Ok(false);
		};
		if depth > 0 {
			return Err(InvalidTransaction::Call.into());
		}
		Pallet::<T>::pre_dispatch_renewal_signed(who, inner_call).map(|_| true)
	}

	fn leaf_weight(call: &RuntimeCallOf<T>) -> Weight {
		match <_ as IsSubType<Call<T>>>::is_sub_type(call) {
			Some(
				Call::renew { .. } |
				Call::force_renew { .. } |
				Call::enable_auto_renew { .. } |
				Call::disable_auto_renew { .. },
			) => <T as Config>::WeightInfo::validate_renew(),
			_ => Weight::zero(),
		}
	}
}

// -----------------------------------------------------------------------------
// Renewal-pallet validators (called by RenewalLeaves above).
// -----------------------------------------------------------------------------

impl<T: Config> Pallet<T> {
	/// Pool-time validation for a signed renewal call. Returns the
	/// [`ValidTransaction`] metadata and the [`AuthorizationScope`] to set on
	/// the rewritten origin.
	pub fn validate_renewal_signed(
		who: &T::AccountId,
		call: &Call<T>,
	) -> Result<RenewalValidation<T>, TransactionValidityError> {
		Self::check_renewal_signed(who, call, txs::CheckContext::Validate)
	}

	/// Pre-dispatch counterpart: consumes the authorization extent so the
	/// dispatchable runs against post-consumption state.
	pub(crate) fn pre_dispatch_renewal_signed(
		who: &T::AccountId,
		call: &Call<T>,
	) -> Result<(), TransactionValidityError> {
		Self::check_renewal_signed(who, call, txs::CheckContext::PreDispatch).map(|_| ())
	}

	fn check_renewal_signed(
		who: &T::AccountId,
		call: &Call<T>,
		context: txs::CheckContext,
	) -> Result<RenewalValidation<T>, TransactionValidityError> {
		let consume = matches!(context, txs::CheckContext::PreDispatch);
		let want_valid = matches!(context, txs::CheckContext::Validate);

		match call {
			Call::renew { entry } => {
				let info = txs::Pallet::<T>::resolve_transaction_ref(entry)
					.map_err(|_| crate::RENEWED_NOT_FOUND)?;
				if crate::Renewals::<T>::contains_key(info.content_hash) {
					return Err(crate::AUTO_RENEWAL_ALREADY_ENABLED.into());
				}
				Pallet::<T>::check_renew_authorization(
					&AuthorizationScope::Account(who.clone()),
					info.size,
					consume,
				)?;
				let scope = AuthorizationScope::Account(who.clone());
				let valid = if want_valid {
					ValidTransaction::with_tag_prefix("DataRenewalRenew")
						.and_provides((who.clone(), info.content_hash))
						.priority(<T as txs::Config>::StoreRenewPriority::get())
						.longevity(<T as txs::Config>::StoreRenewLongevity::get())
						.into()
				} else {
					ValidTransaction::default()
				};
				Ok((valid, Some(scope)))
			},
			Call::force_renew { entry } => {
				let info = txs::Pallet::<T>::resolve_transaction_ref(entry)
					.map_err(|_| crate::RENEWED_NOT_FOUND)?;
				// Prefer a preimage grant, fall back to the caller's account quota.
				let scope =
					Pallet::<T>::authorize_renew(who, info.content_hash, info.size, consume)?;
				let valid = if want_valid {
					match &scope {
						// Preimage renewals are submitter-agnostic: tag on the content
						// hash alone so they don't dedup against the account path.
						AuthorizationScope::Preimage(hash) =>
							ValidTransaction::with_tag_prefix("DataRenewalForceRenewPreimage")
								.and_provides(*hash)
								.priority(<T as txs::Config>::StoreRenewPriority::get())
								.longevity(<T as txs::Config>::StoreRenewLongevity::get())
								.into(),
						_ => ValidTransaction::with_tag_prefix("DataRenewalForceRenew")
							.and_provides((who.clone(), info.content_hash))
							.priority(<T as txs::Config>::StoreRenewPriority::get())
							.longevity(<T as txs::Config>::StoreRenewLongevity::get())
							.into(),
					}
				} else {
					ValidTransaction::default()
				};
				Ok((valid, Some(scope)))
			},
			Call::enable_auto_renew { content_hash } => {
				if crate::Renewals::<T>::contains_key(*content_hash) {
					return Err(crate::AUTO_RENEWAL_ALREADY_ENABLED.into());
				}
				let (block, index) = txs::Pallet::<T>::lookup_by_content_hash(*content_hash)
					.ok_or(crate::RENEWED_NOT_FOUND)?;
				let info = txs::Pallet::<T>::transaction_info(block, index)
					.ok_or(crate::RENEWED_NOT_FOUND)?;

				Pallet::<T>::check_renew_authorization(
					&AuthorizationScope::Account(who.clone()),
					info.size,
					consume,
				)?;

				let scope = AuthorizationScope::Account(who.clone());
				let valid = if want_valid {
					ValidTransaction::with_tag_prefix("DataRenewalEnable")
						.and_provides((who.clone(), info.content_hash))
						.priority(<T as txs::Config>::StoreRenewPriority::get())
						.longevity(<T as txs::Config>::StoreRenewLongevity::get())
						.into()
				} else {
					ValidTransaction::default()
				};
				Ok((valid, Some(scope)))
			},
			Call::disable_auto_renew { content_hash } => {
				let renewal_data = crate::Renewals::<T>::get(content_hash)
					.ok_or(crate::AUTO_RENEWAL_NOT_ENABLED)?;
				if &renewal_data.account != who {
					return Err(crate::NOT_AUTO_RENEWAL_OWNER.into());
				}
				if renewal_data.paid {
					return Err(crate::CANNOT_DISABLE_PREPAID_AUTO_RENEWAL.into());
				}
				let scope = AuthorizationScope::Account(who.clone());
				let valid = if want_valid {
					ValidTransaction {
						priority: <T as txs::Config>::StoreRenewPriority::get(),
						longevity: <T as txs::Config>::StoreRenewLongevity::get(),
						..Default::default()
					}
				} else {
					ValidTransaction::default()
				};
				Ok((valid, Some(scope)))
			},
			// `process_pending_renewals` is a mandatory inherent — never signed.
			_ => Err(InvalidTransaction::Call.into()),
		}
	}
}
