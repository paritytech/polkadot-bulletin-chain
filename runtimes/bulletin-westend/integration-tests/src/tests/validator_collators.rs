// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

//! Wiring of `pallet-validator-collators` with Asset Hub Westend.

use crate::{
	chains::{asset_hub_westend, AssetHubWestend},
	BulletinWestend, WestendMockNet,
};
use codec::{Decode, Encode};
use cumulus_primitives_core::AggregateMessageOrigin;
use emulated_integration_tests_common::{collators, xcm_emulator::pallet_aura};
use frame_support::{
	assert_ok,
	traits::{Get, GetCallMetadata},
};
use pallet_staking_async::OnEraStart;
use pallet_staking_async_points_converter as points_converter;
use pallet_validator_collators::{CollatorDraw, DrawnCollators};
use parachains_common::{AccountId, AuraId};
use sp_externalities::ExternalitiesExt;
use sp_keyring::Sr25519Keyring;
use sp_keystore::{testing::MemoryKeystore, KeystoreExt};
use sp_runtime::AccountId32;
use sp_staking::ServiceKind;
use std::collections::BTreeSet;
use xcm_emulator::{assert_expected_events, Chain, Network, Parachain, TestExt};

type AssetHubWestendNet = AssetHubWestend<WestendMockNet>;
type BulletinWestendNet = BulletinWestend<WestendMockNet>;
type AssetHubRuntime = <AssetHubWestendNet as Chain>::Runtime;
type BulletinRuntime = <BulletinWestendNet as Chain>::Runtime;
type BulletinValidatorCollators = pallet_validator_collators::Pallet<BulletinRuntime>;

fn register_keys_on_bulletin(who: &AccountId) -> AuraId {
	sp_externalities::with_externalities(|mut ext| {
		let _ = ext.register_extension(KeystoreExt::new(MemoryKeystore::new()));
	});
	let keys = bulletin_westend_runtime::SessionKeys::generate(&who.encode(), None);
	let aura = keys.keys.aura.clone();
	assert_ok!(pallet_session::Pallet::<BulletinRuntime>::set_keys(
		<BulletinWestendNet as Chain>::RuntimeOrigin::signed(who.clone()),
		keys.keys,
		keys.proof.encode(),
	));
	aura
}

fn invulnerables() -> Vec<(AccountId, AuraId)> {
	let mut invulnerables = collators::invulnerables()
		.into_iter()
		.map(|(who, aura, _)| (who, aura))
		.collect::<Vec<_>>();
	invulnerables.sort_by(|(a, _), (b, _)| a.cmp(b));
	invulnerables
}

#[test]
fn era_start_on_asset_hub_makes_validators_with_keys_collators_on_bulletin() {
	WestendMockNet::reset();

	let alice = Sr25519Keyring::Alice.to_account_id();
	let bob = Sr25519Keyring::Bob.to_account_id();
	let charlie = Sr25519Keyring::Charlie.to_account_id();
	let validators = vec![alice.clone(), bob.clone(), charlie];
	let bulletin = BulletinWestendNet::para_id();

	// GIVEN Alice and Bob registered Aura keys on Bulletin, Charlie did not, and root removed
	// Bulletin's default cap of zero collators
	let (alice_aura, bob_aura, session_before) = BulletinWestendNet::execute_with(|| {
		assert_ok!(BulletinValidatorCollators::set_max_collators(
			<BulletinWestendNet as Chain>::RuntimeOrigin::root(),
			CollatorDraw::All,
		));
		(
			register_keys_on_bulletin(&alice),
			register_keys_on_bulletin(&bob),
			pallet_session::Pallet::<BulletinRuntime>::current_index(),
		)
	});

	// WHEN Asset Hub starts era 1 with Alice, Bob and Charlie
	AssetHubWestendNet::execute_with(|| {
		type RuntimeEvent = <AssetHubWestendNet as Chain>::RuntimeEvent;
		asset_hub_westend_runtime::staking::AnnounceValidatorSet::on_era_start(1, &validators);
		assert_expected_events!(
			AssetHubWestendNet,
			vec![
				RuntimeEvent::ValidatorSetAnnouncer(
					pallet_validator_set_announcer::Event::AnnouncementSent { destination, era: 1 }
				) => { destination: *destination == bulletin, },
			]
		);
		assert!(!<AssetHubWestendNet as Chain>::events().iter().any(|event| matches!(
			event,
			RuntimeEvent::ValidatorSetAnnouncer(
				pallet_validator_set_announcer::Event::AnnouncementFailed { destination, .. }
			) if *destination == bulletin
		)));
	});

	// THEN Bulletin accepts the set from Asset Hub and stores it
	BulletinWestendNet::execute_with(|| {
		type RuntimeEvent = <BulletinWestendNet as Chain>::RuntimeEvent;
		assert_expected_events!(
			BulletinWestendNet,
			vec![
				RuntimeEvent::ValidatorCollators(
					pallet_validator_collators::Event::ValidatorSetReceived { era: 1, count: 3 }
				) => {},
				RuntimeEvent::MessageQueue(pallet_message_queue::Event::Processed {
					origin, success: true, ..
				}) => {
					origin: *origin ==
						AggregateMessageOrigin::Sibling(asset_hub_westend::PARA_ID.into()),
				},
			]
		);
		assert!(
			pallet_validator_collators::ExternalSetReceivedAt::<BulletinRuntime>::get().is_some()
		);
	});

	// AND after two forced rotations Bulletin collates with the invulnerables, Alice and Bob
	// Every `execute_with` runs one block, and the rotations run in the two blocks after the one
	// that received the set.
	BulletinWestendNet::execute_with(|| {});
	BulletinWestendNet::execute_with(|| {
		type Session = pallet_session::Pallet<BulletinRuntime>;
		let drawn = DrawnCollators::<BulletinRuntime>::get()
			.expect("the forced rotations drew the collators")
			.collators
			.into_inner();
		assert_eq!(
			drawn.iter().cloned().collect::<BTreeSet<_>>(),
			BTreeSet::from([alice.clone(), bob.clone()])
		);
		let aura_of = |who: &AccountId| if *who == alice { &alice_aura } else { &bob_aura };
		let (expected, expected_authorities): (Vec<_>, Vec<_>) = invulnerables()
			.into_iter()
			.chain(drawn.iter().map(|who| (who.clone(), aura_of(who).clone())))
			.unzip();
		assert_eq!(Session::current_index(), session_before + 2);
		assert_eq!(Session::validators(), expected);
		assert_eq!(
			pallet_aura::Authorities::<BulletinRuntime>::get().into_inner(),
			expected_authorities
		);
	});
}

