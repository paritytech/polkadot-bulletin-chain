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

use bulletin_westend_runtime::{
	xcm_config::{GovernanceLocation, LocationToAccountId},
	AllPalletsWithoutSystem, Block, Runtime, RuntimeCall, RuntimeEvent, RuntimeOrigin, SessionKeys,
	System, TxExtension, UncheckedExtrinsic,
};
use frame_support::{
	assert_err, assert_ok, dispatch::GetDispatchInfo, traits::fungible::Mutate as FungibleMutate,
};
use parachains_common::{AccountId, AuraId, Balance, Hash as PcHash, Signature as PcSignature};
use parachains_runtimes_test_utils::{ExtBuilder, GovernanceOrigin, RuntimeHelper};
use sp_core::{crypto::Ss58Codec, Encode, Pair};
use sp_keyring::Sr25519Keyring;
use sp_runtime::{transaction_validity::InvalidTransaction, ApplyExtrinsicResult, Either};
use testnet_parachains_constants::westend::{fee::WeightToFee, locations::PeopleLocation};
use xcm::latest::prelude::*;
use xcm_runtime_apis::conversions::LocationToAccountHelper;

const ALICE: [u8; 32] = [1u8; 32];

fn construct_extrinsic(
	sender: sp_core::sr25519::Pair,
	call: RuntimeCall,
) -> Result<UncheckedExtrinsic, sp_runtime::transaction_validity::TransactionValidityError> {
	let account_id = parachains_common::AccountId::from(sender.public());
	// provide a known block hash for the immortal era check
	frame_system::BlockHash::<Runtime>::insert(0, PcHash::default());
	let inner = (
		frame_system::AuthorizeCall::<Runtime>::new(),
		frame_system::CheckNonZeroSender::<Runtime>::new(),
		frame_system::CheckSpecVersion::<Runtime>::new(),
		frame_system::CheckTxVersion::<Runtime>::new(),
		frame_system::CheckGenesis::<Runtime>::new(),
		frame_system::CheckEra::<Runtime>::from(sp_runtime::generic::Era::immortal()),
		frame_system::CheckNonce::<Runtime>::from(
			frame_system::Pallet::<Runtime>::account(&account_id).nonce,
		),
		frame_system::CheckWeight::<Runtime>::new(),
		pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(0u128),
		bulletin_westend_runtime::ValidateSigned,
		frame_metadata_hash_extension::CheckMetadataHash::<Runtime>::new(false),
	);
	let tx_ext: TxExtension =
		cumulus_pallet_weight_reclaim::StorageWeightReclaim::<Runtime, _>::from(inner);
	let payload = sp_runtime::generic::SignedPayload::new(call.clone(), tx_ext.clone())?;
	let signature = payload.using_encoded(|e| sender.sign(e));
	Ok(UncheckedExtrinsic::new_signed(
		call,
		account_id.into(),
		PcSignature::Sr25519(signature),
		tx_ext,
	))
}

fn construct_and_apply_extrinsic(
	account: sp_core::sr25519::Pair,
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
	bulletin_westend_runtime::Executive::apply_extrinsic(xt)
}

#[test]
fn transaction_storage_runtime_sizes() {
	use bulletin_westend_runtime as runtime;
	use bulletin_westend_runtime::BuildStorage;
	use frame_support::assert_ok;
	use pallet_transaction_storage::{
		AuthorizationExtent, Call as TxStorageCall, Config as TxStorageConfig,
	};
	use sp_keyring::Sr25519Keyring;

	sp_io::TestExternalities::new(
		runtime::RuntimeGenesisConfig::default().build_storage().unwrap(),
	)
	.execute_with(|| {
		// prepare data
		let account = Sr25519Keyring::Alice;
		let who: AccountId = account.to_account_id();
		#[allow(clippy::identity_op)]
		let sizes: [usize; 5] = [
			2000,                  // 2 KB
			256 * 1024,            // 256 KB
			512 * 1024,            // 512 KB
			1 * 1024 * 1024,       // 1 MB
			(3 * 1024 * 1024) / 2, // 1.5 MB
		];
		let total_bytes: u64 = sizes.iter().map(|s| *s as u64).sum();

		// fund Alice to cover length-based tx fees
		let initial: Balance = 10_000_000_000_000_000_000u128;
		<pallet_balances::Pallet<Runtime> as FungibleMutate<_>>::set_balance(&who, initial);

		// authorize
		assert_ok!(runtime::TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who.clone(),
			sizes.len() as u32,
			total_bytes,
		));
		assert_eq!(
			runtime::TransactionStorage::account_authorization_extent(who.clone()),
			AuthorizationExtent { transactions: sizes.len() as u32, bytes: total_bytes },
		);

		// store data via signed extrinsics (ValidateSigned consumes authorization)
		for (index, size) in sizes.into_iter().enumerate() {
			tracing::info!("Storing data with size: {size} and index: {index}");
			let res = construct_and_apply_extrinsic(
				account.pair(),
				RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
					data: vec![0u8; size],
				}),
			);
			assert_ok!(res);
			assert_ok!(res.unwrap());
		}
		assert_eq!(
			runtime::TransactionStorage::account_authorization_extent(who.clone()),
			AuthorizationExtent { transactions: 0, bytes: 0 },
		);

		// (MaxTransactionSize+1) should exceed MaxTransactionSize and fail
		let oversized: u64 =
			(<<Runtime as TxStorageConfig>::MaxTransactionSize as frame_support::traits::Get<
				u32,
			>>::get() + 1)
				.into();
		assert_ok!(runtime::TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			who.clone(),
			1,
			oversized,
		));
		assert_eq!(
			runtime::TransactionStorage::account_authorization_extent(who.clone()),
			AuthorizationExtent { transactions: 1_u32, bytes: oversized },
		);
		let res = construct_and_apply_extrinsic(
			account.pair(),
			RuntimeCall::TransactionStorage(TxStorageCall::<Runtime>::store {
				data: vec![0u8; oversized as usize],
			}),
		);
		// On Westend, very large extrinsics may be rejected earlier for exhausting resources
		// (block length/weight) before reaching the pallet's BAD_DATA_SIZE check.
		assert!(
			res == Err(pallet_transaction_storage::BAD_DATA_SIZE.into()) ||
				res == Err(InvalidTransaction::ExhaustsResources.into()),
			"unexpected error: {res:?}"
		);
	});
}

