//! Runtime API for querying transaction-storage authorizations.

#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Codec, Decode, Encode};
use scale_info::TypeInfo;

/// Summary of an account's storage authorization. Lets a client answer the
/// three questions an app actually has:
///
/// - **Is the authorization still valid?** — `expires_at` is the block at which it lapses; the API
///   returns `None` for accounts with no active authorization.
/// - **Will my next `store` call be high-priority?** — yes, iff `size <= priority_bytes` (and a
///   transaction slot is still available). Stores that exceed `priority_bytes` still validate, just
///   at base priority.
/// - **Can I renew?** — yes, iff `size <= renew_bytes`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct AccountAuthorization<BlockNumber> {
	/// Block at which this account's authorization expires.
	pub expires_at: BlockNumber,
	/// Bytes that fit inside the high-priority window for `store` calls
	/// (`bytes_allowance - bytes`).
	pub priority_bytes: u64,
	/// Bytes available for `renew` calls (`bytes_allowance - bytes_permanent`).
	pub renew_bytes: u64,
}

sp_api::decl_runtime_apis! {
	/// Runtime API for querying transaction-storage authorizations.
	pub trait TransactionStorageAuthorizationApi<AccountId, BlockNumber>
	where
		AccountId: Codec,
		BlockNumber: Codec,
	{
		/// Authorization summary for `account`, or `None` if the account has
		/// no active authorization.
		fn account_authorization(account: AccountId) -> Option<AccountAuthorization<BlockNumber>>;
	}
}
