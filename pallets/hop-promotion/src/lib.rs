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

//! # HOP Promotion Pallet
//!
//! Promotes near-expiry HOP pool data to permanent chain storage via
//! `pallet-transaction-storage`. Uses general transactions with
//! `#[pallet::authorize]` — no signature, no fees, priority 0, and no
//! debit of the submitter's Bulletin allowance: promotion only lands in
//! blockspace that would otherwise be unused, so charging the user
//! would just leave that space empty for no benefit.
//!
//! The authorize closure verifies the user's submit-time signature and the
//! freshness of the submit timestamp, and refuses promotion for accounts
//! whose Bulletin authorization is missing or expired.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;
pub use weights::WeightInfo;

use sp_runtime::transaction_validity::InvalidTransaction;

#[cfg(feature = "runtime-benchmarks")]
pub mod benchmarking;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
pub mod weights;

/// Domain separator for V1 `hop_submit` signatures. Must remain byte-identical
/// to the constant in `sc-hop` (`substrate/client/hop/src/types.rs`).
pub const HOP_SUBMIT_CONTEXT: &[u8] = b"hop-submit-v1:";

/// Domain separator for V2 `hop_submit` signatures. V2 binds the chain's
/// genesis hash and a hash of the recipients list to prevent cross-chain
/// replay and rebinding of recipients. Must remain byte-identical to the
/// matching constant in `sc-hop`.
pub const HOP_SUBMIT_CONTEXT_V2: &[u8] = b"hop-submit-v2:";

/// `blake2_256` of the SCALE-encoded empty recipients list (`[0x00]`). V2
/// promotions carrying this hash are rejected: `sc-hop` refuses zero-recipient
/// submissions and the chain mirrors it. Pinned by a test.
pub const EMPTY_RECIPIENTS_HASH: [u8; 32] = [
	0x03, 0x17, 0x0a, 0x2e, 0x75, 0x97, 0xb7, 0xb7, 0xe3, 0xd8, 0x4c, 0x05, 0x39, 0x1d, 0x13, 0x9a,
	0x62, 0xb1, 0x57, 0xe7, 0x87, 0x86, 0xd8, 0xc0, 0x82, 0xf2, 0x9d, 0xcf, 0x4c, 0x11, 0x13, 0x14,
];

/// Promotion carries the hash of an empty recipients list. Kept clear of
/// `pallet-bulletin-transaction-storage`'s `Custom` codes (currently 0..=12),
/// which this pallet reuses for shared checks like `BAD_DATA_SIZE`.
pub const NO_RECIPIENTS: InvalidTransaction = InvalidTransaction::Custom(100);

/// Reconstructs the V1 signing payload that the user signed at submit time,
/// given the precomputed blake2_256 hash of the data.
///
/// The bytes must remain identical to the SDK-side construction in `sc-hop`,
/// otherwise valid promotions will be rejected on chain.
pub fn signing_payload(data_hash: &[u8; 32], submit_timestamp: u64) -> [u8; 32] {
	const CTX_LEN: usize = HOP_SUBMIT_CONTEXT.len();
	let mut buf = [0u8; CTX_LEN + 32 + 8];
	buf[..CTX_LEN].copy_from_slice(HOP_SUBMIT_CONTEXT);
	buf[CTX_LEN..CTX_LEN + 32].copy_from_slice(data_hash);
	buf[CTX_LEN + 32..].copy_from_slice(&submit_timestamp.to_le_bytes());
	sp_io::hashing::blake2_256(&buf)
}

/// Reconstructs the V2 signing payload. Extends V1 with the chain's genesis
/// hash and a hash of the recipients list.
///
/// Byte layout (pre-hash):
/// `HOP_SUBMIT_CONTEXT_V2 || data_hash || submit_timestamp.to_le_bytes()
///  || genesis_hash || recipients_hash`
///
/// `recipients_hash` is computed off-chain by `sc-hop` as the `blake2_256` of
/// the SCALE-encoded recipients list — a `Vec<MultiSigner>` in submission
/// order, NOT `sc-hop`'s internal recipient type. The chain treats it as
/// opaque. Must remain byte-identical to the SDK-side construction in
/// `sc-hop`.
pub fn signing_payload_v2(
	data_hash: &[u8; 32],
	submit_timestamp: u64,
	genesis_hash: &[u8; 32],
	recipients_hash: &[u8; 32],
) -> [u8; 32] {
	const CTX_LEN: usize = HOP_SUBMIT_CONTEXT_V2.len();
	let mut buf = [0u8; CTX_LEN + 32 + 8 + 32 + 32];
	let mut offset = 0;
	buf[offset..offset + CTX_LEN].copy_from_slice(HOP_SUBMIT_CONTEXT_V2);
	offset += CTX_LEN;
	buf[offset..offset + 32].copy_from_slice(data_hash);
	offset += 32;
	buf[offset..offset + 8].copy_from_slice(&submit_timestamp.to_le_bytes());
	offset += 8;
	buf[offset..offset + 32].copy_from_slice(genesis_hash);
	offset += 32;
	buf[offset..offset + 32].copy_from_slice(recipients_hash);
	sp_io::hashing::blake2_256(&buf)
}

