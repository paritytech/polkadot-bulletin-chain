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

//! Reserve transfer tests for WND between Asset Hub Westend and Bulletin Westend.
//!
//! These tests demonstrate:
//! 1. WND reserve transfers from Asset Hub to Bulletin (Asset Hub is reserve - LocalReserve)
//! 2. WND reserve transfers from Bulletin back to Asset Hub (DestinationReserve)

use crate::{
	AssetHubWestendParaReceiver, AssetHubWestendParaSender, BulletinWestend,
	BulletinWestendParaReceiver, BulletinWestendParaSender, WestendMockNet, BULLETIN_PARA_ID,
};
use asset_hub_westend_emulated_chain::AssetHubWestend;
use frame_support::{
	assert_ok,
	traits::fungible::{Inspect, Mutate},
};
use parachains_common::Balance;
use xcm::{latest::prelude::*, VersionedXcm};
use xcm_emulator::{Chain, Network, Parachain, TestExt};
use xcm_executor::traits::TransferType;

/// Asset Hub Westend parachain ID.
const ASSET_HUB_PARA_ID: u32 = 1000;

/// Amount to transfer in tests.
const TRANSFER_AMOUNT: Balance = 1_000_000_000_000; // 1 WND

/// Amount to use for fees.
const FEE_AMOUNT: Balance = 500_000_000_000; // 0.5 WND

/// Type alias for AssetHubWestend with our network.
type AssetHubWestendNet = AssetHubWestend<WestendMockNet>;

/// Type alias for BulletinWestend with our network.
type BulletinWestendNet = BulletinWestend<WestendMockNet>;

/// Test reserve transfer of WND from Asset Hub Westend to Bulletin Westend.
///
/// This test verifies that:
/// 1. Alice on Asset Hub can initiate a reserve transfer to Bulletin
/// 2. The transfer reduces Alice's balance on Asset Hub
/// 3. Bob's balance increases on Bulletin (receiving the transferred WND)
///
/// Since Asset Hub is the reserve for WND, we use LocalReserve transfer type.
#[test]
fn reserve_transfer_wnd_from_asset_hub_to_bulletin() {
	WestendMockNet::reset();

	let sender = AssetHubWestendParaSender::get();
	let receiver = BulletinWestendParaReceiver::get();

	let sender_initial_on_asset_hub = AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		<Balances as Inspect<_>>::balance(&sender)
	});

	let receiver_initial_on_bulletin = BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;
		<Balances as Inspect<_>>::balance(&receiver)
	});

	assert!(
		sender_initial_on_asset_hub >= TRANSFER_AMOUNT + FEE_AMOUNT,
		"Sender needs sufficient balance on Asset Hub"
	);

	let dest = Location::new(1, [Parachain(BULLETIN_PARA_ID)]);
	let beneficiary =
		Location::new(0, [AccountId32 { network: None, id: receiver.clone().into() }]);
	let wnd_location = Location::parent();
	let assets: Assets = (wnd_location.clone(), TRANSFER_AMOUNT).into();
	let fee_asset_id: AssetId = wnd_location.into();
	let xcm_on_dest = Xcm::<()>(vec![DepositAsset { assets: Wild(AllCounted(1)), beneficiary }]);

	AssetHubWestendNet::execute_with(|| {
		type PolkadotXcm = asset_hub_westend_runtime::PolkadotXcm;
		type RuntimeOrigin = <AssetHubWestendNet as Chain>::RuntimeOrigin;

		assert_ok!(PolkadotXcm::transfer_assets_using_type_and_then(
			RuntimeOrigin::signed(sender.clone()),
			Box::new(dest.into()),
			Box::new(assets.into()),
			Box::new(TransferType::LocalReserve),
			Box::new(fee_asset_id.into()),
			Box::new(TransferType::LocalReserve),
			Box::new(VersionedXcm::from(xcm_on_dest)),
			WeightLimit::Unlimited,
		));
	});

	AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		let sender_balance = <Balances as Inspect<_>>::balance(&sender);
		assert!(
			sender_balance < sender_initial_on_asset_hub,
			"Sender's balance should decrease after transfer"
		);
		assert!(
			sender_initial_on_asset_hub - sender_balance >= TRANSFER_AMOUNT,
			"Sender should have transferred at least {} WND",
			TRANSFER_AMOUNT
		);
	});

	BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;
		let receiver_balance = <Balances as Inspect<_>>::balance(&receiver);
		assert!(
			receiver_balance > receiver_initial_on_bulletin,
			"Receiver's balance should increase after receiving transfer. Initial: {}, Current: {}",
			receiver_initial_on_bulletin,
			receiver_balance
		);
	});
}

