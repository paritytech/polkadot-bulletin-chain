use crate::{
	cids::{CidCodec, HashingAlgorithm},
	ContentHash,
};
use codec::{Decode, Encode};
use scale_info::TypeInfo;

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct IndexedTransactionInfo {
	pub content_hash: ContentHash,
	pub size: u32,
	pub hashing: HashingAlgorithm,
	pub cid_codec: CidCodec,
}

sp_api::decl_runtime_apis! {
	/// Runtime API exposing indexed-transaction metadata for a block.
	pub trait IndexedTransactionsApi {
		/// Returns `None` if `Transactions[block]` storage entry doesn't exist.
		fn indexed_transactions(block: u32) -> Option<alloc::vec::Vec<IndexedTransactionInfo>>;
	}
}
