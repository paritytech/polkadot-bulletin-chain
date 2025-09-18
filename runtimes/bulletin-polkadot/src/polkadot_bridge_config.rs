use crate::{
	xcm_config::{decode_bridge_message, XcmConfig},
	ConstU32, Runtime, RuntimeEvent,
};
use bp_messages::{
	source_chain::MessagesBridge,
	target_chain::{DispatchMessage, MessageDispatch},
	LegacyLaneId,
};
use bp_parachains::SingleParaStoredHeaderDataBuilder;
use bp_runtime::messages::MessageDispatchResult;
use codec::{Decode, DecodeWithMemTracking, Encode};
use frame_support::{parameter_types, CloneNoBound, EqNoBound, PartialEqNoBound};
use pallet_xcm_bridge_hub::XcmAsPlainPayload;
use scale_info::TypeInfo;
use sp_runtime::SaturatedConversion;
use sp_std::marker::PhantomData;
use xcm::prelude::*;
use xcm_builder::{DispatchBlob, DispatchBlobError, HaulBlob, HaulBlobError, HaulBlobExporter};
use xcm_executor::XcmExecutor;

use frame_support::weights::Weight;

// TODO: when migrated to the Fellows, we can remove and reuse Fellows ones
pub mod bp_polkadot {
	use bp_header_chain::ChainWithGrandpa;
	use bp_polkadot_core::*;
	use bp_runtime::{decl_bridge_finality_runtime_apis, Chain, ChainId};
	use frame_support::weights::Weight;
	use sp_runtime::StateVersion;

	/// Polkadot Chain
	pub struct Polkadot;

	impl Chain for Polkadot {
		const ID: ChainId = *b"pdot";

		type BlockNumber = BlockNumber;
		type Hash = Hash;
		type Hasher = Hasher;
		type Header = Header;

		type AccountId = AccountId;
		type Balance = Balance;
		type Nonce = Nonce;
		type Signature = Signature;

		const STATE_VERSION: StateVersion = StateVersion::V1;

		fn max_extrinsic_size() -> u32 {
			max_extrinsic_size()
		}

		fn max_extrinsic_weight() -> Weight {
			max_extrinsic_weight()
		}
	}

	impl ChainWithGrandpa for Polkadot {
		const WITH_CHAIN_GRANDPA_PALLET_NAME: &'static str = WITH_POLKADOT_GRANDPA_PALLET_NAME;
		const MAX_AUTHORITIES_COUNT: u32 = MAX_AUTHORITIES_COUNT;
		const REASONABLE_HEADERS_IN_JUSTIFICATION_ANCESTRY: u32 =
			REASONABLE_HEADERS_IN_JUSTIFICATION_ANCESTRY;
		const MAX_MANDATORY_HEADER_SIZE: u32 = MAX_MANDATORY_HEADER_SIZE;
		const AVERAGE_HEADER_SIZE: u32 = AVERAGE_HEADER_SIZE;
	}

	/// Name of the With-Polkadot GRANDPA pallet instance that is deployed at bridged chains.
	pub const WITH_POLKADOT_GRANDPA_PALLET_NAME: &str = "BridgePolkadotGrandpa";

	/// Maximal size of encoded `bp_parachains::ParaStoredHeaderData` structure among all Polkadot
	/// parachains.
	///
	/// It includes the block number and state root, so it shall be near 40 bytes, but let's have
	/// some reserve.
	pub const MAX_NESTED_PARACHAIN_HEAD_DATA_SIZE: u32 = 128;

	decl_bridge_finality_runtime_apis!(polkadot, grandpa);
}

// TODO: when migrated to the Fellows, we can remove and reuse Fellows ones
pub mod bp_people_polkadot {
	use bp_bridge_hub_cumulus::*;
	pub use bp_bridge_hub_cumulus::{BlockNumber, Hash, EXTRA_STORAGE_PROOF_SIZE};
	use bp_messages::*;
	use bp_runtime::{
		decl_bridge_finality_runtime_apis, decl_bridge_messages_runtime_apis, Chain, ChainId,
		Parachain,
	};
	use frame_support::{dispatch::DispatchClass, weights::Weight};
	use sp_runtime::{RuntimeDebug, StateVersion};

	/// PeoplePolkadot parachain.
	#[derive(RuntimeDebug)]
	pub struct PeoplePolkadot;

	impl Chain for PeoplePolkadot {
		const ID: ChainId = *b"phpd";
		const STATE_VERSION: StateVersion = StateVersion::V1;

		type BlockNumber = BlockNumber;
		type Hash = Hash;
		type Hasher = Hasher;
		type Header = Header;

		type AccountId = AccountId;
		type Balance = Balance;
		type Nonce = Nonce;
		type Signature = Signature;

		fn max_extrinsic_size() -> u32 {
			*BlockLength::get().max.get(DispatchClass::Normal)
		}

		fn max_extrinsic_weight() -> Weight {
			BlockWeights::get()
				.get(DispatchClass::Normal)
				.max_extrinsic
				.unwrap_or(Weight::MAX)
		}
	}

	impl Parachain for PeoplePolkadot {
		const PARACHAIN_ID: u32 = PEOPLE_POLKADOT_PARACHAIN_ID;
		const MAX_HEADER_SIZE: u32 = MAX_BRIDGE_HUB_HEADER_SIZE;
	}