#[test]
fn location_conversion_works() {
	// the purpose of hardcoded values is to catch an unintended location conversion logic change.
	struct TestCase {
		description: &'static str,
		location: Location,
		expected_account_id_str: &'static str,
	}

	let test_cases = vec![
		// DescribeTerminus
		TestCase {
			description: "DescribeTerminus Parent",
			location: Location::new(1, Here),
			expected_account_id_str: "5Dt6dpkWPwLaH4BBCKJwjiWrFVAGyYk3tLUabvyn4v7KtESG",
		},
		TestCase {
			description: "DescribeTerminus Sibling",
			location: Location::new(1, [Parachain(1111)]),
			expected_account_id_str: "5Eg2fnssmmJnF3z1iZ1NouAuzciDaaDQH7qURAy3w15jULDk",
		},
		// DescribePalletTerminal
		TestCase {
			description: "DescribePalletTerminal Parent",
			location: Location::new(1, [PalletInstance(50)]),
			expected_account_id_str: "5CnwemvaAXkWFVwibiCvf2EjqwiqBi29S5cLLydZLEaEw6jZ",
		},
		TestCase {
			description: "DescribePalletTerminal Sibling",
			location: Location::new(1, [Parachain(1111), PalletInstance(50)]),
			expected_account_id_str: "5GFBgPjpEQPdaxEnFirUoa51u5erVx84twYxJVuBRAT2UP2g",
		},
		// DescribeAccountId32Terminal
		TestCase {
			description: "DescribeAccountId32Terminal Parent",
			location: Location::new(
				1,
				[Junction::AccountId32 { network: None, id: AccountId::from(ALICE).into() }],
			),
			expected_account_id_str: "5DN5SGsuUG7PAqFL47J9meViwdnk9AdeSWKFkcHC45hEzVz4",
		},
		TestCase {
			description: "DescribeAccountId32Terminal Sibling",
			location: Location::new(
				1,
				[
					Parachain(1111),
					Junction::AccountId32 { network: None, id: AccountId::from(ALICE).into() },
				],
			),
			expected_account_id_str: "5DGRXLYwWGce7wvm14vX1Ms4Vf118FSWQbJkyQigY2pfm6bg",
		},
		// DescribeAccountKey20Terminal
		TestCase {
			description: "DescribeAccountKey20Terminal Parent",
			location: Location::new(1, [AccountKey20 { network: None, key: [0u8; 20] }]),
			expected_account_id_str: "5F5Ec11567pa919wJkX6VHtv2ZXS5W698YCW35EdEbrg14cg",
		},
		TestCase {
			description: "DescribeAccountKey20Terminal Sibling",
			location: Location::new(
				1,
				[Parachain(1111), AccountKey20 { network: None, key: [0u8; 20] }],
			),
			expected_account_id_str: "5CB2FbUds2qvcJNhDiTbRZwiS3trAy6ydFGMSVutmYijpPAg",
		},
		// DescribeTreasuryVoiceTerminal
		TestCase {
			description: "DescribeTreasuryVoiceTerminal Parent",
			location: Location::new(1, [Plurality { id: BodyId::Treasury, part: BodyPart::Voice }]),
			expected_account_id_str: "5CUjnE2vgcUCuhxPwFoQ5r7p1DkhujgvMNDHaF2bLqRp4D5F",
		},
		TestCase {
			description: "DescribeTreasuryVoiceTerminal Sibling",
			location: Location::new(
				1,
				[Parachain(1111), Plurality { id: BodyId::Treasury, part: BodyPart::Voice }],
			),
			expected_account_id_str: "5G6TDwaVgbWmhqRUKjBhRRnH4ry9L9cjRymUEmiRsLbSE4gB",
		},
		// DescribeBodyTerminal
		TestCase {
			description: "DescribeBodyTerminal Parent",
			location: Location::new(1, [Plurality { id: BodyId::Unit, part: BodyPart::Voice }]),
			expected_account_id_str: "5EBRMTBkDisEXsaN283SRbzx9Xf2PXwUxxFCJohSGo4jYe6B",
		},
		TestCase {
			description: "DescribeBodyTerminal Sibling",
			location: Location::new(
				1,
				[Parachain(1111), Plurality { id: BodyId::Unit, part: BodyPart::Voice }],
			),
			expected_account_id_str: "5DBoExvojy8tYnHgLL97phNH975CyT45PWTZEeGoBZfAyRMH",
		},
	];

	for tc in test_cases {
		let expected =
			AccountId::from_string(tc.expected_account_id_str).expect("Invalid AccountId string");

		let got = LocationToAccountHelper::<AccountId, LocationToAccountId>::convert_location(
			tc.location.into(),
		)
		.unwrap();

		assert_eq!(got, expected, "{}", tc.description);
	}
}

