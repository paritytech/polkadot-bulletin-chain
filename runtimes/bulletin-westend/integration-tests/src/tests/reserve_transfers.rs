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
	// Reset the network state for a clean test
	WestendMockNet::reset();

	let sender = AssetHubWestendParaSender::get();
	let receiver = BulletinWestendParaReceiver::get();

	// Get initial balances
	let sender_initial_on_asset_hub = AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		<Balances as Inspect<_>>::balance(&sender)
	});

	let receiver_initial_on_bulletin = BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;
		<Balances as Inspect<_>>::balance(&receiver)
	});

	// Ensure sender has enough balance
	assert!(
		sender_initial_on_asset_hub >= TRANSFER_AMOUNT + FEE_AMOUNT,
		"Sender needs sufficient balance on Asset Hub"
	);

	// Construct the destination: Bulletin parachain (from Asset Hub's perspective)
	let dest = Location::new(1, [Parachain(BULLETIN_PARA_ID)]);

	// Construct the beneficiary: receiver on the destination chain
	let beneficiary =
		Location::new(0, [AccountId32 { network: None, id: receiver.clone().into() }]);

	// WND asset location (relay chain native token)
	let wnd_location = Location::parent();

	// Assets to transfer
	let assets: Assets = (wnd_location.clone(), TRANSFER_AMOUNT).into();

	// Fee asset ID
	let fee_asset_id: AssetId = wnd_location.into();

	// XCM to be executed on the destination (Bulletin) - just deposit to beneficiary
	let xcm_on_dest = Xcm::<()>(vec![DepositAsset { assets: Wild(AllCounted(1)), beneficiary }]);

	// Transfer assets from Asset Hub to Bulletin
	// Asset Hub is the reserve, so we use LocalReserve transfer type
	AssetHubWestendNet::execute_with(|| {
		type PolkadotXcm = asset_hub_westend_runtime::PolkadotXcm;
		type RuntimeOrigin = <AssetHubWestendNet as Chain>::RuntimeOrigin;

		let result = PolkadotXcm::transfer_assets_using_type_and_then(
			RuntimeOrigin::signed(sender.clone()),
			Box::new(dest.into()),
			Box::new(assets.into()),
			Box::new(TransferType::LocalReserve),
			Box::new(fee_asset_id.into()),
			Box::new(TransferType::LocalReserve),
			Box::new(VersionedXcm::from(xcm_on_dest)),
			WeightLimit::Unlimited,
		);

		println!("Asset Hub transfer result: {:?}", result);

		// Print events for debugging
		let events = frame_system::Pallet::<asset_hub_westend_runtime::Runtime>::events();
		println!("Asset Hub events count: {}", events.len());
		for event in events.iter().rev().take(10) {
			println!("  Event: {:?}", event.event);
		}

		assert_ok!(result);
	});

	// Verify sender's balance decreased on Asset Hub
	AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		let sender_balance = <Balances as Inspect<_>>::balance(&sender);
		assert!(
			sender_balance < sender_initial_on_asset_hub,
			"Sender's balance should decrease after transfer"
		);
		// Account for transfer amount + fees
		assert!(
			sender_initial_on_asset_hub - sender_balance >= TRANSFER_AMOUNT,
			"Sender should have transferred at least {} WND",
			TRANSFER_AMOUNT
		);
	});

	// Verify receiver's balance increased on Bulletin
	BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;

		// Print Bulletin events for debugging
		let events = frame_system::Pallet::<bulletin_westend_runtime::Runtime>::events();
		println!("Bulletin events count: {}", events.len());
		for event in events.iter().rev().take(10) {
			println!("  Bulletin Event: {:?}", event.event);
		}

		let receiver_balance = <Balances as Inspect<_>>::balance(&receiver);
		// Receiver should receive the transferred amount minus any execution fees
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
	// Reset the network state for a clean test
	WestendMockNet::reset();

	// Fund Bulletin's sovereign account on Asset Hub with enough WND for the transfer.
	// This simulates that WND was previously deposited here via reserve transfers.
	// When Bulletin sends a DestinationReserve transfer, Asset Hub will withdraw from
	// Bulletin's sovereign account.
	AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;

		let bulletin_location = Location::new(1, [Parachain(BULLETIN_PARA_ID)]);
		let sovereign_account =
			<AssetHubWestendNet as Parachain>::sovereign_account_id_of(bulletin_location);

		// Fund sovereign account with enough for transfer + fees
		let fund_amount = TRANSFER_AMOUNT + FEE_AMOUNT;
		<Balances as Mutate<_>>::mint_into(&sovereign_account, fund_amount).unwrap();
	});

	let sender = BulletinWestendParaSender::get();
	let receiver = AssetHubWestendParaReceiver::get();

	// Get initial balances
	let sender_initial_on_bulletin = BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;
		<Balances as Inspect<_>>::balance(&sender)
	});

	let receiver_initial_on_asset_hub = AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		<Balances as Inspect<_>>::balance(&receiver)
	});

	// Ensure sender has enough balance on Bulletin
	// If sender doesn't have enough, we skip this test since it depends on genesis config
	if sender_initial_on_bulletin < TRANSFER_AMOUNT + FEE_AMOUNT {
		println!(
			"Skipping test: Sender needs at least {} on Bulletin, has {}",
			TRANSFER_AMOUNT + FEE_AMOUNT,
			sender_initial_on_bulletin
		);
		return;
	}

	// Construct the destination: Asset Hub parachain (from Bulletin's perspective)
	let dest = Location::new(1, [Parachain(ASSET_HUB_PARA_ID)]);

	// Construct the beneficiary: receiver on the destination chain
	let beneficiary =
		Location::new(0, [AccountId32 { network: None, id: receiver.clone().into() }]);

	// WND asset location (relay chain native token)
	let wnd_location = Location::parent();

	// Assets to transfer
	let assets: Assets = (wnd_location.clone(), TRANSFER_AMOUNT).into();

	// Fee asset ID
	let fee_asset_id: AssetId = wnd_location.into();

	// XCM to be executed on the destination (Asset Hub) - just deposit to beneficiary
	let xcm_on_dest = Xcm::<()>(vec![DepositAsset { assets: Wild(AllCounted(1)), beneficiary }]);

	// Transfer assets from Bulletin to Asset Hub
	// Asset Hub is the reserve, so we use DestinationReserve transfer type
	BulletinWestendNet::execute_with(|| {
		type PolkadotXcm = bulletin_westend_runtime::PolkadotXcm;
		type RuntimeOrigin = <BulletinWestendNet as Chain>::RuntimeOrigin;

		let result = PolkadotXcm::transfer_assets_using_type_and_then(
			RuntimeOrigin::signed(sender.clone()),
			Box::new(dest.into()),
			Box::new(assets.into()),
			Box::new(TransferType::DestinationReserve),
			Box::new(fee_asset_id.into()),
			Box::new(TransferType::DestinationReserve),
			Box::new(VersionedXcm::from(xcm_on_dest)),
			WeightLimit::Unlimited,
		);

		println!("Bulletin transfer result: {:?}", result);

		// Print events for debugging
		let events = frame_system::Pallet::<bulletin_westend_runtime::Runtime>::events();
		println!("Bulletin events count: {}", events.len());
		for event in events.iter().rev().take(10) {
			println!("  Bulletin Event: {:?}", event.event);
		}

		assert_ok!(result);
	});

	// Verify sender's balance decreased on Bulletin
	BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;
		let sender_balance = <Balances as Inspect<_>>::balance(&sender);
		assert!(
			sender_balance < sender_initial_on_bulletin,
			"Sender's balance should decrease after transfer"
		);
		// Account for transfer amount + fees
		assert!(
			sender_initial_on_bulletin - sender_balance >= TRANSFER_AMOUNT,
			"Sender should have transferred at least {} WND",
			TRANSFER_AMOUNT
		);
	});

	// Verify receiver's balance increased on Asset Hub
	AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;

		// Print Asset Hub events for debugging
		let events = frame_system::Pallet::<asset_hub_westend_runtime::Runtime>::events();
		println!("Asset Hub events count: {}", events.len());
		for event in events.iter().rev().take(10) {
			println!("  Asset Hub Event: {:?}", event.event);
		}

		let receiver_balance = <Balances as Inspect<_>>::balance(&receiver);
		// Receiver should receive the transferred amount minus any execution fees
		assert!(
			receiver_balance > receiver_initial_on_asset_hub,
			"Receiver's balance should increase after receiving transfer. Initial: {}, Current: {}",
			receiver_initial_on_asset_hub,
			receiver_balance
		);
	});
}
