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

//! Single transaction extension that validates both storage- and renewal-pallet
//! calls in one pass.
//!
//! `validate` walks the call tree once through
//! [`CallInspector::inspect_wrapper`], visiting every direct `Call<T>` leaf and
//! dispatching it to the matching pallet's `validate_signed` /
//! `validate_renewal_signed`. `prepare` repeats the walk to consume the
//! authorization. The origin is rewritten to
//! [`pallet_bulletin_transaction_storage::Origin::Authorized`] once, after the
//! walk. Wrapped `store` / `store_with_cid_config` and any renewal dispatchable
//! are rejected at depth > 0 — those must be direct extrinsics.

use crate::{Call, Config, Pallet, WeightInfo as _};
use codec::{Decode, DecodeWithMemTracking, Encode};
use core::{fmt, marker::PhantomData};
use pallet_bulletin_transaction_storage::{
	self as txs, extension::MAX_WRAPPER_DEPTH, AuthorizationScope, CallInspector,
	Origin as TxStorageOrigin, WeightInfo as _,
};
use polkadot_sdk_frame::{
	deps::*,
	prelude::*,
	traits::{AsSystemOriginSigner, Implication, OriginTrait},
};

type RuntimeCallOf<T> = <T as frame_system::Config>::RuntimeCall;

/// Output of [`Pallet::validate_renewal_signed`] / [`Pallet::check_renewal_signed`]:
/// the pool-side `ValidTransaction` plus the [`AuthorizationScope`] consumed (if
/// any), to be carried via an `Origin::Authorized` rewrite by the extension.
pub type RenewalValidation<T> =
	(ValidTransaction, Option<AuthorizationScope<<T as frame_system::Config>::AccountId>>);

/// Single extension that validates both storage- and renewal-pallet calls. The
/// `I` type parameter supplies the wrapper inspector (e.g. `Utility::batch`
/// unwrap) used by both validators in a single tree walk.
#[derive(Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, scale_info::TypeInfo)]
#[codec(encode_bound())]
#[codec(decode_bound())]
#[codec(mel_bound())]
#[scale_info(skip_type_params(T, I))]
pub struct ValidateBulletinCalls<T, I = ()>(PhantomData<(T, I)>);

impl<T, I> Default for ValidateBulletinCalls<T, I> {
	fn default() -> Self {
		Self(PhantomData)
	}
}

impl<T: Config + Send + Sync, I: Send + Sync + 'static> fmt::Debug for ValidateBulletinCalls<T, I> {
	#[cfg(feature = "std")]
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "ValidateBulletinCalls")
	}

	#[cfg(not(feature = "std"))]
	fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result {
		Ok(())
	}
}

impl<T: Config + Send + Sync, I: CallInspector<T> + Send + Sync + 'static>
	TransactionExtension<RuntimeCallOf<T>> for ValidateBulletinCalls<T, I>
