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

//! Teleport tests for WND from Asset Hub Westend to Bulletin Westend.
//!
//! Bulletin Westend's XCM config accepts teleports of the relay chain native
//! token from the relay chain and sibling system parachains (see
//! `TrustedTeleporters = ConcreteAssetFromSystem<TokenRelayLocation>`).
//! Asset Hub Westend's `TrustedTeleporters` also uses `ConcreteAssetFromSystem`,
//! which accepts any sibling parachain with a `ParaId` below 2000 as a system
//! parachain. Bulletin's parachain id is below 2000, so AH → Bulletin teleports
//! are accepted on both sides.

use crate::{
	chains::AssetHubWestend, AssetHubWestendParaSender, BulletinWestend,
	BulletinWestendParaReceiver, WestendMockNet, BULLETIN_PARA_ID,
};
use frame_support::{assert_ok, traits::fungible::Inspect};
use parachains_common::Balance;
use xcm::{latest::prelude::*, VersionedXcm};
use xcm_emulator::{Chain, Network, TestExt};
use xcm_executor::traits::TransferType;

/// Amount to transfer in tests.
const TRANSFER_AMOUNT: Balance = 1_000_000_000_000; // 1 WND

/// Amount to use for fees.
const FEE_AMOUNT: Balance = 500_000_000_000; // 0.5 WND

/// Type alias for AssetHubWestend with our network.
type AssetHubWestendNet = AssetHubWestend<WestendMockNet>;

/// Type alias for BulletinWestend with our network.
type BulletinWestendNet = BulletinWestend<WestendMockNet>;

/// Test teleport of WND from Asset Hub Westend to Bulletin Westend.
///
/// This test verifies that:
/// 1. Alice on Asset Hub can initiate a teleport to Bulletin.
/// 2. Alice's balance on Asset Hub decreases (WND burned locally).
/// 3. Bob's balance on Bulletin increases (WND minted on the receiving chain,
///    because Bulletin trusts Asset Hub as a system-parachain teleporter).
#[test]
fn teleport_wnd_from_asset_hub_to_bulletin() {
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
			Box::new(TransferType::Teleport),
			Box::new(fee_asset_id.into()),
			Box::new(TransferType::Teleport),
			Box::new(VersionedXcm::from(xcm_on_dest)),
			WeightLimit::Unlimited,
		));
	});

	AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		let sender_balance = <Balances as Inspect<_>>::balance(&sender);
		assert!(
			sender_balance < sender_initial_on_asset_hub,
			"Sender's balance should decrease after teleport"
		);
		assert!(
			sender_initial_on_asset_hub - sender_balance >= TRANSFER_AMOUNT,
			"Sender should have teleported at least {TRANSFER_AMOUNT} WND"
		);
	});

	BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;
		let receiver_balance = <Balances as Inspect<_>>::balance(&receiver);
		assert!(
			receiver_balance > receiver_initial_on_bulletin,
			"Receiver's balance should increase after receiving teleport. Initial: {receiver_initial_on_bulletin}, Current: {receiver_balance}"
		);
	});
}