/// Test reserve transfer of WND from Bulletin Westend to Asset Hub Westend.
///
/// This test verifies that:
/// 1. Alice on Bulletin can initiate a reserve transfer back to Asset Hub
/// 2. The transfer reduces Alice's balance on Bulletin
/// 3. Bob's balance increases on Asset Hub
///
/// Since Asset Hub is the reserve for WND, we use DestinationReserve transfer type.
#[test]
fn reserve_transfer_wnd_from_bulletin_to_asset_hub() {
	WestendMockNet::reset();

	// Fund Bulletin's sovereign account on Asset Hub. When Bulletin sends a DestinationReserve
	// transfer, Asset Hub will withdraw from Bulletin's sovereign account.
	AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		let bulletin_location = Location::new(1, [Parachain(BULLETIN_PARA_ID)]);
		let sovereign_account =
			<AssetHubWestendNet as Parachain>::sovereign_account_id_of(bulletin_location);
		<Balances as Mutate<_>>::mint_into(&sovereign_account, TRANSFER_AMOUNT + FEE_AMOUNT)
			.unwrap();
	});

	let sender = BulletinWestendParaSender::get();
	let receiver = AssetHubWestendParaReceiver::get();

	let sender_initial_on_bulletin = BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;
		<Balances as Inspect<_>>::balance(&sender)
	});

	let receiver_initial_on_asset_hub = AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		<Balances as Inspect<_>>::balance(&receiver)
	});

	assert!(
		sender_initial_on_bulletin >= TRANSFER_AMOUNT + FEE_AMOUNT,
		"Sender needs sufficient balance on Bulletin"
	);

	let dest = Location::new(1, [Parachain(ASSET_HUB_PARA_ID)]);
	let beneficiary =
		Location::new(0, [AccountId32 { network: None, id: receiver.clone().into() }]);
	let wnd_location = Location::parent();
	let assets: Assets = (wnd_location.clone(), TRANSFER_AMOUNT).into();
	let fee_asset_id: AssetId = wnd_location.into();
	let xcm_on_dest = Xcm::<()>(vec![DepositAsset { assets: Wild(AllCounted(1)), beneficiary }]);

	BulletinWestendNet::execute_with(|| {
		type PolkadotXcm = bulletin_westend_runtime::PolkadotXcm;
		type RuntimeOrigin = <BulletinWestendNet as Chain>::RuntimeOrigin;

		assert_ok!(PolkadotXcm::transfer_assets_using_type_and_then(
			RuntimeOrigin::signed(sender.clone()),
			Box::new(dest.into()),
			Box::new(assets.into()),
			Box::new(TransferType::DestinationReserve),
			Box::new(fee_asset_id.into()),
			Box::new(TransferType::DestinationReserve),
			Box::new(VersionedXcm::from(xcm_on_dest)),
			WeightLimit::Unlimited,
		));
	});

	BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;
		let sender_balance = <Balances as Inspect<_>>::balance(&sender);
		assert!(
			sender_balance < sender_initial_on_bulletin,
			"Sender's balance should decrease after transfer"
		);
		assert!(
			sender_initial_on_bulletin - sender_balance >= TRANSFER_AMOUNT,
			"Sender should have transferred at least {} WND",
			TRANSFER_AMOUNT
		);
	});

	AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		let receiver_balance = <Balances as Inspect<_>>::balance(&receiver);
		assert!(
			receiver_balance > receiver_initial_on_asset_hub,
			"Receiver's balance should increase after receiving transfer. Initial: {}, Current: {}",
			receiver_initial_on_asset_hub,
			receiver_balance
		);
	});
}
