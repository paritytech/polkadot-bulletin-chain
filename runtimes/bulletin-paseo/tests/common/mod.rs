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

#![cfg(test)]
#![allow(dead_code)]

use bulletin_paseo_runtime::{
	Runtime, RuntimeCall, System, TransactionStorage, TxExtension, UncheckedExtrinsic,
};
use frame_support::{assert_ok, dispatch::GetDispatchInfo, pallet_prelude::Hooks};
use parachains_common::{AccountId, Hash as PcHash, Signature as PcSignature};
use sp_core::{Encode, Pair};
use sp_runtime::{transaction_validity, ApplyExtrinsicResult};

pub const ALICE: [u8; 32] = [1u8; 32];

/// Advance to the next block for testing transaction storage.
pub fn advance_block() {
	let current = frame_system::Pallet::<Runtime>::block_number();

	<TransactionStorage as Hooks<_>>::on_finalize(current);
	<System as Hooks<_>>::on_finalize(current);

	let next = current + 1;
	System::set_block_number(next);

	frame_system::BlockWeight::<Runtime>::kill();
	frame_system::BlockSize::<Runtime>::kill();

	<System as Hooks<_>>::on_initialize(next);
	<TransactionStorage as Hooks<_>>::on_initialize(next);
}

pub fn construct_extrinsic(
	sender: Option<sp_core::sr25519::Pair>,
	call: RuntimeCall,
) -> Result<UncheckedExtrinsic, transaction_validity::TransactionValidityError> {
	// provide a known block hash for the immortal era check
	frame_system::BlockHash::<Runtime>::insert(0, PcHash::default());
	let inner = (
		frame_system::AuthorizeCall::<Runtime>::new(),
		frame_system::CheckNonZeroSender::<Runtime>::new(),
		frame_system::CheckSpecVersion::<Runtime>::new(),
		frame_system::CheckTxVersion::<Runtime>::new(),
		frame_system::CheckGenesis::<Runtime>::new(),
		frame_system::CheckEra::<Runtime>::from(sp_runtime::generic::Era::immortal()),
		frame_system::CheckNonce::<Runtime>::from(if let Some(s) = sender.as_ref() {
			let account_id = AccountId::from(s.public());
			frame_system::Pallet::<Runtime>::account(&account_id).nonce
		} else {
			0
		}),
		frame_system::CheckWeight::<Runtime>::new(),
		pallet_skip_feeless_payment::SkipCheckIfFeeless::from(
			pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(0u128),
		),
		pallet_bulletin_transaction_storage::extension::ValidateStorageCalls::<
			Runtime,
			bulletin_paseo_runtime::storage::StorageCallInspector,
		>::default(),
		pallet_bulletin_transaction_storage::extension::AllowanceBasedPriority::<Runtime>::default(
		),
		frame_metadata_hash_extension::CheckMetadataHash::<Runtime>::new(false),
	);
	let tx_ext: TxExtension =
		cumulus_pallet_weight_reclaim::StorageWeightReclaim::<Runtime, _>::from(inner);

	if let Some(s) = sender.as_ref() {
		// Signed call.
		let account_id = AccountId::from(s.public());
		let payload = sp_runtime::generic::SignedPayload::new(call.clone(), tx_ext.clone())?;
		let signature = payload.using_encoded(|e| s.sign(e));
		Ok(UncheckedExtrinsic::new_signed(
			call,
			account_id.into(),
			PcSignature::Sr25519(signature),
			tx_ext,
		))
	} else {
		// Unsigned call.
		Ok(UncheckedExtrinsic::new_transaction(call, tx_ext))
	}
}

pub fn construct_and_apply_extrinsic(
	account: Option<sp_core::sr25519::Pair>,
	call: RuntimeCall,
) -> ApplyExtrinsicResult {
	let dispatch_info = call.get_dispatch_info();
	let xt = construct_extrinsic(account, call)?;
	let xt_len = xt.encode().len();
	tracing::info!(
		"Applying extrinsic: class={:?} pays_fee={:?} weight={:?} encoded_len={} bytes",
		dispatch_info.class,
		dispatch_info.pays_fee,
		dispatch_info.total_weight(),
		xt_len
	);
	bulletin_paseo_runtime::Executive::apply_extrinsic(xt)
}

pub fn assert_extrinsic_ok(apply_result: ApplyExtrinsicResult) {
	assert_ok!(apply_result);
	assert_ok!(apply_result.unwrap());
}
