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

//! Transaction extension for data-renewal pallet signed calls.
//!
//! Mirrors `pallet-bulletin-transaction-storage::extension::ValidateStorageCalls`
//! for the renewal-pallet `Call` variants. Direct signed calls (`renew`,
//! `force_renew`, `enable_auto_renew`, `disable_auto_renew`) are validated against
//! the caller's authorization, hard-cap accounting is consumed in `prepare`, and
//! the origin is rewritten to `pallet_bulletin_transaction_storage::Origin::Authorized`
//! so the dispatchable's `ensure_authorized` accepts it.
//!
//! limitation: this extension does not recurse into wrapper calls. Renew calls
//! must be submitted as direct extrinsics. Wrapper-inspection support can be added
//! later once `CallInspector` is generalized across pallets.

use crate::{Call, Config, Pallet, WeightInfo};
use codec::{Decode, DecodeWithMemTracking, Encode};
use core::{fmt, marker::PhantomData};
use pallet_bulletin_transaction_storage::{AuthorizationScope, Origin as TxStorageOrigin};
use polkadot_sdk_frame::{
	deps::*,
	prelude::*,
	traits::{AsSystemOriginSigner, Implication, OriginTrait},
};

type RuntimeCallOf<T> = <T as frame_system::Config>::RuntimeCall;

#[derive(Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, scale_info::TypeInfo)]
#[codec(encode_bound())]
#[codec(decode_bound())]
#[codec(mel_bound())]
#[scale_info(skip_type_params(T))]
pub struct ValidateRenewalCalls<T>(PhantomData<T>);

impl<T> Default for ValidateRenewalCalls<T> {
	fn default() -> Self {
		Self(PhantomData)
	}
}

impl<T: Config + Send + Sync> fmt::Debug for ValidateRenewalCalls<T> {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "ValidateRenewalCalls")
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
		Ok(())
	}
}

impl<T: Config + Send + Sync> TransactionExtension<RuntimeCallOf<T>> for ValidateRenewalCalls<T>
where
	RuntimeCallOf<T>: IsSubType<Call<T>>,
	T::RuntimeOrigin: OriginTrait + AsSystemOriginSigner<T::AccountId> + From<TxStorageOrigin<T>>,
	<T::RuntimeOrigin as OriginTrait>::PalletsOrigin:
		From<TxStorageOrigin<T>> + TryInto<TxStorageOrigin<T>>,
{
	const IDENTIFIER: &'static str = "ValidateRenewalCalls";

	type Implicit = ();
	fn implicit(&self) -> Result<Self::Implicit, TransactionValidityError> {
		Ok(())
	}

	type Val = Option<T::AccountId>;
	type Pre = ();

	fn weight(&self, call: &RuntimeCallOf<T>) -> Weight {
		let Some(inner) = call.is_sub_type() else {
			return Weight::zero();
		};
		match inner {
			Call::renew { .. } |
			Call::force_renew { .. } |
			Call::enable_auto_renew { .. } |
			Call::disable_auto_renew { .. } => <T as Config>::WeightInfo::validate_renew(),
			_ => Weight::zero(),
		}
	}

	fn validate(
		&self,
		mut origin: T::RuntimeOrigin,
		call: &RuntimeCallOf<T>,
		_info: &DispatchInfoOf<RuntimeCallOf<T>>,
		_len: usize,
		_self_implicit: Self::Implicit,
		_inherited_implication: &impl Implication,
		_source: TransactionSource,
	) -> ValidateResult<Self::Val, RuntimeCallOf<T>> {
		// Pass through unsigned + non-system origins (XCM, custom origins, etc.).
		let who = match origin.as_system_origin_signer() {
			Some(who) => who.clone(),
			None => return Ok((ValidTransaction::default(), None, origin)),
		};

		let Some(inner) = call.is_sub_type() else {
			return Ok((ValidTransaction::default(), None, origin));
		};

		let (valid_tx, maybe_scope) = Pallet::<T>::validate_renewal_signed(&who, inner)?;
		if let Some(scope) = maybe_scope {
			origin.set_caller_from(TxStorageOrigin::<T>::Authorized { who: who.clone(), scope });
		}
		Ok((valid_tx, Some(who), origin))
	}

	fn prepare(
		self,
		val: Self::Val,
		_origin: &T::RuntimeOrigin,
		call: &RuntimeCallOf<T>,
		_info: &DispatchInfoOf<RuntimeCallOf<T>>,
		_len: usize,
	) -> Result<Self::Pre, TransactionValidityError> {
		let Some(who) = val else { return Ok(()) };
		let Some(inner) = call.is_sub_type() else { return Ok(()) };
		Pallet::<T>::pre_dispatch_renewal_signed(&who, inner).map(|_| ())
	}
}