	impl ChainWithMessages for PeoplePolkadot {
		const WITH_CHAIN_MESSAGES_PALLET_NAME: &'static str =
			WITH_PEOPLE_POLKADOT_MESSAGES_PALLET_NAME;
		const MAX_UNREWARDED_RELAYERS_IN_CONFIRMATION_TX: MessageNonce =
			MAX_UNREWARDED_RELAYERS_IN_CONFIRMATION_TX;
		/// This constant limits the maximum number of messages in `receive_messages_proof`.
		/// We need to adjust it from 4096 to 2024 due to the actual weights identified by
		/// `check_message_lane_weights`. A higher value can be set once we switch
		/// `max_extrinsic_weight` to `BlockWeightsForAsyncBacking`.
		const MAX_UNCONFIRMED_MESSAGES_IN_CONFIRMATION_TX: MessageNonce = 2024;
	}

	/// Identifier of PeoplePolkadot in the Polkadot relay chain.
	pub const PEOPLE_POLKADOT_PARACHAIN_ID: u32 = 1004;

	/// Name of the With-PeoplePolkadot messages pallet instance that is deployed at bridged chains.
	pub const WITH_PEOPLE_POLKADOT_MESSAGES_PALLET_NAME: &str = "BridgePolkadotMessages";

	decl_bridge_finality_runtime_apis!(people_polkadot);
	decl_bridge_messages_runtime_apis!(people_polkadot, LegacyLaneId);
}

/// Lane that we are using to send and receive messages.
pub const XCM_LANE: LegacyLaneId = LegacyLaneId([0, 0, 0, 0]);

parameter_types! {
	pub PolkadotGlobalConsensusNetwork: NetworkId = NetworkId::Polkadot;
	pub BridgedNetwork: NetworkId = PolkadotGlobalConsensusNetwork::get();
	pub PolkadotGlobalConsensusNetworkLocation: Location = Location::new(
		1,
		[GlobalConsensus(PolkadotGlobalConsensusNetwork::get())]
	);
	/// Location of the PeoplePolkadot parachain, relative to this runtime.
	pub PeoplePolkadotLocation: Location = Location::new(1, [
		GlobalConsensus(BridgedNetwork::get()),
		Parachain(bp_people_polkadot::PEOPLE_POLKADOT_PARACHAIN_ID),
	]);

	/// A number of Polkadot mandatory headers that are accepted for free at every
	/// **this chain** block.
	pub const MaxFreePolkadotHeadersPerBlock: u32 = 4;
	/// A number of Polkadot header digests that we keep in the storage.
	pub const PolkadotHeadersToKeep: u32 = 1_200;
	/// A name of parachains pallet at Pokadot.
	pub const AtPolkadotParasPalletName: &'static str = "Paras";

	/// A number of People Polkadot head digests that we keep in the storage.
	pub const PeoplePolkadotHeadsToKeep: u32 = 600;
	/// A maximal size of Polkadot Bridge Hub head digest.
	pub const MaxPeoplePolkadotHeadSize: u32 = bp_polkadot::MAX_NESTED_PARACHAIN_HEAD_DATA_SIZE;
}

/// An instance of `pallet_bridge_grandpa` used to bridge with Polkadot.
pub type WithPolkadotBridgeGrandpaInstance = ();
impl pallet_bridge_grandpa::Config<WithPolkadotBridgeGrandpaInstance> for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = crate::weights::pallet_bridge_grandpa::WeightInfo<Runtime>;

	type BridgedChain = bp_polkadot::Polkadot;
	type MaxFreeHeadersPerBlock = MaxFreePolkadotHeadersPerBlock;
	type FreeHeadersInterval = ConstU32<5>;
	type HeadersToKeep = PolkadotHeadersToKeep;
}

/// An instance of `pallet_bridge_parachains` used to bridge with Polkadot.
pub type WithPolkadotBridgeParachainsInstance = ();
impl pallet_bridge_parachains::Config<WithPolkadotBridgeParachainsInstance> for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = crate::weights::pallet_bridge_parachains::WeightInfo<Runtime>;

	type BridgesGrandpaPalletInstance = WithPolkadotBridgeGrandpaInstance;
	type ParasPalletName = AtPolkadotParasPalletName;
	type ParaStoredHeaderDataBuilder =
		SingleParaStoredHeaderDataBuilder<bp_people_polkadot::PeoplePolkadot>;
	type HeadsToKeep = PeoplePolkadotHeadsToKeep;
	type MaxParaHeadDataSize = MaxPeoplePolkadotHeadSize;
	type OnNewHead = ();
}

const LOG_TARGET_BRIDGE_DISPATCH: &str = "runtime::bridge-dispatch";

