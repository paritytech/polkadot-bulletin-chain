// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Polkadot.

// Polkadot is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Polkadot is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Polkadot.  If not, see <http://www.gnu.org/licenses/>.

//! XCM configuration for Polkadot Bulletin chain.

use crate::{
	bridge_config::{BridgedNetwork, PeoplePolkadotLocation, ToBridgeHaulBlobExporter},
	AllPalletsWithSystem, RuntimeCall, RuntimeOrigin,
};

use codec::Encode;
use frame_support::{
	parameter_types,
	traits::{Contains, Equals, Everything, Nothing},
	weights::Weight,
};
use sp_core::ConstU32;
use sp_runtime::traits::Get;
use xcm::latest::prelude::*;
use xcm_builder::{
	AllowExplicitUnpaidExecutionFrom, FixedWeightBounds, FrameTransactionalProcessor,
	LocalExporter, LocationAsSuperuser, TrailingSetTopicAsId, WithComputedOrigin,
};
use xcm_executor::{
	traits::{WeightTrader, WithOriginFilter},
	AssetsInHolding,
};

parameter_types! {
	/// The Polkadot Bulletin Chain network ID.
	pub const ThisNetwork: NetworkId = NetworkId::PolkadotBulletin;
	/// Our location in the universe of consensus systems.
	pub UniversalLocation: InteriorLocation = ThisNetwork::get().into();

	/// The amount of weight an XCM operation takes. This is a safe overestimate.
	pub const BaseXcmWeight: Weight = Weight::from_parts(1_000_000_000, 0);
	/// Maximum number of instructions in a single XCM fragment. A sanity check against weight
	/// calculations getting too crazy.
	pub const MaxInstructions: u32 = 100;
}

pub struct UniversalAliases;
impl Contains<(Location, Junction)> for UniversalAliases {
	fn contains(l: &(Location, Junction)) -> bool {
		matches!(
			l,
			(
				origin_location,
				GlobalConsensus(bridged_network),
			) if origin_location == &PeoplePolkadotLocation::get() && bridged_network == &BridgedNetwork::get())
	}
}

/// Weight trader that does nothing.
pub struct NoopTrader;
impl WeightTrader for NoopTrader {
	fn new() -> Self {
		NoopTrader
	}

	fn buy_weight(
		&mut self,
		_weight: Weight,
		_payment: AssetsInHolding,
		_context: &XcmContext,
	) -> Result<AssetsInHolding, XcmError> {
		Ok(AssetsInHolding::new())
	}
}

/// The means that we convert an XCM origin `Location` into the runtime's `Origin` type for
/// local dispatch. This is a conversion function from an `OriginKind` type along with the
/// `Location` value and returns an `Origin` value or an error.
type XcmOriginToTransactDispatchOrigin =
	(LocationAsSuperuser<Equals<PeoplePolkadotLocation>, RuntimeOrigin>,);

/// Only bridged destination is supported.
pub type XcmRouter = LocalExporter<ToBridgeHaulBlobExporter, UniversalLocation>;

/// The barriers one of which must be passed for an XCM message to be executed.
pub type Barrier = TrailingSetTopicAsId<
	WithComputedOrigin<
		AllowExplicitUnpaidExecutionFrom<Equals<PeoplePolkadotLocation>>,
		UniversalLocation,
		ConstU32<8>,
	>,
>;

/// XCM executor configuration.
pub struct XcmConfig;
impl xcm_executor::Config for XcmConfig {
	type RuntimeCall = RuntimeCall;
	type XcmSender = XcmRouter;
	type AssetTransactor = ();
	type OriginConverter = XcmOriginToTransactDispatchOrigin;
	type IsReserve = ();
	type IsTeleporter = ();
	type UniversalLocation = UniversalLocation;
	type Barrier = Barrier;
	// TODO [bridge]: is it ok to use `FixedWeightBounds` here? We don't have the `pallet-xcm`
	// and IIUC can't use XCM benchmarks because of that?
	type Weigher = FixedWeightBounds<BaseXcmWeight, RuntimeCall, MaxInstructions>;
	type Trader = NoopTrader;
	type ResponseHandler = ();
	type AssetTrap = ();
	type AssetLocker = ();
	type AssetExchanger = ();
	type AssetClaims = ();
	type SubscriptionService = ();
	type PalletInstancesInfo = AllPalletsWithSystem;
	type MaxAssetsIntoHolding = ConstU32<0>;
	type FeeManager = ();
	// TODO: Why? This could allow processing of `ExportMessage` from People?
	type MessageExporter = ToBridgeHaulBlobExporter;
	type UniversalAliases = UniversalAliases;
	type CallDispatcher = WithOriginFilter<Everything>;
	type SafeCallFilter = Everything;
	type Aliasers = Nothing;
	type TransactionalProcessor = FrameTransactionalProcessor;
	type HrmpNewChannelOpenRequestHandler = ();
	type HrmpChannelAcceptedHandler = ();
	type HrmpChannelClosingHandler = ();
	type XcmRecorder = ();
	// TODO: maybe add here some emitter?
	type XcmEventEmitter = ();
}