#[test]
fn xcm_payment_api_works() {
	parachains_runtimes_test_utils::test_cases::xcm_payment_api_with_native_token_works::<
		Runtime,
		RuntimeCall,
		RuntimeOrigin,
		Block,
		WeightToFee,
	>();
}

#[test]
fn governance_authorize_upgrade_works() {
	use westend_runtime_constants::system_parachain::{ASSET_HUB_ID, COLLECTIVES_ID};

	// no - random para
	assert_err!(
		parachains_runtimes_test_utils::test_cases::can_governance_authorize_upgrade::<
			Runtime,
			RuntimeOrigin,
		>(GovernanceOrigin::Location(Location::new(1, Parachain(12334)))),
		Either::Right(InstructionError { index: 0, error: XcmError::Barrier })
	);
	// ok - AssetHub
	assert_ok!(parachains_runtimes_test_utils::test_cases::can_governance_authorize_upgrade::<
		Runtime,
		RuntimeOrigin,
	>(GovernanceOrigin::Location(Location::new(1, Parachain(ASSET_HUB_ID)))));
	// no - Collectives
	assert_err!(
		parachains_runtimes_test_utils::test_cases::can_governance_authorize_upgrade::<
			Runtime,
			RuntimeOrigin,
		>(GovernanceOrigin::Location(Location::new(1, Parachain(COLLECTIVES_ID)))),
		Either::Right(InstructionError { index: 0, error: XcmError::Barrier })
	);
	// no - Collectives Voice of Fellows plurality
	assert_err!(
		parachains_runtimes_test_utils::test_cases::can_governance_authorize_upgrade::<
			Runtime,
			RuntimeOrigin,
		>(GovernanceOrigin::LocationAndDescendOrigin(
			Location::new(1, Parachain(COLLECTIVES_ID)),
			Plurality { id: BodyId::Technical, part: BodyPart::Voice }.into()
		)),
		Either::Right(InstructionError { index: 2, error: XcmError::BadOrigin })
	);

	// ok - relaychain
	assert_ok!(parachains_runtimes_test_utils::test_cases::can_governance_authorize_upgrade::<
		Runtime,
		RuntimeOrigin,
	>(GovernanceOrigin::Location(Location::parent())));

	// ok - governance location
	assert_ok!(parachains_runtimes_test_utils::test_cases::can_governance_authorize_upgrade::<
		Runtime,
		RuntimeOrigin,
	>(GovernanceOrigin::Location(GovernanceLocation::get())));
}

#[test]
fn people_chain_can_authorize_storage_with_transact() {
	// Prepare call.
	let account = Sr25519Keyring::Ferdie;
	let authorize_call = RuntimeCall::TransactionStorage(pallet_transaction_storage::Call::<
		Runtime,
	>::authorize_account {
		who: account.to_account_id(),
		transactions: 16,
		bytes: 1024,
	});

	// Execute XCM as People chain origin would do with `Transact -> Origin::Xcm`.
	ExtBuilder::<Runtime>::default()
		.with_collators(vec![AccountId::from(ALICE)])
		.with_session_keys(vec![(
			AccountId::from(ALICE),
			AccountId::from(ALICE),
			SessionKeys { aura: AuraId::from(sp_core::sr25519::Public::from_raw(ALICE)) },
		)])
		.with_tracing()
		.build()
		.execute_with(|| {
			assert_ok!(RuntimeHelper::<Runtime, AllPalletsWithoutSystem>::execute_as_origin(
				(PeopleLocation::get(), OriginKind::Xcm),
				authorize_call,
				None
			)
			.ensure_complete());

			// Check event.
			System::assert_has_event(RuntimeEvent::TransactionStorage(
				pallet_transaction_storage::Event::AccountAuthorized {
					who: account.to_account_id(),
					transactions: 16,
					bytes: 1024,
				},
			));
		})
}
