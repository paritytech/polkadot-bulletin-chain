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

//! Reserve transfer tests for DOT between Asset Hub and Bulletin Polkadot parachain.
//!
//! These tests demonstrate:
//! 1. DOT reserve transfers from Asset Hub to Bulletin (Asset Hub is reserve - LocalReserve)
//! 2. DOT reserve transfers from Bulletin back to Asset Hub (DestinationReserve)

use crate::{
	AssetHubWestendParaReceiver, AssetHubWestendParaSender, BulletinPolkadotParachain,
	BulletinPolkadotParachainParaReceiver, BulletinPolkadotParachainParaSender, MockNet,
	BULLETIN_PARA_ID,
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

/// Asset Hub parachain ID.
const ASSET_HUB_PARA_ID: u32 = 1000;

/// Amount to transfer in tests.
const TRANSFER_AMOUNT: Balance = 1_000_000_000_000; // 1 DOT

/// Amount to use for fees.
const FEE_AMOUNT: Balance = 500_000_000_000; // 0.5 DOT

/// Type alias for AssetHubWestend with our network.
type AssetHubNet = AssetHubWestend<MockNet>;

/// Type alias for BulletinPolkadotParachain with our network.
type BulletinNet = BulletinPolkadotParachain<MockNet>;

/// Test reserve transfer of DOT from Asset Hub to Bulletin Polkadot parachain.
///
/// This test verifies that:
/// 1. Alice on Asset Hub can initiate a reserve transfer to Bulletin
/// 2. The transfer reduces Alice's balance on Asset Hub
/// 3. Bob's balance increases on Bulletin (receiving the transferred DOT)
///
/// Since Asset Hub is the reserve for DOT, we use LocalReserve transfer type.
#[test]
fn reserve_transfer_dot_from_asset_hub_to_bulletin() {
	MockNet::reset();

	let sender = AssetHubWestendParaSender::get();
	let receiver = BulletinPolkadotParachainParaReceiver::get();

	let sender_initial_on_asset_hub = AssetHubNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		<Balances as Inspect<_>>::balance(&sender)
	});

	let receiver_initial_on_bulletin = BulletinNet::execute_with(|| {
		type Balances = bulletin_polkadot_parachain_runtime::Balances;
		<Balances as Inspect<_>>::balance(&receiver)
	});

	assert!(
		sender_initial_on_asset_hub >= TRANSFER_AMOUNT + FEE_AMOUNT,
		"Sender needs sufficient balance on Asset Hub"
	);

	let dest = Location::new(1, [Parachain(BULLETIN_PARA_ID)]);
	let beneficiary =
		Location::new(0, [AccountId32 { network: None, id: receiver.clone().into() }]);
	let relay_token_location = Location::parent();
	let assets: Assets = (relay_token_location.clone(), TRANSFER_AMOUNT).into();
	let fee_asset_id: AssetId = relay_token_location.into();
	let xcm_on_dest = Xcm::<()>(vec![DepositAsset { assets: Wild(AllCounted(1)), beneficiary }]);

	AssetHubNet::execute_with(|| {
		type PolkadotXcm = asset_hub_westend_runtime::PolkadotXcm;
		type RuntimeOrigin = <AssetHubNet as Chain>::RuntimeOrigin;

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

	AssetHubNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		let sender_balance = <Balances as Inspect<_>>::balance(&sender);
		assert!(
			sender_balance < sender_initial_on_asset_hub,
			"Sender's balance should decrease after transfer"
		);
		assert!(
			sender_initial_on_asset_hub - sender_balance >= TRANSFER_AMOUNT,
			"Sender should have transferred at least {TRANSFER_AMOUNT} DOT"
		);
	});

	BulletinNet::execute_with(|| {
		type Balances = bulletin_polkadot_parachain_runtime::Balances;
		let receiver_balance = <Balances as Inspect<_>>::balance(&receiver);
		assert!(
			receiver_balance > receiver_initial_on_bulletin,
			"Receiver's balance should increase after receiving transfer. Initial: {receiver_initial_on_bulletin}, Current: {receiver_balance}"
		);
	});
}

/// Test reserve transfer of DOT from Bulletin Polkadot parachain to Asset Hub.
///
/// This test verifies that:
/// 1. Alice on Bulletin can initiate a reserve transfer back to Asset Hub
/// 2. The transfer reduces Alice's balance on Bulletin
/// 3. Bob's balance increases on Asset Hub
///
/// Since Asset Hub is the reserve for DOT, we use DestinationReserve transfer type.
#[test]
fn reserve_transfer_dot_from_bulletin_to_asset_hub() {
	MockNet::reset();

	// Fund Bulletin's sovereign account on Asset Hub. When Bulletin sends a DestinationReserve
	// transfer, Asset Hub will withdraw from Bulletin's sovereign account.
	AssetHubNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		let bulletin_location = Location::new(1, [Parachain(BULLETIN_PARA_ID)]);
		let sovereign_account =
			<AssetHubNet as Parachain>::sovereign_account_id_of(bulletin_location);
		<Balances as Mutate<_>>::mint_into(&sovereign_account, TRANSFER_AMOUNT + FEE_AMOUNT)
			.unwrap();
	});

	let sender = BulletinPolkadotParachainParaSender::get();
	let receiver = AssetHubWestendParaReceiver::get();

	let sender_initial_on_bulletin = BulletinNet::execute_with(|| {
		type Balances = bulletin_polkadot_parachain_runtime::Balances;
		<Balances as Inspect<_>>::balance(&sender)
	});

	let receiver_initial_on_asset_hub = AssetHubNet::execute_with(|| {
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
	let relay_token_location = Location::parent();
	let assets: Assets = (relay_token_location.clone(), TRANSFER_AMOUNT).into();
	let fee_asset_id: AssetId = relay_token_location.into();
	let xcm_on_dest = Xcm::<()>(vec![DepositAsset { assets: Wild(AllCounted(1)), beneficiary }]);

	BulletinNet::execute_with(|| {
		type PolkadotXcm = bulletin_polkadot_parachain_runtime::PolkadotXcm;
		type RuntimeOrigin = <BulletinNet as Chain>::RuntimeOrigin;

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

	BulletinNet::execute_with(|| {
		type Balances = bulletin_polkadot_parachain_runtime::Balances;
		let sender_balance = <Balances as Inspect<_>>::balance(&sender);
		assert!(
			sender_balance < sender_initial_on_bulletin,
			"Sender's balance should decrease after transfer"
		);
		assert!(
			sender_initial_on_bulletin - sender_balance >= TRANSFER_AMOUNT,
			"Sender should have transferred at least {TRANSFER_AMOUNT} DOT"
		);
	});

	AssetHubNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		let receiver_balance = <Balances as Inspect<_>>::balance(&receiver);
		assert!(
			receiver_balance > receiver_initial_on_asset_hub,
			"Receiver's balance should increase after receiving transfer. Initial: {receiver_initial_on_asset_hub}, Current: {receiver_balance}"
		);
	});
}