/// `SencXcm` implementation that executes XCM message at this chain.
pub struct ImmediateExecutingXcmRouter<Execute, Call, AsOrigin>(
	core::marker::PhantomData<(Execute, Call, AsOrigin)>,
);
impl<Execute: ExecuteXcm<Call>, Call, AsOrigin: Get<Location>> SendXcm
	for ImmediateExecutingXcmRouter<Execute, Call, AsOrigin>
{
	type Ticket = Xcm<Call>;

	fn validate(
		dest: &mut Option<Location>,
		msg: &mut Option<Xcm<()>>,
	) -> SendResult<Self::Ticket> {
		let dest = dest.as_ref().ok_or(SendError::MissingArgument)?;
		match dest.unpack() {
			// Accept only messages for `Here`.
			(0, []) => {
				let msg = msg.take().ok_or(SendError::MissingArgument)?;
				Ok((Xcm::<Call>::from(msg), Assets::new()))
			},
			_ => {
				tracing::trace!(
					target: "xcm::execute::validate",
					"ImmediateExecutingXcmRouter unsupported destination: {dest:?}",
				);
				Err(SendError::NotApplicable)
			},
		}
	}

	fn deliver(message: Self::Ticket) -> Result<XcmHash, SendError> {
		// we allow any calls through XCM, so no limit. It doesn't mean that we really spend
		// the `Weight::MAX` here. Actual message weight is computed by `MessageDispatch`
		// implementation and is limited by the weight of the ` receive_messsages_proof ` call.
		let weight_limit = Weight::MAX;

		// execute the XCM program
		let mut message_hash = message.using_encoded(sp_io::hashing::blake2_256);
		Execute::prepare_and_execute(
			AsOrigin::get(),
			message,
			&mut message_hash,
			weight_limit,
			Weight::zero(),
		)
		.ensure_complete()
		.map(|_| message_hash)
		.map_err(|e| {
			tracing::trace!(
				target: "xcm::execute::deliver",
				"XCM message from {:?} was dispatched with an error: {:?}",
				AsOrigin::get(),
				e,
			);

			// nothing better than this error :/
			SendError::Transport("XCM execution failed!")
		})
	}
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;
	use crate::{
		polkadot_bridge_config::{
			bp_people_polkadot::PEOPLE_POLKADOT_PARACHAIN_ID, tests::run_test,
			WithPeoplePolkadotMessagesInstance, XcmBlobMessageDispatchResult, XCM_LANE,
		},
		Runtime,
	};
	use bp_messages::{
		target_chain::{DispatchMessage, DispatchMessageData, MessageDispatch},
		MessageKey,
	};
	use codec::Encode;
	use pallet_bridge_messages::Config as MessagesConfig;
	use sp_keyring::Sr25519Keyring as AccountKeyring;
	use xcm::{prelude::VersionedXcm, VersionedInteriorLocation};
	use xcm_builder::{BridgeMessage, DispatchBlobError};
	use xcm_executor::traits::{Properties, ShouldExecute};

	type Dispatcher =
		<Runtime as MessagesConfig<WithPeoplePolkadotMessagesInstance>>::MessageDispatch;

	fn test_storage_key() -> Vec<u8> {
		(*b"test_key").to_vec()
	}

	fn test_storage_value() -> Vec<u8> {
		(*b"test_value").to_vec()
	}

	pub fn encoded_xcm_message_with_root_call_from_people_polkadot(
		origin_kind: OriginKind,
		descend_origin: Option<InteriorLocation>,
	) -> Vec<u8> {
		let mut xcm = Xcm::<()>::builder_unsafe()
			.universal_origin(GlobalConsensus(BridgedNetwork::get()))
			.descend_origin(Parachain(PEOPLE_POLKADOT_PARACHAIN_ID));
		if let Some(descend_origin) = descend_origin {
			xcm = xcm.descend_origin(descend_origin);
		}
		let message = VersionedXcm::from(
			xcm.unpaid_execution(Unlimited, None)
				.transact(
					origin_kind,
					None,
					RuntimeCall::System(frame_system::Call::set_storage {
						items: vec![(test_storage_key(), test_storage_value())],
					})
					.encode(),
				)
				.build(),
		);

		let universal_dest: VersionedInteriorLocation = GlobalConsensus(ThisNetwork::get()).into();

		BridgeMessage { universal_dest, message }.encode()
	}

	#[test]
	fn messages_from_people_polkadot_are_dispatched_and_executed() {
		run_test(|| {
			// Ok - dispatches OriginKind::Superuser from a people chain.
			assert_eq!(frame_support::storage::unhashed::get_raw(&test_storage_key()), None);
			assert_eq!(
				Dispatcher::dispatch(DispatchMessage {
					key: MessageKey { lane_id: XCM_LANE, nonce: 1 },
					data: DispatchMessageData {
						payload: Ok(encoded_xcm_message_with_root_call_from_people_polkadot(
							OriginKind::Superuser,
							None,
						)),
					},
				})
				.dispatch_level_result,
				XcmBlobMessageDispatchResult::Dispatched
			);
			assert_eq!(
				frame_support::storage::unhashed::get_raw(&test_storage_key()),
				Some(test_storage_value()),
			);
		});

		// Err - OriginKind::Xcm is not supported
		assert_eq!(
			Dispatcher::dispatch(DispatchMessage {
				key: MessageKey { lane_id: XCM_LANE, nonce: 1 },
				data: DispatchMessageData {
					payload: Ok(encoded_xcm_message_with_root_call_from_people_polkadot(
						OriginKind::Xcm,
						None,
					)),
				},
			})
			.dispatch_level_result,
			XcmBlobMessageDispatchResult::NotDispatched(Some(DispatchBlobError::RoutingError))
		);

		// Err - people chain users cannot trigger transacting
		assert_eq!(
			Dispatcher::dispatch(DispatchMessage {
				key: MessageKey { lane_id: XCM_LANE, nonce: 1 },
				data: DispatchMessageData {
					payload: Ok(encoded_xcm_message_with_root_call_from_people_polkadot(
						OriginKind::Superuser,
						Some(Junctions::X1(
							[Junction::AccountId32 {
								network: None,
								id: AccountKeyring::Alice.public().0,
							}]
							.into()
						)),
					)),
				},
			})
			.dispatch_level_result,
			XcmBlobMessageDispatchResult::NotDispatched(Some(DispatchBlobError::RoutingError))
		);
		// Err - people chain users cannot trigger transacting
		assert_eq!(
			Dispatcher::dispatch(DispatchMessage {
				key: MessageKey { lane_id: XCM_LANE, nonce: 1 },
				data: DispatchMessageData {
					payload: Ok(encoded_xcm_message_with_root_call_from_people_polkadot(
						OriginKind::Xcm,
						Some(Junctions::X1(
							[Junction::AccountId32 {
								network: None,
								id: AccountKeyring::Alice.public().0,
							}]
							.into()
						)),
					)),
				},
			})
			.dispatch_level_result,
			XcmBlobMessageDispatchResult::NotDispatched(Some(DispatchBlobError::RoutingError))
		);
	}

	#[test]
	fn expected_message_from_people_polkadot_passes_barrier() {
		// prepare a message that we expect to come from the Polkadot BH
		// (everything is relative to Polkadot BH)
		let people_polkadot_as_universal_source: InteriorLocation =
			[GlobalConsensus(BridgedNetwork::get()), Parachain(PEOPLE_POLKADOT_PARACHAIN_ID)]
				.into();
		let (local_net, local_sub) = people_polkadot_as_universal_source.split_global().unwrap();
		let mut xcm: Xcm<RuntimeCall> = vec![
			UniversalOrigin(GlobalConsensus(local_net)),
			DescendOrigin(local_sub),
			UnpaidExecution { weight_limit: Unlimited, check_origin: None },
			Transact {
				origin_kind: OriginKind::Superuser,
				fallback_max_weight: None,
				call: RuntimeCall::System(frame_system::Call::remark { remark: vec![42] })
					.encode()
					.into(),
			},
		]
		.into();

		// ensure that it passes local XCM Barrier
		assert_eq!(
			Barrier::should_execute(
				&Here.into(),
				xcm.inner_mut(),
				Weight::MAX,
				&mut Properties { weight_credit: Weight::MAX, message_id: None },
			),
			Ok(())
		);
		assert_eq!(
			Barrier::should_execute(
				&PeoplePolkadotLocation::get(),
				xcm.inner_mut(),
				Weight::MAX,
				&mut Properties { weight_credit: Weight::MAX, message_id: None },
			),
			Ok(())
		);
	}
}
