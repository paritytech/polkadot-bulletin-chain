// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

//! Wiring of `pallet-validator-collators` with Asset Hub Westend.

use codec::{Decode, Encode};
use frame_support::traits::{Get, GetCallMetadata};
use sp_runtime::AccountId32;

#[test]
fn report_call_decodes_on_asset_hub() {
	// GIVEN the largest call Bulletin sends to report blocks
	type MaxEntries =
		<bulletin_westend_runtime::Runtime as pallet_validator_collators::Config>::MaxValidators;
	let blocks = (0..MaxEntries::get())
		.map(|i| {
			let mut account = [0u8; 32];
			account[..4].copy_from_slice(&i.to_le_bytes());
			(AccountId32::from(account), i + 1)
		})
		.collect::<Vec<_>>();
	let encoded = bulletin_westend_runtime::ReportBlocksToAssetHub::encoded_call(&blocks);
	// WHEN Asset Hub decodes it
	let call = asset_hub_westend_runtime::RuntimeCall::decode(&mut &encoded[..])
		.expect("Bulletin's report decodes on Asset Hub");
	// THEN it is `report_authored_blocks` of `StakingPointsConverter` with the same entries
	let metadata = call.get_call_metadata();
	assert_eq!(
		(metadata.pallet_name, metadata.function_name),
		("StakingPointsConverter", "report_authored_blocks")
	);
	assert_eq!(call.encode(), encoded);
}