#[frame_support::pallet]
pub mod pallet {
	use super::{signing_payload, signing_payload_v2, EMPTY_RECIPIENTS_HASH, NO_RECIPIENTS};
	use crate::WeightInfo;
	use alloc::vec::Vec;
	use bulletin_transaction_storage_primitives::{
		cids::{HashingAlgorithm, RAW_CODEC},
		ContentHash,
	};
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use pallet_bulletin_transaction_storage::{WeightInfo as _, BAD_DATA_SIZE};
	use sp_core::H256;
	use sp_runtime::{
		traits::{IdentifyAccount, Verify, Zero},
		AccountId32, MultiSignature, MultiSigner,
	};

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config:
		frame_system::Config<AccountId = AccountId32, Hash = H256>
		+ pallet_bulletin_transaction_storage::Config
		+ pallet_timestamp::Config<Moment = u64>
	{
		/// Maximum allowable skew (in milliseconds) between the user's
		/// submit timestamp and the on-chain time when validating a promotion.
		#[pallet::constant]
		type SubmitTimestampTolerance: Get<u64>;

		/// Weight information for this pallet.
		type WeightInfo: crate::WeightInfo;
	}

	impl<T: Config> Pallet<T> {
		/// Returns whether `who` may have a HOP blob promoted on their behalf.
		///
		/// Satisfied when the account has an unexpired authorization entry in
		/// `pallet-bulletin-transaction-storage`, even if its store/renew
		/// extent has been fully spent. The storage pallet keeps the entry
		/// around (with zero extent) until expiration so that promotion stays
		/// available for the rest of the auth window.
		pub fn can_account_promote(who: &T::AccountId, _data_len: u32) -> bool {
			pallet_bulletin_transaction_storage::Pallet::<T>::account_has_active_authorization(who)
		}

		/// Whether `content_hash` is currently stored on-chain — i.e. some
		/// retained transaction in `pallet-bulletin-transaction-storage`
		/// indexes it.
		///
		/// Used by HOP's maintenance task to confirm a previously submitted
		/// promotion extrinsic landed in a block. Delegates to
		/// `pallet-bulletin-transaction-storage::contains_transaction`,
		/// which answers in O(1) via the content-hash index.
		pub fn is_promoted_on_chain(content_hash: ContentHash) -> bool {
			pallet_bulletin_transaction_storage::Pallet::<T>::contains_transaction(content_hash)
		}

		/// Authorizes a [`Call::promote`] dispatch in the tx pool: validates the
		/// source, data size, block fullness, submit-timestamp freshness, account
		/// authorization, and the user's sr25519 signature over `(data, ts)`.
		// Signature must match the `Call::promote` variant (`Vec<u8>`), so the
		// reference is `&Vec<u8>` rather than `&[u8]`.
		#[allow(clippy::ptr_arg)]
		pub fn authorize_promote(
			source: TransactionSource,
			signer: &MultiSigner,
			signature: &MultiSignature,
			submit_timestamp: &u64,
			data: &Vec<u8>,
		) -> Result<(ValidTransaction, Weight), TransactionValidityError> {
			if matches!(source, TransactionSource::External) {
				return Err(InvalidTransaction::Call.into());
			}
			if !pallet_bulletin_transaction_storage::Pallet::<T>::data_size_ok(data.len()) {
				return Err(BAD_DATA_SIZE.into());
			}

			// Mirrors the early-out in pallet_bulletin_transaction_storage so we don't pay for
			// chunking + ordered-root hashing when the block is already at MaxBlockTransactions.
			if pallet_bulletin_transaction_storage::Pallet::<T>::block_transactions_full() {
				return Err(InvalidTransaction::ExhaustsResources.into());
			}

			// Reject signatures whose submit_timestamp is too far from the current block time.
			let now_ms = pallet_timestamp::Pallet::<T>::get();
			let skew = now_ms.abs_diff(*submit_timestamp);
			if skew > T::SubmitTimestampTolerance::get() {
				return Err(InvalidTransaction::Stale.into());
			}

			// Account-level authorization check before the expensive signature verify so
			// unauthorized accounts can't force sr25519 verifies on garbage signatures.
			let account_id = signer.clone().into_account();
			if !Self::can_account_promote(&account_id, data.len() as u32) {
				return Err(InvalidTransaction::BadSigner.into());
			}

			// Verify the user's signature over (data, submit_timestamp).
			let data_hash = sp_io::hashing::blake2_256(data);
			let payload = signing_payload(&data_hash, *submit_timestamp);
			if !signature.verify(&payload[..], &account_id) {
				return Err(InvalidTransaction::BadProof.into());
			}

			Ok((
				ValidTransaction::with_tag_prefix("HopPromotion")
					.priority(0)
					.longevity(5)
					.propagate(false)
					.and_provides(data_hash)
					.build()
					.expect("builder always succeeds; qed"),
				Weight::zero(),
			))
		}

