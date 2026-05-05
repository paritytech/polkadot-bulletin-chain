//! Runtime API for querying transaction-storage authorizations.

#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Decode, Encode};
use scale_info::TypeInfo;

/// Remaining storage capacity for an account.
///
/// All fields are zero when the account has no authorization, has fully
/// consumed it, or its authorization has expired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct AccountStorageAuthorization {
	/// Remaining number of store/renew transactions the account may submit.
	pub transactions: u32,
	/// Remaining bytes the account may store.
	pub bytes: u64,
}

sp_api::decl_runtime_apis! {
	/// Runtime API for querying transaction-storage authorizations.
	pub trait TransactionStorageAuthorizationApi<AccountId>
	where
		AccountId: codec::Codec,
	{
		/// Remaining storage authorization for `account`.
		fn account_authorization(account: AccountId) -> AccountStorageAuthorization;
	}
}