#[test]
fn bulletin_reports_authored_blocks_to_asset_hub() {
	WestendMockNet::reset();

	let alice = Sr25519Keyring::Alice.to_account_id();
	let bob = Sr25519Keyring::Bob.to_account_id();
	let invulnerable = invulnerables()[0].0.clone();
	let bulletin = BulletinWestendNet::para_id();

	// GIVEN era 1 is active on Asset Hub with Alice exposed, and Bulletin's blocks earn 20 points
	AssetHubWestendNet::execute_with(|| {
		pallet_staking_async::ActiveEra::<AssetHubRuntime>::put(
			pallet_staking_async::ActiveEraInfo { index: 1, start: Some(0) },
		);
		pallet_staking_async::session_rotation::Eras::<AssetHubRuntime>::upsert_exposure(
			1,
			&alice,
			pallet_staking_async::Exposure { total: 1, own: 1, others: vec![] },
		);
		assert_ok!(points_converter::Pallet::<AssetHubRuntime>::set_points_per_service(
			<AssetHubWestendNet as Chain>::RuntimeOrigin::root(),
			bulletin,
			ServiceKind::Collation,
			20,
		));
	});

	// AND Bulletin stores the set of Alice and Bob, Alice authored 3 blocks, Bob 1 and an
	// invulnerable 2, and the next block is a report block. Bulletin drops the invulnerable,
	// which is not in its set, and Asset Hub drops Bob, who is in Bulletin's set but not exposed.
	BulletinWestendNet::execute_with(|| {
		assert_ok!(BulletinValidatorCollators::set_validators(
			<BulletinWestendNet as Chain>::RuntimeOrigin::root(),
			1,
			BTreeSet::from([alice.clone(), bob.clone()])
				.try_into()
				.expect("two validators fit `MaxValidators`"),
		));
		let note = |who: &AccountId, blocks: u32| {
			(0..blocks).for_each(|_| {
				<BulletinValidatorCollators as pallet_authorship::EventHandler<_, _>>::note_author(
					who.clone(),
				)
			})
		};
		note(&alice, 3);
		note(&bob, 1);
		note(&invulnerable, 2);
		let next = <BulletinWestendNet as Chain>::System::block_number() + 1;
		assert_ok!(BulletinValidatorCollators::set_report_period(
			<BulletinWestendNet as Chain>::RuntimeOrigin::root(),
			next,
		));
	});

	// WHEN Bulletin reaches the report block, whose `on_poll` the emulator does not run
	BulletinWestendNet::execute_with(|| {
		type RuntimeEvent = <BulletinWestendNet as Chain>::RuntimeEvent;
		<BulletinValidatorCollators as frame_support::traits::Hooks<_>>::on_poll(
			<BulletinWestendNet as Chain>::System::block_number(),
			&mut frame_support::weights::WeightMeter::new(),
		);
		assert_expected_events!(
			BulletinWestendNet,
			vec![
				RuntimeEvent::ValidatorCollators(
					pallet_validator_collators::Event::BlocksReported { reported: 2, .. }
				) => {},
			]
		);
		assert_eq!(pallet_validator_collators::AuthoredBlocks::<BulletinRuntime>::count(), 0);
	});

	// THEN Asset Hub executes the report unpaid, credits Alice's blocks with Bulletin's points
	// for the collation service, drops Bob's and credits nobody else
	AssetHubWestendNet::execute_with(|| {
		type RuntimeEvent = <AssetHubWestendNet as Chain>::RuntimeEvent;
		assert_expected_events!(
			AssetHubWestendNet,
			vec![
				RuntimeEvent::StakingPointsConverter(
					points_converter::Event::BlocksCredited { source, blocks: 4 }
				) => { source: *source == bulletin, },
				RuntimeEvent::Staking(
					pallet_staking_async::Event::ServicePointsCredited {
						service: ServiceKind::Collation,
						era: 1,
						credited: 1,
						dropped: 1,
					}
				) => {},
				RuntimeEvent::MessageQueue(pallet_message_queue::Event::Processed {
					origin, success: true, ..
				}) => { origin: *origin == AggregateMessageOrigin::Sibling(bulletin), },
			]
		);
		let points = pallet_staking_async::ErasRewardPoints::<AssetHubRuntime>::get(1);
		assert_eq!(points.individual.into_iter().collect::<Vec<_>>(), vec![(alice.clone(), 60)]);
		assert_eq!(points.total, 60);
		// AND they are recorded as collation points only
		let by_service = |service| {
			let points =
				pallet_staking_async::ErasRewardPointsByService::<AssetHubRuntime>::get(1, service);
			(points.total, points.individual.into_iter().collect::<Vec<_>>())
		};
		assert_eq!(by_service(ServiceKind::Collation), (60, vec![(alice, 60)]));
		assert_eq!(by_service(ServiceKind::RelayConsensus), (0, vec![]));
	});
}

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
