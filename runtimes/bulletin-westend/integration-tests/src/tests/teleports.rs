// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

//! Teleport tests for WND between Asset Hub Westend and Bulletin Westend.
//!
//! Bulletin Westend's XCM config accepts teleports of the relay chain native
//! token from the relay chain and sibling system parachains (see
//! `TrustedTeleporters = ConcreteAssetFromSystem<TokenRelayLocation>`).
//! Asset Hub Westend's `TrustedTeleporters` also uses `ConcreteAssetFromSystem`,
//! which accepts any sibling parachain with a `ParaId` below 2000 as a system
//! parachain. Both parachain ids are below 2000, so teleports are accepted on
//! both sides in both directions.

use crate::{
	chains::{asset_hub_westend::PARA_ID as ASSET_HUB_PARA_ID, AssetHubWestend},
	AssetHubWestendParaReceiver, AssetHubWestendParaSender, BulletinWestend,
	BulletinWestendParaReceiver, BulletinWestendParaSender, WestendMockNet, BULLETIN_PARA_ID,
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
/// 3. Bob's balance on Bulletin increases (WND minted on the receiving chain, because Bulletin
///    trusts Asset Hub as a system-parachain teleporter).
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

/// Test teleport of WND from Bulletin Westend back to Asset Hub Westend.
///
/// This test verifies that:
/// 1. Alice on Bulletin can initiate a teleport to Asset Hub (both chains trust each other as
///    system-parachain teleporters of WND).
/// 2. Alice's balance on Bulletin decreases (WND burned locally).
/// 3. Bob's balance on Asset Hub increases, and Asset Hub's teleport-tracking checking account
///    burns exactly the teleported amount.
#[test]
fn teleport_wnd_from_bulletin_to_asset_hub() {
	WestendMockNet::reset();

	let sender = BulletinWestendParaSender::get();
	let receiver = AssetHubWestendParaReceiver::get();

	// Asset Hub tracks WND teleports (`MintLocation::Local`): incoming teleports burn from its
	// checking account. Upstream's emulated genesis pre-funds it; ours doesn't, so fund it here.
	let check_account = AssetHubWestendNet::execute_with(|| {
		asset_hub_westend_runtime::PolkadotXcm::check_account()
	});
	AssetHubWestendNet::fund_accounts(vec![(check_account.clone(), TRANSFER_AMOUNT * 2)]);

	let sender_initial_on_bulletin = BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;
		<Balances as Inspect<_>>::balance(&sender)
	});

	let (receiver_initial_on_asset_hub, check_account_initial) =
		AssetHubWestendNet::execute_with(|| {
			type Balances = asset_hub_westend_runtime::Balances;
			(
				<Balances as Inspect<_>>::balance(&receiver),
				<Balances as Inspect<_>>::balance(&check_account),
			)
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
			Box::new(TransferType::Teleport),
			Box::new(fee_asset_id.into()),
			Box::new(TransferType::Teleport),
			Box::new(VersionedXcm::from(xcm_on_dest)),
			WeightLimit::Unlimited,
		));
	});

	BulletinWestendNet::execute_with(|| {
		type Balances = bulletin_westend_runtime::Balances;
		let sender_balance = <Balances as Inspect<_>>::balance(&sender);
		assert!(
			sender_balance < sender_initial_on_bulletin,
			"Sender's balance should decrease after teleport"
		);
		assert!(
			sender_initial_on_bulletin - sender_balance >= TRANSFER_AMOUNT,
			"Sender should have teleported at least {TRANSFER_AMOUNT} WND"
		);
	});

	AssetHubWestendNet::execute_with(|| {
		type Balances = asset_hub_westend_runtime::Balances;
		let receiver_balance = <Balances as Inspect<_>>::balance(&receiver);
		assert!(
			receiver_balance > receiver_initial_on_asset_hub,
			"Receiver's balance should increase after receiving teleport. Initial: {receiver_initial_on_asset_hub}, Current: {receiver_balance}"
		);
		let check_account_balance = <Balances as Inspect<_>>::balance(&check_account);
		assert_eq!(
			check_account_initial - check_account_balance,
			TRANSFER_AMOUNT,
			"Checking account should burn exactly the teleported amount"
		);
	});
}
