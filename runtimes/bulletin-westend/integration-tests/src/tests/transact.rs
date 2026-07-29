// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

//! XCM `Transact` policy tests: storage-mutating calls must not be reachable
//! via XCM from other chains.

use crate::{
	chains::{asset_hub_westend, AssetHubWestend},
	BulletinWestend, WestendMockNet, BULLETIN_PARA_ID,
};
use codec::Encode;
use cumulus_primitives_core::AggregateMessageOrigin;
use emulated_integration_tests_common::xcm_helpers::xcm_transact_unpaid_execution;
use frame_support::assert_ok;
use xcm::latest::prelude::*;
use xcm_emulator::{assert_expected_events, Chain, Network, Parachain, TestExt};

type AssetHubWestendNet = AssetHubWestend<WestendMockNet>;
type BulletinWestendNet = BulletinWestend<WestendMockNet>;

/// An XCM `Transact` carrying `transaction_storage::store`, sent from Asset Hub
/// over XCMP, must be rejected on Bulletin.
///
/// The message passes the barrier (siblings get explicit unpaid execution) and
/// fails inside the `Transact` instruction: the `SafeCallFilter`
/// (`EverythingBut<StorageCallInspector>`) rejects the decoded call with
/// `XcmError::NoPermission` before origin conversion and dispatch. The executor
/// emits `PolkadotXcm::ProcessXcmError` and the message queue reports the
/// message as processed unsuccessfully.
#[test]
fn xcm_transact_store_from_asset_hub_is_blocked() {
	WestendMockNet::reset();

	let data = vec![42u8; 100];

	// The account `store` would be dispatched as (via
	// `OriginKind::SovereignAccount`) if the filter let the call through.
	let ah_sovereign = BulletinWestendNet::sovereign_account_id_of(Location::new(
		1,
		[Parachain(asset_hub_westend::PARA_ID)],
	));

	// Authorize the sovereign account so the call filter is the only blocker.
	let granted_extent = BulletinWestendNet::execute_with(|| {
		type TransactionStorage = bulletin_westend_runtime::TransactionStorage;
		type RuntimeOrigin = <BulletinWestendNet as Chain>::RuntimeOrigin;

		assert_ok!(TransactionStorage::authorize_account(
			RuntimeOrigin::root(),
			ah_sovereign.clone(),
			1,
			data.len() as u64,
		));
		let extent = TransactionStorage::account_authorization_extent(ah_sovereign.clone());
		assert_ne!(extent, Default::default(), "authorization must have been granted");
		extent
	});

	let store_call = bulletin_westend_runtime::RuntimeCall::TransactionStorage(
		pallet_bulletin_transaction_storage::Call::<bulletin_westend_runtime::Runtime>::store {
			data,
		},
	);
	let xcm =
		xcm_transact_unpaid_execution(store_call.encode().into(), OriginKind::SovereignAccount);

	AssetHubWestendNet::execute_with(|| {
		type PolkadotXcm = asset_hub_westend_runtime::PolkadotXcm;
		type RuntimeOrigin = <AssetHubWestendNet as Chain>::RuntimeOrigin;

		// Root sends from `Here`, so the message arrives on Bulletin with the
		// plain sibling-parachain origin and passes the unpaid-execution barrier.
		assert_ok!(PolkadotXcm::send(
			RuntimeOrigin::root(),
			Box::new(Location::new(1, [Parachain(BULLETIN_PARA_ID)]).into()),
			Box::new(xcm),
		));
		AssetHubWestendNet::assert_xcm_pallet_sent();
	});

	BulletinWestendNet::execute_with(|| {
		type RuntimeEvent = <BulletinWestendNet as Chain>::RuntimeEvent;
		type TransactionStorage = bulletin_westend_runtime::TransactionStorage;

		assert_expected_events!(
			BulletinWestendNet,
			vec![
				RuntimeEvent::PolkadotXcm(pallet_xcm::Event::ProcessXcmError { error, .. }) => {
					error: *error == XcmError::NoPermission,
				},
				RuntimeEvent::MessageQueue(pallet_message_queue::Event::Processed {
					origin, success: false, ..
				}) => {
					origin: *origin ==
						AggregateMessageOrigin::Sibling(asset_hub_westend::PARA_ID.into()),
				},
			]
		);

		// Nothing was stored and the authorization was not consumed.
		assert!(
			!BulletinWestendNet::events().iter().any(|event| matches!(
				event,
				RuntimeEvent::TransactionStorage(
					pallet_bulletin_transaction_storage::Event::Stored { .. }
				)
			)),
			"no data must be stored on Bulletin"
		);
		assert_eq!(TransactionStorage::account_authorization_extent(ah_sovereign), granted_extent);
	});
}