/// Message dispatch result type for a single message.
#[derive(
	CloneNoBound,
	EqNoBound,
	PartialEqNoBound,
	Encode,
	Decode,
	DecodeWithMemTracking,
	Debug,
	TypeInfo,
)]
pub enum XcmBlobMessageDispatchResult {
	/// We've been unable to decode the message payload.
	InvalidPayload,
	/// Message has been dispatched.
	Dispatched,
	/// Message has **NOT** been dispatched because of a given error.
	NotDispatched(#[codec(skip)] Option<DispatchBlobError>),
}

pub struct XcmBlobMessageDispatch<DispatchBlob, Weights> {
	_marker: PhantomData<(DispatchBlob, Weights)>,
}

impl<BlobDispatcher: DispatchBlob, Weights: pallet_bridge_messages::WeightInfoExt> MessageDispatch
	for XcmBlobMessageDispatch<BlobDispatcher, Weights>
{
	type DispatchPayload = XcmAsPlainPayload;
	type DispatchLevelResult = XcmBlobMessageDispatchResult;
	type LaneId = LegacyLaneId;

	fn is_active(_lane: Self::LaneId) -> bool {
		true
	}

	fn dispatch_weight(
		message: &mut DispatchMessage<Self::DispatchPayload, Self::LaneId>,
	) -> Weight {
		match message.data.payload {
			Ok(ref payload) => {
				let payload_size = payload.encoded_size().saturated_into();
				Weights::message_dispatch_weight(payload_size)
			},
			Err(_) => Weight::zero(),
		}
	}

	fn dispatch(
		message: DispatchMessage<Self::DispatchPayload, Self::LaneId>,
	) -> MessageDispatchResult<Self::DispatchLevelResult> {
		let payload = match message.data.payload {
			Ok(payload) => payload,
			Err(e) => {
				log::error!(
					target: LOG_TARGET_BRIDGE_DISPATCH,
					"dispatch - payload error: {e:?} for lane_id: {:?} and message_nonce: {:?}",
					message.key.lane_id,
					message.key.nonce
				);
				return MessageDispatchResult {
					unspent_weight: Weight::zero(),
					dispatch_level_result: XcmBlobMessageDispatchResult::InvalidPayload,
				}
			},
		};
		let dispatch_level_result = match BlobDispatcher::dispatch_blob(payload) {
			Ok(_) => {
				log::debug!(
					target: LOG_TARGET_BRIDGE_DISPATCH,
					"dispatch - `DispatchBlob::dispatch_blob` was ok for lane_id: {:?} and message_nonce: {:?}",
					message.key.lane_id,
					message.key.nonce
				);
				XcmBlobMessageDispatchResult::Dispatched
			},
			Err(e) => {
				log::error!(
					target: LOG_TARGET_BRIDGE_DISPATCH,
					"dispatch - `DispatchBlob::dispatch_blob` failed with error: {e:?} for lane_id: {:?} and message_nonce: {:?}",
					message.key.lane_id,
					message.key.nonce
				);
				XcmBlobMessageDispatchResult::NotDispatched(Some(e))
			},
		};
		MessageDispatchResult { unspent_weight: Weight::zero(), dispatch_level_result }
	}
}

/// An instance of `pallet_bridge_messages` used to bridge with Polkadot Bridge Hub.
pub type WithPeoplePolkadotMessagesInstance = ();
impl pallet_bridge_messages::Config<WithPeoplePolkadotMessagesInstance> for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = crate::weights::pallet_bridge_messages::WeightInfo<Runtime>;

	type ThisChain = bp_polkadot_bulletin::PolkadotBulletin;
	type BridgedChain = bp_people_polkadot::PeoplePolkadot;
	type BridgedHeaderChain = pallet_bridge_parachains::ParachainHeaders<
		Runtime,
		WithPolkadotBridgeParachainsInstance,
		bp_people_polkadot::PeoplePolkadot,
	>;

	type OutboundPayload = XcmAsPlainPayload;
	type InboundPayload = XcmAsPlainPayload;
	type LaneId = LegacyLaneId;

	type DeliveryPayments = ();
	type DeliveryConfirmationPayments = ();

	type MessageDispatch = WithXcmWeightDispatcher<
		XcmBlobMessageDispatch<FromPeoplePolkadotBlobDispatcher, Self::WeightInfo>,
	>;
	type OnMessagesDelivered = ();
}

/// Message dispatcher that decodes XCM message and return its actual dispatch weight.
pub struct WithXcmWeightDispatcher<Inner>(PhantomData<Inner>);

impl<Inner> MessageDispatch for WithXcmWeightDispatcher<Inner>
where
	Inner: MessageDispatch<DispatchPayload = XcmAsPlainPayload, LaneId = LegacyLaneId>,
{
	type DispatchPayload = XcmAsPlainPayload;
	type DispatchLevelResult = Inner::DispatchLevelResult;
	type LaneId = LegacyLaneId;

	fn is_active(lane: Self::LaneId) -> bool {
		Inner::is_active(lane)
	}

	fn dispatch_weight(
		message: &mut DispatchMessage<Self::DispatchPayload, Self::LaneId>,
	) -> Weight {
		message
			.data
			.payload
			.as_ref()
			.map_err(drop)
			.and_then(|payload| decode_bridge_message(payload).map(|(_, xcm)| xcm).map_err(drop))
			.and_then(|xcm| xcm.try_into().map_err(drop))
			.and_then(|xcm| XcmExecutor::<XcmConfig>::prepare(xcm, Weight::MAX).map_err(drop))
			.map(|weighed_xcm| weighed_xcm.weight_of())
			.unwrap_or(Weight::zero())
	}

	fn dispatch(
		message: DispatchMessage<Self::DispatchPayload, Self::LaneId>,
	) -> MessageDispatchResult<Self::DispatchLevelResult> {
		let mut result = Inner::dispatch(message);
		// ensure that unspent is always zero here to avoid inconstency
		result.unspent_weight = Weight::zero();
		result
	}
}

/// Dispatches received XCM messages from the Polkadot Bridge Hub.
pub type FromPeoplePolkadotBlobDispatcher = crate::xcm_config::ImmediateXcmDispatcher;

pub struct XcmBlobHauler<Runtime, MessagesInstance> {
	_marker: PhantomData<(Runtime, MessagesInstance)>,
}