/// Output of `validate_renewal_signed` / `check_renewal_signed`. Holds the
/// pool-side `ValidTransaction` metadata and the [`AuthorizationScope`] that
/// must be carried via origin rewrite (`Some` for renewal calls; `None` for
/// non-renewal Call variants the extension passes through).
pub type RenewalValidation<T> =
	(ValidTransaction, Option<AuthorizationScope<<T as frame_system::Config>::AccountId>>);

impl<T: Config> Pallet<T> {
	/// Validate a signed renewal call without consuming authorization.
	///
	/// Returns the `ValidTransaction` metadata for the pool and, for calls that
	/// require origin rewriting (all four renewal dispatchables), the
	/// [`AuthorizationScope`] consumed.
	pub fn validate_renewal_signed(
		who: &T::AccountId,
		call: &Call<T>,
	) -> Result<RenewalValidation<T>, TransactionValidityError> {
		Self::check_renewal_signed(
			who,
			call,
			pallet_bulletin_transaction_storage::CheckContext::Validate,
		)
	}

	/// Pre-dispatch a signed renewal call. Consumes the authorization extent so
	/// the dispatchable runs against the post-consumption state.
	pub fn pre_dispatch_renewal_signed(
		who: &T::AccountId,
		call: &Call<T>,
	) -> Result<(), TransactionValidityError> {
		Self::check_renewal_signed(
			who,
			call,
			pallet_bulletin_transaction_storage::CheckContext::PreDispatch,
		)
		.map(|_| ())
	}

	fn check_renewal_signed(
		who: &T::AccountId,
		call: &Call<T>,
		context: pallet_bulletin_transaction_storage::CheckContext,
	) -> Result<RenewalValidation<T>, TransactionValidityError> {
		use pallet_bulletin_transaction_storage as txs;

		let consume = matches!(context, txs::CheckContext::PreDispatch);
		let want_valid = matches!(context, txs::CheckContext::Validate);

		match call {
			Call::renew { entry } => {
				let info = txs::Pallet::<T>::resolve_transaction_ref(entry)
					.map_err(|_| txs::RENEWED_NOT_FOUND)?;
				if crate::AutoRenewals::<T>::contains_key(info.content_hash) {
					return Err(txs::AUTO_RENEWAL_ALREADY_ENABLED.into());
				}
				txs::Pallet::<T>::check_authorization(
					&AuthorizationScope::Account(who.clone()),
					info.size,
					consume,
					true,
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
					.map_err(|_| txs::RENEWED_NOT_FOUND)?;
				txs::Pallet::<T>::check_authorization(
					&AuthorizationScope::Account(who.clone()),
					info.size,
					consume,
					true,
				)?;
				let scope = AuthorizationScope::Account(who.clone());
				let valid = if want_valid {
					ValidTransaction::with_tag_prefix("DataRenewalForceRenew")
						.and_provides((who.clone(), info.content_hash))
						.priority(<T as txs::Config>::StoreRenewPriority::get())
						.longevity(<T as txs::Config>::StoreRenewLongevity::get())
						.into()
				} else {
					ValidTransaction::default()
				};
				Ok((valid, Some(scope)))
			},
			Call::enable_auto_renew { content_hash } => {
				if crate::AutoRenewals::<T>::contains_key(*content_hash) {
					return Err(txs::AUTO_RENEWAL_ALREADY_ENABLED.into());
				}
				let (block, index) = txs::Pallet::<T>::lookup_by_content_hash(*content_hash)
					.ok_or(txs::RENEWED_NOT_FOUND)?;
				let info = txs::Pallet::<T>::transaction_info(block, index)
					.ok_or(txs::RENEWED_NOT_FOUND)?;

				txs::Pallet::<T>::check_authorization(
					&AuthorizationScope::Account(who.clone()),
					info.size,
					consume,
					true,
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
				let renewal_data = crate::AutoRenewals::<T>::get(content_hash)
					.ok_or(txs::AUTO_RENEWAL_NOT_ENABLED)?;
				if &renewal_data.account != who {
					return Err(txs::NOT_AUTO_RENEWAL_OWNER.into());
				}
				if renewal_data.paid {
					return Err(txs::CANNOT_DISABLE_PREPAID_AUTO_RENEWAL.into());
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
			Call::process_pending_renewals { .. } => {
				// Mandatory inherent — not a signed transaction. Reject signed submission.
				Err(InvalidTransaction::Call.into())
			},
			_ => Err(InvalidTransaction::Call.into()),
		}
	}
}