where
	RuntimeCallOf<T>: IsSubType<txs::Call<T>> + IsSubType<Call<T>>,
	T::RuntimeOrigin: OriginTrait + AsSystemOriginSigner<T::AccountId> + From<TxStorageOrigin<T>>,
	<T::RuntimeOrigin as OriginTrait>::PalletsOrigin:
		From<TxStorageOrigin<T>> + TryInto<TxStorageOrigin<T>>,
{
	const IDENTIFIER: &'static str = "ValidateBulletinCalls";

	type Implicit = ();
	fn implicit(&self) -> Result<Self::Implicit, TransactionValidityError> {
		Ok(())
	}

	/// Signer (when the tx is signed and any pallet call was visited); drives
	/// `prepare`'s second walk.
	type Val = Option<T::AccountId>;
	type Pre = ();

	fn weight(&self, call: &RuntimeCallOf<T>) -> Weight {
		if let Some(inner) = <_ as IsSubType<txs::Call<T>>>::is_sub_type(call) {
			return match inner {
				txs::Call::store { data, .. } | txs::Call::store_with_cid_config { data, .. } =>
					<T as txs::Config>::WeightInfo::validate_store(data.len() as u32),
				_ => Weight::zero(),
			};
		}
		if let Some(inner) = <_ as IsSubType<Call<T>>>::is_sub_type(call) {
			return match inner {
				Call::renew { .. } |
				Call::force_renew { .. } |
				Call::enable_auto_renew { .. } |
				Call::disable_auto_renew { .. } => <T as Config>::WeightInfo::validate_renew(),
				_ => Weight::zero(),
			};
		}
		Weight::zero()
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
		// Unsigned + non-system origins pass through.
		let who = match origin.as_system_origin_signer() {
			Some(w) => w.clone(),
			None => return Ok((ValidTransaction::default(), None, origin)),
		};

		let mut combined = ValidTransaction::default();
		let mut last_scope: Option<AuthorizationScope<T::AccountId>> = None;
		let mut visited_any = false;

		Self::walk::<I, _>(call, 0, &mut |leaf, depth| {
			if let Some(c) = <_ as IsSubType<txs::Call<T>>>::is_sub_type(leaf) {
				if depth > 0 && Self::is_direct_only_storage_call(c) {
					return Err(InvalidTransaction::Call.into());
				}
				let (vt, maybe_scope) = txs::Pallet::<T>::validate_signed(&who, c)?;
				combined = core::mem::take(&mut combined).combine_with(vt);
				if let Some(s) = maybe_scope {
					last_scope = Some(s);
				}
				visited_any = true;
				return Ok(());
			}
			if let Some(c) = <_ as IsSubType<Call<T>>>::is_sub_type(leaf) {
				if depth > 0 {
					// All four renewal dispatchables must be direct extrinsics.
					return Err(InvalidTransaction::Call.into());
				}
				let (vt, maybe_scope) = Pallet::<T>::validate_renewal_signed(&who, c)?;
				combined = core::mem::take(&mut combined).combine_with(vt);
				if let Some(s) = maybe_scope {
					last_scope = Some(s);
				}
				visited_any = true;
				return Ok(());
			}
			Ok(())
		})?;

		if let Some(scope) = last_scope {
			origin.set_caller_from(TxStorageOrigin::<T>::Authorized { who: who.clone(), scope });
		}
		Ok((combined, visited_any.then_some(who), origin))
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
		Self::walk::<I, _>(call, 0, &mut |leaf, depth| {
			if let Some(c) = <_ as IsSubType<txs::Call<T>>>::is_sub_type(leaf) {
				if depth > 0 && Self::is_direct_only_storage_call(c) {
					return Err(InvalidTransaction::Call.into());
				}
				return txs::Pallet::<T>::pre_dispatch_signed(&who, c).map(|_| ());
			}
			if let Some(c) = <_ as IsSubType<Call<T>>>::is_sub_type(leaf) {
				if depth > 0 {
					return Err(InvalidTransaction::Call.into());
				}
				return Pallet::<T>::pre_dispatch_renewal_signed(&who, c);
			}
			Ok(())
		})
	}
}

impl<T: Config, I> ValidateBulletinCalls<T, I> {
	/// Storage-pallet calls that must arrive as direct extrinsics (rejected at
	/// any wrapper depth > 0). Management calls (`authorize_*`, `refresh_*`,
	/// `remove_*`) are intentionally allowed inside wrappers.
	fn is_direct_only_storage_call(call: &txs::Call<T>) -> bool {
		matches!(call, txs::Call::store { .. } | txs::Call::store_with_cid_config { .. })
	}

	/// Visit each direct storage- or renewal-pallet `Call` in the tree exactly
	/// once. Unwraps wrappers (e.g. `Utility::batch`) through
	/// [`CallInspector::inspect_wrapper`]; bails out at [`MAX_WRAPPER_DEPTH`]
	/// with `ExhaustsResources`. The visitor receives the leaf and its current
	/// wrapper depth so it can apply direct-only restrictions.
	fn walk<II, F>(
		call: &RuntimeCallOf<T>,
		depth: u32,
		visitor: &mut F,
	) -> Result<(), TransactionValidityError>
	where
		II: CallInspector<T>,
		RuntimeCallOf<T>: IsSubType<txs::Call<T>> + IsSubType<Call<T>>,
		F: FnMut(&RuntimeCallOf<T>, u32) -> Result<(), TransactionValidityError>,
	{
		let is_storage_leaf = <_ as IsSubType<txs::Call<T>>>::is_sub_type(call).is_some();
		let is_renewal_leaf = <_ as IsSubType<Call<T>>>::is_sub_type(call).is_some();
		if is_storage_leaf || is_renewal_leaf {
			return visitor(call, depth);
		}
		if depth >= MAX_WRAPPER_DEPTH {
			// Fail-safe: refuse rather than risk letting a hidden direct-only
			// call slip through. Matches the legacy storage extension's
			// behaviour for the same case.
			return Err(InvalidTransaction::Call.into());
		}
		if let Some(inner_calls) = II::inspect_wrapper(call) {
			for inner in inner_calls {
				Self::walk::<II, F>(inner, depth + 1, visitor)?;
			}
		}
		Ok(())
	}
}

// -----------------------------------------------------------------------------
// Renewal-pallet validators (called by the combined extension above).
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
	pub fn pre_dispatch_renewal_signed(
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
				// Prefer a preimage grant, fall back to the caller's account quota.
				let scope =
					txs::Pallet::<T>::authorize_renew(who, info.content_hash, info.size, consume)?;
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
			// `process_pending_renewals` is a mandatory inherent — never signed.
			_ => Err(InvalidTransaction::Call.into()),
		}
	}
}