impl<Runtime, MessagesInstance: 'static> HaulBlob for XcmBlobHauler<Runtime, MessagesInstance>
where
	Runtime: pallet_bridge_messages::Config<
		MessagesInstance,
		LaneId = LegacyLaneId,
		OutboundPayload = XcmAsPlainPayload,
	>,
{
	fn haul_blob(blob: XcmAsPlainPayload) -> Result<(), HaulBlobError> {
		let send_message_args =
			pallet_bridge_messages::Pallet::<Runtime, MessagesInstance>::validate_message(
				XCM_LANE, &blob,
			)
			.map_err(|e| {
				log::error!(
					target: LOG_TARGET_BRIDGE_DISPATCH,
					"haul_blob result - error: {:?} on lane: {:?}",
					e,
					XCM_LANE,
				);
				HaulBlobError::Transport("MessageSenderError")
			})?;
		let artifacts = pallet_bridge_messages::Pallet::<Runtime, MessagesInstance>::send_message(
			send_message_args,
		);
		log::info!(
			target: LOG_TARGET_BRIDGE_DISPATCH,
			"haul_blob result - ok: {:?} on lane: {:?}. Enqueued messages: {}",
			artifacts.nonce,
			XCM_LANE,
			artifacts.enqueued_messages,
		);

		Ok(())
	}
}

/// Export XCM messages to be relayed to the Polkadot Bridge Hub chain.
pub type ToBridgeHaulBlobExporter = HaulBlobExporter<
	XcmBlobHauler<Runtime, WithPeoplePolkadotMessagesInstance>,
	PolkadotGlobalConsensusNetworkLocation,
	AlwaysV5,
	(),
>;

#[cfg(feature = "runtime-benchmarks")]
pub mod benchmarking {
	use super::*;

	/// Proof of messages, coming from PeoplePolkadot.
	pub type FromPeoplePolkadotMessagesProof =
		bp_messages::target_chain::FromBridgedChainMessagesProof<
			bp_people_polkadot::Hash,
			pallet_bridge_messages::LaneIdOf<Runtime, WithPeoplePolkadotMessagesInstance>,
		>;

	/// Message delivery proof for `PeoplePolkadot` messages.
	pub type ToPeoplePolkadotMessagesDeliveryProof =
		bp_messages::source_chain::FromBridgedChainMessagesDeliveryProof<
			bp_people_polkadot::Hash,
			pallet_bridge_messages::LaneIdOf<Runtime, WithPeoplePolkadotMessagesInstance>,
		>;
}

