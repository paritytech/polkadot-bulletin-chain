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

//! Public traits exposed by `pallet-bulletin-transaction-storage`.
//!
//! These are the only seams used by sibling pallets (e.g. `pallet-storage-auto-renewal`)
//! to interact with transaction-storage. Cross-pallet logic must go through these
//! traits — never via direct `Pallet::<T>::...` calls.

use crate::{AuthorizationExtent, TransactionInfo};
use alloc::vec::Vec;
use bulletin_transaction_storage_primitives::ContentHash;
use polkadot_sdk_frame::prelude::{DispatchError, Weight};

/// Notification emitted by `pallet-bulletin-transaction-storage` whenever a stored
/// transaction is about to be dropped from `Transactions` storage in the current block
/// (because its retention period has elapsed).
///
/// Implementors may use this hook to take their own action — most notably,
/// `pallet-storage-auto-renewal` enqueues the transaction for renewal if a registration
/// exists for the content hash.
///
/// The default `()` impl is a no-op: transaction-storage compiles and runs standalone
/// when no consumer is wired in.
pub trait OnTransactionExpiring {
	/// Called for each `(content_hash, tx_info)` pair as the transactions for an
	/// expiring block are being dropped. Implementors must not panic; failures
	/// should be best-effort and silent (e.g. queue overflow).
	fn on_expiring(content_hash: ContentHash, tx_info: &TransactionInfo);

	/// Worst-case weight contribution for `n` calls to [`Self::on_expiring`] in a
	/// single block. The transaction-storage pallet adds this to its `on_initialize`
	/// weight so that consumer pallets (e.g. auto-renewal) can correctly account for
	/// their own storage operations.
	fn on_expiring_weight(_n: u32) -> Weight {
		Weight::zero()
	}
}

impl OnTransactionExpiring for () {
	fn on_expiring(_content_hash: ContentHash, _tx_info: &TransactionInfo) {}
}

/// Operations that `pallet-bulletin-transaction-storage` exposes to consumer pallets
/// which need to read its data or mutate it on behalf of users (e.g. perform a renewal
/// from an inherent in `pallet-storage-auto-renewal`).
///
/// This trait defines the boundary: implementors of [`OnTransactionExpiring`] should
/// drive their own state via this trait rather than reaching into transaction-storage's
/// pallet internals.
pub trait StorageRenewer<AccountId> {
	/// Look up the latest [`TransactionInfo`] for the most-recent transaction
	/// matching `content_hash`, or `None` if no such transaction is currently stored.
	fn transaction_info_for_content_hash(content_hash: ContentHash) -> Option<TransactionInfo>;

	/// Returns the (unused and unexpired) authorization extent for the given account.
	fn account_authorization_extent(who: &AccountId) -> AuthorizationExtent;

	/// Renew a batch of previously-stored transactions in a single `BlockTransactions`
	/// mutate. For each input, consumes the account's authorization (one transaction
	/// plus `size` bytes of permanent capacity) and, on success, appends the renewal
	/// to the current block.
	///
	/// Returns one outcome per input, in order:
	/// - `Ok(new_index)` on a successful renewal.
	/// - `Err(_)` if the account had insufficient authorization, the block is full, or the call is
	///   made out of context. State is left unchanged for that item.
	///
	/// Implementors must perform at most one read and one write of the underlying
	/// block-transactions queue across the whole batch (amortized O(n) for n items),
	/// rather than per-item O(n²) re-encoding.
	fn renew_batch(items: &[(AccountId, TransactionInfo)]) -> Vec<Result<u32, DispatchError>>;
}