		/// Authorizes a [`Call::promote_v2`] dispatch in the tx pool: identical
		/// pre-checks to [`Self::authorize_promote`], but verifies the user's
		/// signature against the V2 payload, which additionally binds the chain's
		/// genesis hash (cross-chain replay) and a hash of the recipients list
		/// (rebinding to a different audience).
		#[allow(clippy::ptr_arg)]
		pub fn authorize_promote_v2(
			source: TransactionSource,
			signer: &MultiSigner,
			signature: &MultiSignature,
			submit_timestamp: &u64,
			recipients_hash: &H256,
			data: &Vec<u8>,
		) -> Result<(ValidTransaction, Weight), TransactionValidityError> {
			if matches!(source, TransactionSource::External) {
				return Err(InvalidTransaction::Call.into());
			}
			if !pallet_bulletin_transaction_storage::Pallet::<T>::data_size_ok(data.len()) {
				return Err(BAD_DATA_SIZE.into());
			}

			if pallet_bulletin_transaction_storage::Pallet::<T>::block_transactions_full() {
				return Err(InvalidTransaction::ExhaustsResources.into());
			}

			let now_ms = pallet_timestamp::Pallet::<T>::get();
			let skew = now_ms.abs_diff(*submit_timestamp);
			if skew > T::SubmitTimestampTolerance::get() {
				return Err(InvalidTransaction::Stale.into());
			}

			// `sc-hop` rejects submissions with zero recipients (`NoRecipients`);
			// mirror that here by refusing the hash of the empty recipients list.
			if *recipients_hash.as_fixed_bytes() == EMPTY_RECIPIENTS_HASH {
				return Err(NO_RECIPIENTS.into());
			}

			let account_id = signer.clone().into_account();
			if !Self::can_account_promote(&account_id, data.len() as u32) {
				return Err(InvalidTransaction::BadSigner.into());
			}

			let data_hash = sp_io::hashing::blake2_256(data);
			let genesis_hash = frame_system::Pallet::<T>::block_hash(BlockNumberFor::<T>::zero());
			let payload = signing_payload_v2(
				&data_hash,
				*submit_timestamp,
				genesis_hash.as_fixed_bytes(),
				recipients_hash.as_fixed_bytes(),
			);
			if !signature.verify(&payload[..], &account_id) {
				return Err(InvalidTransaction::BadProof.into());
			}

			Ok((
				ValidTransaction::with_tag_prefix("HopPromotion")
					.priority(0)
					.longevity(5)
					.propagate(false)
					.and_provides(data_hash)
					.build()
					.expect("builder always succeeds; qed"),
				Weight::zero(),
			))
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::call_index(0)]
		#[pallet::weight(
			<T as pallet_bulletin_transaction_storage::Config>::WeightInfo::store(data.len() as u32)
		)]
		#[pallet::authorize(Pallet::<T>::authorize_promote)]
		#[pallet::weight_of_authorize(<T as Config>::WeightInfo::authorize_promote(data.len() as u32))]
		// `signer`/`signature`/`submit_timestamp` are validated by `authorize_promote`
		// above; the dispatch body trusts them and only runs after authorization.
		//
		// `data` MUST be the last argument — see the FOOTGUN note on
		// `pallet_bulletin_transaction_storage::Pallet::do_store`: the trailing
		// `data.len()` bytes of the encoded extrinsic get indexed, so any field
		// encoded after `data` corrupts the stored blob.
		pub fn promote(
			origin: OriginFor<T>,
			_signer: MultiSigner,
			_signature: MultiSignature,
			_submit_timestamp: u64,
			data: Vec<u8>,
		) -> DispatchResult {
			ensure_authorized(origin)?;
			pallet_bulletin_transaction_storage::Pallet::<T>::do_store(
				data,
				HashingAlgorithm::Blake2b256,
				RAW_CODEC,
			)
		}

		/// V2 variant of [`Self::promote`]: identical body, but the authorize hook
		/// requires the user's signature to additionally cover the chain genesis
		/// hash and a hash of the recipients list (see [`signing_payload_v2`]).
		#[pallet::call_index(1)]
		#[pallet::weight(
			<T as pallet_bulletin_transaction_storage::Config>::WeightInfo::store(data.len() as u32)
		)]
		#[pallet::authorize(Pallet::<T>::authorize_promote_v2)]
		#[pallet::weight_of_authorize(
			<T as Config>::WeightInfo::authorize_promote_v2(data.len() as u32)
		)]
		// `data` MUST be the last argument — see the FOOTGUN note on
		// `pallet_bulletin_transaction_storage::Pallet::do_store`: the trailing
		// `data.len()` bytes of the encoded extrinsic get indexed, so any field
		// encoded after `data` corrupts the stored blob.
		pub fn promote_v2(
			origin: OriginFor<T>,
			_signer: MultiSigner,
			_signature: MultiSignature,
			_submit_timestamp: u64,
			_recipients_hash: H256,
			data: Vec<u8>,
		) -> DispatchResult {
			ensure_authorized(origin)?;
			pallet_bulletin_transaction_storage::Pallet::<T>::do_store(
				data,
				HashingAlgorithm::Blake2b256,
				RAW_CODEC,
			)
		}
	}
}