// TODO: enable tests as much as possible
#[cfg(test)]
pub(crate) mod tests {
	use super::*;
	// 	use crate::{
	// 		xcm_config::{
	// 			tests::{
	// 				encoded_xcm_message_from_people_polkadot,
	// 				encoded_xcm_message_from_people_polkadot_require_wight_at_most,
	// 			},
	// 			BaseXcmWeight,
	// 		},
	// 		BridgePolkadotGrandpa, BridgePolkadotMessages, BridgeRejectObsoleteHeadersAndMessages,
	// 		Executive, RuntimeCall, Signature, SignedExtra, SignedPayload, UncheckedExtrinsic,
	// 		ValidateSigned,
	// 	};
	// 	use bp_header_chain::{justification::GrandpaJustification, HeaderChain, InitializationData};
	// 	use bp_messages::{
	// 		target_chain::DispatchMessageData, DeliveredMessages, InboundLaneData, MessageKey,
	// 		OutboundLaneData, UnrewardedRelayer, UnrewardedRelayersState,
	// 	};
	// 	use bp_polkadot::parachains::{ParaHead, ParaHeadsProof};
	// 	use bp_runtime::{
	// 		record_all_trie_keys, BasicOperatingMode, HeaderIdProvider, Parachain, RawStorageProof,
	// 		StorageProofSize,
	// 	};
	// 	use bridge_runtime_common::{
	// 		assert_complete_bridge_types,
	// 		integrity::{
	// 			assert_complete_bridge_constants, check_message_lane_weights,
	// 			AssertBridgeMessagesPalletConstants, AssertBridgePalletNames, AssertChainConstants,
	// 			AssertCompleteBridgeConstants,
	// 		},
	// 		messages::{
	// 			source::FromBridgedChainMessagesDeliveryProof, target::FromBridgedChainMessagesProof,
	// 		},
	// 		messages_generation::{
	// 			encode_all_messages, encode_lane_data, prepare_messages_storage_proof,
	// 		},
	// 	};
	// 	use codec::Encode;
	// 	use frame_support::assert_ok;
	// 	use sp_api::HeaderT;
	// 	use sp_consensus_grandpa::{AuthorityList, SetId};
	use sp_keyring::Sr25519Keyring as AccountKeyring;
	use sp_runtime::{
		// 		generic::Era,
		// 		transaction_validity::{InvalidTransaction, TransactionValidityError},
		BuildStorage,
	};
	// 	use sp_trie::{trie_types::TrieDBMutBuilderV1, LayoutV1, MemoryDB, TrieMut};
	//
	// 	const POLKADOT_HEADER_NUMBER: bp_polkadot::BlockNumber = 100;
	// 	const people_hub_HEADER_NUMBER: bp_people_polkadot::BlockNumber = 200;
	//
	// 	#[derive(Clone, Copy)]
	// 	enum HeaderType {
	// 		WithMessages,
	// 		WithDeliveredMessages,
	// 	}
	//
	// 	fn relayer_account_at_polkadot() -> bp_polkadot::AccountId {
	// 		[42u8; 32].into()
	// 	}
	//
	// 	fn sudo_signer() -> AccountKeyring {
	// 		AccountKeyring::Alice
	// 	}
	//
	fn relayer_signer() -> AccountKeyring {
		AccountKeyring::Bob
	}
	//
	// 	fn non_relay_signer() -> AccountKeyring {
	// 		AccountKeyring::Charlie
	// 	}
	//
	// 	fn polkadot_initial_header() -> bp_polkadot::Header {
	// 		bp_test_utils::test_header(POLKADOT_HEADER_NUMBER - 1)
	// 	}
	//
	// 	fn polkadot_header(t: HeaderType) -> bp_polkadot::Header {
	// 		let people_polkadot_head_storage_proof = people_polkadot_head_storage_proof(t);
	// 		let state_root = people_polkadot_head_storage_proof.0;
	// 		bp_test_utils::test_header_with_root(POLKADOT_HEADER_NUMBER, state_root)
	// 	}
	//
	// 	fn polkadot_grandpa_justification(t: HeaderType) ->
	// GrandpaJustification<bp_polkadot::Header> { 		bp_test_utils::make_default_justification(&
	// polkadot_header(t)) 	}
	//
	// 	fn people_polkadot_header(t: HeaderType) -> bp_people_polkadot::Header {
	// 		bp_test_utils::test_header_with_root(
	// 			people_hub_HEADER_NUMBER,
	// 			match t {
	// 				HeaderType::WithMessages => people_polkadot_message_storage_proof().0,
	// 				HeaderType::WithDeliveredMessages =>
	// 					people_polkadot_message_delivery_storage_proof().0,
	// 			},
	// 		)
	// 	}
	//
	// 	fn people_polkadot_head_storage_proof(
	// 		t: HeaderType,
	// 	) -> (bp_polkadot::Hash, ParaHeadsProof) {
	// 		let (state_root, proof, _) =
	// 			bp_test_utils::prepare_parachain_heads_proof::<bp_polkadot::Header>(vec![(
	// 				BridgeHubPolkadotOrPolkadot::PARACHAIN_ID,
	// 				ParaHead(people_polkadot_header(t).encode()),
	// 			)]);
	// 		(state_root, proof)
	// 	}
	//
	// 	fn people_polkadot_message_storage_proof() -> (bp_people_polkadot::Hash,
	// RawStorageProof) 	{
	// 		prepare_messages_storage_proof::<WithBridgeHubPolkadotMessageBridge>(
	// 			XCM_LANE,
	// 			1..=1,
	// 			None,
	// 			StorageProofSize::Minimal(0),
	// 			vec![42],
	// 			encode_all_messages,
	// 			encode_lane_data,
	// 		)
	// 	}
	//
	// 	fn people_polkadot_message_proof(
	// 	) -> FromBridgedChainMessagesProof<bp_people_polkadot::Hash> {
	// 		let (_, storage_proof) = people_polkadot_message_storage_proof();
	// 		let bridged_header_hash = people_polkadot_header(HeaderType::WithMessages).hash();
	// 		FromBridgedChainMessagesProof {
	// 			bridged_header_hash,
	// 			storage_proof,
	// 			lane: XCM_LANE,
	// 			nonces_start: 1,
	// 			nonces_end: 1,
	// 		}
	// 	}
	//
	// 	fn people_polkadot_message_delivery_storage_proof(
	// 	) -> (bp_people_polkadot::Hash, RawStorageProof) {
	// 		let storage_key = bp_messages::storage_keys::inbound_lane_data_key(
	// 			WithBridgeHubPolkadotMessageBridge::BRIDGED_MESSAGES_PALLET_NAME,
	// 			&XCM_LANE,
	// 		)
	// 		.0;
	// 		let storage_value = InboundLaneData::<AccountId> {
	// 			relayers: vec![UnrewardedRelayer {
	// 				relayer: relayer_signer().into(),
	// 				messages: DeliveredMessages { begin: 1, end: 1 },
	// 			}]
	// 			.into(),
	// 			last_confirmed_nonce: 0,
	// 		}
	// 		.encode();
	// 		let mut root = Default::default();
	// 		let mut mdb = MemoryDB::default();
	// 		{
	// 			let mut trie =
	// 				TrieDBMutBuilderV1::<bp_people_polkadot::Hasher>::new(&mut mdb, &mut root)
	// 					.build();
	// 			trie.insert(&storage_key, &storage_value).unwrap();
	// 		}
	//
	// 		let storage_proof =
	// 			record_all_trie_keys::<LayoutV1<bp_people_polkadot::Hasher>, _>(&mdb, &root)
	// 				.unwrap();
	//
	// 		(root, storage_proof)
	// 	}
	//
	// 	fn people_polkadot_message_delivery_proof(
	// 	) -> FromBridgedChainMessagesDeliveryProof<bp_people_polkadot::Hash> {
	// 		let (_, storage_proof) = people_polkadot_message_delivery_storage_proof();
	// 		let bridged_header_hash =
	// 			people_polkadot_header(HeaderType::WithDeliveredMessages).hash();
	// 		FromBridgedChainMessagesDeliveryProof { bridged_header_hash, storage_proof, lane: XCM_LANE
	// } 	}
	//
	// 	fn polkadot_authority_set() -> AuthorityList {
	// 		bp_test_utils::authority_list()
	// 	}
	//
	// 	fn polkadot_authority_set_id() -> SetId {
	// 		1
	// 	}
	//
	// 	// normally we would simply use `RuntimeCall::dispatch` in tests, but we need to test
	// 	// signed extension here, so we need to generate full-scale transaction and dispatch
	// 	// it using `Executive`
	// 	fn construct_and_apply_extrinsic(
	// 		signer: AccountKeyring,
	// 		call: RuntimeCall,
	// 	) -> sp_runtime::ApplyExtrinsicResult {
	// 		let nonce = frame_system::Account::<Runtime>::get(AccountId::from(signer)).nonce;
	// 		let extra: SignedExtra = (
	// 			frame_system::CheckNonZeroSender::<Runtime>::new(),
	// 			frame_system::CheckSpecVersion::<Runtime>::new(),
	// 			frame_system::CheckTxVersion::<Runtime>::new(),
	// 			frame_system::CheckGenesis::<Runtime>::new(),
	// 			frame_system::CheckEra::<Runtime>::from(Era::immortal()),
	// 			frame_system::CheckNonce::<Runtime>::from(nonce),
	// 			frame_system::CheckWeight::<Runtime>::new(),
	// 			ValidateSigned,
	// 			BridgeRejectObsoleteHeadersAndMessages,
	// 		);
	// 		let payload = SignedPayload::new(call.clone(), extra.clone()).unwrap();
	// 		let signature = payload.using_encoded(|e| signer.sign(e));
	// 		Executive::apply_extrinsic(UncheckedExtrinsic::new_signed(
	// 			call,
	// 			AccountId::from(signer.public()).into(),
	// 			Signature::Sr25519(signature.clone()),
	// 			extra,
	// 		))
	// 	}
	//
	// 	fn assert_ok_ok(apply_result: sp_runtime::ApplyExtrinsicResult) {
	// 		assert_ok!(apply_result);
	// 		assert_ok!(apply_result.unwrap());
	// 	}
	//
	pub fn run_test<T>(test: impl FnOnce() -> T) -> T {
		let _ = sp_tracing::try_init_simple();
		let mut t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
		pallet_relayer_set::GenesisConfig::<Runtime> {
			initial_relayers: vec![relayer_signer().into()],
		}
		.assimilate_storage(&mut t)
		.unwrap();

		sp_io::TestExternalities::new(t).execute_with(test)
	}
	//
	// 	fn initialize_polkadot_grandpa_pallet() -> sp_runtime::ApplyExtrinsicResult {
	// 		construct_and_apply_extrinsic(
	// 			sudo_signer(),
	// 			RuntimeCall::Sudo(pallet_sudo::Call::sudo {
	// 				call: Box::new(RuntimeCall::BridgePolkadotGrandpa(
	// 					pallet_bridge_grandpa::Call::initialize {
	// 						init_data: InitializationData {
	// 							header: Box::new(polkadot_initial_header()),
	// 							authority_list: polkadot_authority_set(),
	// 							set_id: polkadot_authority_set_id(),
	// 							operating_mode: BasicOperatingMode::Normal,
	// 						},
	// 					},
	// 				)),
	// 			}),
	// 		)
	// 	}
	//
	// 	fn submit_polkadot_header(
	// 		signer: AccountKeyring,
	// 		t: HeaderType,
	// 	) -> sp_runtime::ApplyExtrinsicResult {
	// 		construct_and_apply_extrinsic(
	// 			signer,
	// 			RuntimeCall::BridgePolkadotGrandpa(
	// 				pallet_bridge_grandpa::Call::submit_finality_proof {
	// 					finality_target: Box::new(polkadot_header(t)),
	// 					justification: polkadot_grandpa_justification(t),
	// 				},
	// 			),
	// 		)
	// 	}
	//
	// 	fn submit_polkadot_people_hub_header(
	// 		signer: AccountKeyring,
	// 		t: HeaderType,
	// 	) -> sp_runtime::ApplyExtrinsicResult {
	// 		construct_and_apply_extrinsic(
	// 			signer,
	// 			RuntimeCall::BridgePolkadotParachains(
	// 				pallet_bridge_parachains::Call::submit_parachain_heads {
	// 					at_relay_block: (POLKADOT_HEADER_NUMBER, polkadot_header(t).hash()),
	// 					parachains: vec![(
	// 						BridgeHubPolkadotOrPolkadot::PARACHAIN_ID.into(),
	// 						people_polkadot_header(t).hash(),
	// 					)],
	// 					parachain_heads_proof: people_polkadot_head_storage_proof(t).1,
	// 				},
	// 			),
	// 		)
	// 	}
	//
	// 	fn submit_messages_from_polkadot_bridge_hub(
	// 		signer: AccountKeyring,
	// 	) -> sp_runtime::ApplyExtrinsicResult {
	// 		construct_and_apply_extrinsic(
	// 			signer,
	// 			RuntimeCall::BridgePolkadotMessages(
	// 				pallet_bridge_messages::Call::receive_messages_proof {
	// 					relayer_id_at_bridged_chain: relayer_account_at_polkadot(),
	// 					proof: people_polkadot_message_proof(),
	// 					messages_count: 1,
	// 					dispatch_weight: Weight::zero(),
	// 				},
	// 			),
	// 		)
	// 	}
	//
	// 	fn submit_confirmations_from_polkadot_bridge_hub(
	// 		signer: AccountKeyring,
	// 	) -> sp_runtime::ApplyExtrinsicResult {
	// 		construct_and_apply_extrinsic(
	// 			signer,
	// 			RuntimeCall::BridgePolkadotMessages(
	// 				pallet_bridge_messages::Call::receive_messages_delivery_proof {
	// 					proof: people_polkadot_message_delivery_proof(),
	// 					relayers_state: UnrewardedRelayersState {
	// 						unrewarded_relayer_entries: 1,
	// 						messages_in_oldest_entry: 1,
	// 						total_messages: 1,
	// 						last_delivered_nonce: 1,
	// 					},
	// 				},
	// 			),
	// 		)
	// 	}
	//
	// 	fn emulate_sent_messages() {
	// 		pallet_bridge_messages::OutboundLanes::<Runtime,
	// WithBridgeHubPolkadotMessagesInstance>::insert( 			XCM_LANE,
	// 			OutboundLaneData {
	// 				oldest_unpruned_nonce: 1,
	// 				latest_received_nonce: 0,
	// 				latest_generated_nonce: 1,
	// 			},
	// 		);
	// 	}
	//
	// 	#[test]
	// 	fn may_initialize_grandpa_pallet_using_sudo() {
	// 		run_test(|| {
	// 			assert_eq!(BridgePolkadotGrandpa::best_finalized(), None);
	// 			assert_ok_ok(initialize_polkadot_grandpa_pallet());
	// 			assert_eq!(
	// 				BridgePolkadotGrandpa::best_finalized(),
	// 				Some(polkadot_initial_header().id())
	// 			);
	// 		});
	// 	}
	//
	// 	#[test]
	// 	fn only_relayer_may_submit_polkadot_headers() {
	// 		run_test(|| {
	// 			assert_ok_ok(initialize_polkadot_grandpa_pallet());
	//
	// 			assert_eq!(
	// 				BridgePolkadotGrandpa::best_finalized(),
	// 				Some(polkadot_initial_header().id())
	// 			);
	//
	// 			// Non-relayer may not submit Polkadot headers
	// 			// can't use assert_noop here, because we need to mutate storage inside
	// 			// the `construct_and_apply_extrinsic`
	// 			assert_eq!(
	// 				submit_polkadot_header(non_relay_signer(), HeaderType::WithMessages),
	// 				Err(TransactionValidityError::Invalid(InvalidTransaction::BadSigner))
	// 			);
	// 			assert_eq!(
	// 				BridgePolkadotGrandpa::best_finalized(),
	// 				Some(polkadot_initial_header().id())
	// 			);
	//
	// 			// Relayer may submit Polkadot headers
	// 			assert_ok_ok(submit_polkadot_header(relayer_signer(), HeaderType::WithMessages));
	// 			assert_eq!(
	// 				BridgePolkadotGrandpa::best_finalized(),
	// 				Some(polkadot_header(HeaderType::WithMessages).id())
	// 			);
	// 		});
	// 	}
	//
	// 	#[test]
	// 	fn only_relayer_may_submit_polkadot_people_hub_headers() {
	// 		run_test(|| {
	// 			assert_ok_ok(initialize_polkadot_grandpa_pallet());
	// 			assert_ok_ok(submit_polkadot_header(relayer_signer(), HeaderType::WithMessages));
	//
	// 			assert_eq!(
	// 				BridgeHubPolkadotHeadersProvider::finalized_header_state_root(
	// 					people_polkadot_header(HeaderType::WithMessages).hash()
	// 				),
	// 				None,
	// 			);
	//
	// 			// Non-relayer may NOT submit Polkadot BH headers
	// 			// can't use assert_noop here, because we need to mutate storage inside
	// 			// the `construct_and_apply_extrinsic`
	// 			assert_eq!(
	// 				submit_polkadot_people_hub_header(non_relay_signer(), HeaderType::WithMessages),
	// 				Err(TransactionValidityError::Invalid(InvalidTransaction::BadSigner)),
	// 			);
	// 			assert_eq!(
	// 				BridgeHubPolkadotHeadersProvider::finalized_header_state_root(
	// 					people_polkadot_header(HeaderType::WithMessages).hash()
	// 				),
	// 				None
	// 			);
	//
	// 			// Relayer may submit Polkadot BH headers
	// 			assert_ok_ok(submit_polkadot_people_hub_header(
	// 				relayer_signer(),
	// 				HeaderType::WithMessages,
	// 			));
	// 			assert_eq!(
	// 				BridgeHubPolkadotHeadersProvider::finalized_header_state_root(
	// 					people_polkadot_header(HeaderType::WithMessages).hash()
	// 				),
	// 				Some(*people_polkadot_header(HeaderType::WithMessages).state_root())
	// 			);
	// 		});
	// 	}
	//
	// 	#[test]
	// 	fn only_relayer_may_deliver_messages_from_polkadot_bridge_hub() {
	// 		run_test(|| {
	// 			assert_ok_ok(initialize_polkadot_grandpa_pallet());
	// 			assert_ok_ok(submit_polkadot_header(relayer_signer(), HeaderType::WithMessages));
	// 			assert_ok_ok(submit_polkadot_people_hub_header(
	// 				relayer_signer(),
	// 				HeaderType::WithMessages,
	// 			));
	//
	// 			assert!(BridgePolkadotMessages::inbound_lane_data(XCM_LANE).relayers.is_empty());
	//
	// 			// Non-relayer may NOT deliver messages from Polkadot BH
	// 			assert_eq!(
	// 				submit_messages_from_polkadot_bridge_hub(non_relay_signer()),
	// 				Err(TransactionValidityError::Invalid(InvalidTransaction::BadSigner)),
	// 			);
	// 			assert!(BridgePolkadotMessages::inbound_lane_data(XCM_LANE).relayers.is_empty());
	//
	// 			// Relayer may deliver messages from Polkadot BH
	// 			assert_ok_ok(submit_messages_from_polkadot_bridge_hub(relayer_signer()));
	// 			assert!(!BridgePolkadotMessages::inbound_lane_data(XCM_LANE).relayers.is_empty());
	// 		});
	// 	}
	//
	// 	#[test]
	// 	fn only_relayer_may_deliver_confirmations_from_polkadot_bridge_hub() {
	// 		run_test(|| {
	// 			assert_ok_ok(initialize_polkadot_grandpa_pallet());
	// 			assert_ok_ok(submit_polkadot_header(
	// 				relayer_signer(),
	// 				HeaderType::WithDeliveredMessages,
	// 			));
	// 			assert_ok_ok(submit_polkadot_people_hub_header(
	// 				relayer_signer(),
	// 				HeaderType::WithDeliveredMessages,
	// 			));
	// 			emulate_sent_messages();
	//
	// 			assert_eq!(
	// 				BridgePolkadotMessages::outbound_lane_data(XCM_LANE).latest_received_nonce,
	// 				0
	// 			);
	//
	// 			// Non-relayer may NOT deliver confirmations from Polkadot BH
	// 			assert_eq!(
	// 				submit_confirmations_from_polkadot_bridge_hub(non_relay_signer()),
	// 				Err(TransactionValidityError::Invalid(InvalidTransaction::BadSigner)),
	// 			);
	// 			assert_eq!(
	// 				BridgePolkadotMessages::outbound_lane_data(XCM_LANE).latest_received_nonce,
	// 				0
	// 			);
	//
	// 			// Relayer may deliver confirmations from Polkadot BH
	// 			assert_ok_ok(submit_confirmations_from_polkadot_bridge_hub(relayer_signer()));
	// 			assert_ne!(
	// 				BridgePolkadotMessages::outbound_lane_data(XCM_LANE).latest_received_nonce,
	// 				0
	// 			);
	// 		});
	// 	}
	//
	// 	#[test]
	// 	fn ensure_lane_weights_are_correct() {
	// 		check_message_lane_weights::<
	// 			bp_polkadot_bulletin::PolkadotBulletin,
	// 			Runtime,
	// 			WithBridgeHubPolkadotMessagesInstance,
	// 		>(
	// 			bp_people_polkadot::EXTRA_STORAGE_PROOF_SIZE,
	// 			bp_polkadot_bulletin::MAX_UNREWARDED_RELAYERS_IN_CONFIRMATION_TX,
	// 			bp_polkadot_bulletin::MAX_UNCONFIRMED_MESSAGES_IN_CONFIRMATION_TX,
	// 			false,
	// 		);
	// 	}
	//
	// 	#[test]
	// 	fn ensure_bridge_integrity() {
	// 		assert_complete_bridge_types!(
	// 			runtime: Runtime,
	// 			with_bridged_chain_grandpa_instance: WithPolkadotBridgeGrandpaInstance,
	// 			with_bridged_chain_messages_instance: WithBridgeHubPolkadotMessagesInstance,
	// 			bridge: WithBridgeHubPolkadotMessageBridge,
	// 			this_chain: bp_polkadot_bulletin::PolkadotBulletin,
	// 			bridged_chain: bp_polkadot::Polkadot,
	// 		);
	//
	// 		assert_complete_bridge_constants::<
	// 			Runtime,
	// 			WithPolkadotBridgeGrandpaInstance,
	// 			WithBridgeHubPolkadotMessagesInstance,
	// 			WithBridgeHubPolkadotMessageBridge,
	// 		>(AssertCompleteBridgeConstants {
	// 			this_chain_constants: AssertChainConstants {
	// 				block_length: bp_polkadot_bulletin::BlockLength::get(),
	// 				block_weights: bp_polkadot_bulletin::BlockWeights::get(),
	// 			},
	// 			messages_pallet_constants: AssertBridgeMessagesPalletConstants {
	// 				max_unrewarded_relayers_in_bridged_confirmation_tx:
	// 					bp_people_polkadot::MAX_UNREWARDED_RELAYERS_IN_CONFIRMATION_TX,
	// 				max_unconfirmed_messages_in_bridged_confirmation_tx:
	// 					bp_people_polkadot::MAX_UNCONFIRMED_MESSAGES_IN_CONFIRMATION_TX,
	// 				bridged_chain_id: bp_runtime::PEOPLE_POLKADOT_CHAIN_ID,
	// 			},
	// 			pallet_names: AssertBridgePalletNames {
	// 				with_this_chain_messages_pallet_name:
	// 					bp_polkadot_bulletin::WITH_POLKADOT_BULLETIN_MESSAGES_PALLET_NAME,
	// 				with_bridged_chain_grandpa_pallet_name:
	// 					bp_polkadot::WITH_POLKADOT_GRANDPA_PALLET_NAME,
	// 				with_bridged_chain_messages_pallet_name:
	// 					bp_people_polkadot::WITH_PEOPLE_POLKADOT_MESSAGES_PALLET_NAME,
	// 			},
	// 		});
	// 	}
	//
	// 	#[test]
	// 	fn dispatch_weight_of_inbound_message_is_correct() {
	// 		run_test(|| {
	// 			assert_eq!(
	// 				<Runtime as pallet_bridge_messages::Config<
	// 					WithBridgeHubPolkadotMessagesInstance,
	// 				>>::MessageDispatch::dispatch_weight(&mut DispatchMessage {
	// 					key: MessageKey { lane_id: XCM_LANE, nonce: 1 },
	// 					data: DispatchMessageData {
	// 						payload: Ok(encoded_xcm_message_from_people_polkadot())
	// 					},
	// 				}),
	// 				encoded_xcm_message_from_people_polkadot_require_wight_at_most()
	// 					.saturating_add(BaseXcmWeight::get())
	// 			);
	// 		});
	// 	}
}
