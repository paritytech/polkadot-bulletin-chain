#![cfg_attr(not(feature = "std"), no_std)]
// `construct_runtime!` does a lot of recursion and requires us to increase the limit to 256.
#![recursion_limit = "256"]

extern crate alloc;

// Make the WASM binary available.
#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

use bp_runtime::OwnedBridgeModule;
use bridge_runtime_common::generate_bridge_reject_obsolete_headers_and_messages;
use frame_support::{
	derive_impl,
	traits::{InstanceFilter, ValidatorRegistration},
};
use frame_system::EnsureRoot;
use pallet_bridge_grandpa::Call as BridgeGrandpaCall;
use pallet_bridge_messages::Call as BridgeMessagesCall;
use pallet_bridge_parachains::Call as BridgeParachainsCall;
use pallet_grandpa::AuthorityId as GrandpaId;
use pallet_session::Call as SessionCall;
use sp_api::impl_runtime_apis;
use sp_core::{crypto::KeyTypeId, OpaqueMetadata};
use sp_runtime::{
	generic, impl_opaque_keys,
	traits::{
		AccountIdLookup, AsSystemOriginSigner, BlakeTwo256, Block as BlockT, ConvertInto,
		DispatchInfoOf, IdentifyAccount, Implication, NumberFor, OpaqueKeys, PostDispatchInfoOf,
		TransactionExtension, Verify,
	},
	transaction_validity::{
		InvalidTransaction, TransactionLongevity, TransactionPriority, TransactionSource,
		TransactionValidity, TransactionValidityError, ValidTransaction,
	},
	ApplyExtrinsicResult, DispatchResult, MultiSignature,
};
use sp_std::prelude::*;
#[cfg(feature = "std")]
use sp_version::NativeVersion;
use sp_version::RuntimeVersion;

pub use frame_support::{
	construct_runtime, parameter_types,
	traits::{
		ConstBool, ConstU128, ConstU32, ConstU64, ConstU8, Get, KeyOwnerProofSystem, Randomness,
		StorageInfo,
	},
	weights::{
		constants::{
			BlockExecutionWeight, ExtrinsicBaseWeight, RocksDbWeight, WEIGHT_REF_TIME_PER_SECOND,
		},
		IdentityFee, Weight,
	},
	StorageValue,
};
use frame_support::{
	dispatch::GetDispatchInfo,
	genesis_builder_helper::{build_state, get_preset},
};
pub use frame_system::Call as SystemCall;
pub use pallet_timestamp::Call as TimestampCall;

#[cfg(any(feature = "std", test))]
pub use sp_runtime::BuildStorage;
pub use sp_runtime::{Perbill, Permill};

pub mod polkadot_bridge_config;
pub use polkadot_bridge_config::{self as bridge_config, bp_people_polkadot, bp_polkadot};

mod genesis_config_presets;
mod weights;
mod xcm_config;

/// An index to a block.
pub type BlockNumber = u32;

/// Alias to 512-bit hash when used in the context of a transaction signature on the chain.
pub type Signature = MultiSignature;

/// Some way of identifying an account on the chain. We intentionally make it equivalent
/// to the public key of our transaction signing scheme.
pub type AccountId = <<Signature as Verify>::Signer as IdentifyAccount>::AccountId;

/// Index of a transaction in the chain.
pub type Nonce = u32;

/// A hash of some data used by the chain.
pub type Hash = sp_core::H256;

/// Opaque types. These are used by the CLI to instantiate machinery that don't need to know
/// the specifics of the runtime. They can then be made to be agnostic over specific formats
/// of data like extrinsics, allowing for them to continue syncing the network through upgrades
/// to even the core data structures.
pub mod opaque {
	use super::*;

	pub use sp_runtime::OpaqueExtrinsic as UncheckedExtrinsic;

	/// Opaque block header type.
	pub type Header = generic::Header<BlockNumber, BlakeTwo256>;
	/// Opaque block type.
	pub type Block = generic::Block<Header, UncheckedExtrinsic>;
	/// Opaque block identifier type.
	pub type BlockId = generic::BlockId<Block>;

	impl_opaque_keys! {
		pub struct SessionKeys {
			pub babe: Babe,
			pub grandpa: Grandpa,
		}
	}
}

// To learn more about runtime versioning, see:
// https://docs.substrate.io/main-docs/build/upgrade#runtime-versioning
#[sp_version::runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
	spec_name: alloc::borrow::Cow::Borrowed("bulletin-polkadot"),
	impl_name: alloc::borrow::Cow::Borrowed("bulletin-polkadot"),
	authoring_version: 0,
	spec_version: 1_000_000,
	impl_version: 1,
	apis: RUNTIME_API_VERSIONS,
	transaction_version: 1,
	system_version: 1,
};

/// This determines the average expected block time that we are targeting.
/// Blocks will be produced at a minimum duration defined by `SLOT_DURATION`.
///
/// Change this to adjust the block time.
pub const MILLISECS_PER_BLOCK: u64 = 6000;

// NOTE: Currently it is not possible to change the slot duration after the chain has started.
//       Attempting to do so will brick block production.
pub const SLOT_DURATION: u64 = MILLISECS_PER_BLOCK;

// 1 in 4 blocks (on average, not counting collisions) will be primary BABE blocks.
pub const PRIMARY_PROBABILITY: (u64, u64) = (1, 4);

/// The BABE epoch configuration at genesis.
pub const BABE_GENESIS_EPOCH_CONFIG: sp_consensus_babe::BabeEpochConfiguration =
	sp_consensus_babe::BabeEpochConfiguration {
		c: PRIMARY_PROBABILITY,
		allowed_slots: sp_consensus_babe::AllowedSlots::PrimaryAndSecondaryPlainSlots,
	};

// NOTE: Currently it is not possible to change the epoch duration after the chain has started.
//       Attempting to do so will brick block production.
pub const EPOCH_DURATION_IN_BLOCKS: BlockNumber = HOURS;
pub const EPOCH_DURATION_IN_SLOTS: u64 = {
	const SLOT_FILL_RATE: f64 = MILLISECS_PER_BLOCK as f64 / SLOT_DURATION as f64;
	(EPOCH_DURATION_IN_BLOCKS as f64 * SLOT_FILL_RATE) as u64
};

// Time is measured by number of blocks.
pub const MINUTES: BlockNumber = 60_000 / (MILLISECS_PER_BLOCK as BlockNumber);
pub const HOURS: BlockNumber = MINUTES * 60;
pub const DAYS: BlockNumber = HOURS * 24;

/// The version information used to identify this runtime when compiled natively.
#[cfg(feature = "std")]
pub fn native_version() -> NativeVersion {
	NativeVersion { runtime_version: VERSION, can_author_with: Default::default() }
}

// There are fewer system operations on this chain (e.g. staking, governance, etc.). Use a higher
// percentage of the block for data storage.
const NORMAL_DISPATCH_RATIO: Perbill = Perbill::from_percent(90);

parameter_types! {
	pub const BlockHashCount: BlockNumber = 2400;
	pub const Version: RuntimeVersion = VERSION;
	/// We allow for 2 seconds of compute with a 6 second average block time.
	pub BlockWeights: frame_system::limits::BlockWeights =
		frame_system::limits::BlockWeights::with_sensible_defaults(
			Weight::from_parts(2u64 * WEIGHT_REF_TIME_PER_SECOND, u64::MAX),
			NORMAL_DISPATCH_RATIO,
		);
	// Note: Max transaction size is 8 MB. Set max block size to 10 MB to facilitate data storage.
	// This is double the "normal" Relay Chain block length limit.
	pub BlockLength: frame_system::limits::BlockLength = frame_system::limits::BlockLength
		::max_with_normal_ratio(10 * 1024 * 1024, NORMAL_DISPATCH_RATIO);
	// Let's use substrate one: https://github.com/paritytech/ss58-registry/blob/main/ss58-registry.json
	// (Note: Possibly we can add new one.)
	pub const SS58Prefix: u8 = 42;

	pub const MaxAuthorities: u32 = 100;

	pub const EquivocationReportPeriodInEpochs: u64 = 168;
	pub const EquivocationReportPeriodInBlocks: u64 =
		EquivocationReportPeriodInEpochs::get() * (EPOCH_DURATION_IN_BLOCKS as u64);


	// This currently _must_ be set to DEFAULT_STORAGE_PERIOD
	pub const StoragePeriod: BlockNumber = sp_transaction_storage_proof::DEFAULT_STORAGE_PERIOD;
	pub const AuthorizationPeriod: BlockNumber = 7 * DAYS;
	pub const StoreRenewPriority: TransactionPriority = RemoveExpiredAuthorizationPriority::get() - 1;
	pub const StoreRenewLongevity: TransactionLongevity = DAYS as TransactionLongevity;
	pub const RemoveExpiredAuthorizationPriority: TransactionPriority = SetPurgeKeysPriority::get() - 1;
	pub const RemoveExpiredAuthorizationLongevity: TransactionLongevity = DAYS as TransactionLongevity;

	pub const SudoPriority: TransactionPriority = TransactionPriority::MAX;

	pub const SetKeysCooldownBlocks: BlockNumber = 5 * MINUTES;
	pub const SetPurgeKeysPriority: TransactionPriority = SudoPriority::get() - 1;
	pub const SetPurgeKeysLongevity: TransactionLongevity = HOURS as TransactionLongevity;

	pub const BridgeTxFailCooldownBlocks: BlockNumber = 5 * MINUTES;
	pub const BridgeTxPriority: TransactionPriority = StoreRenewPriority::get() - 1;
	pub const BridgeTxLongevity: TransactionLongevity = HOURS as TransactionLongevity;
}

// Configure FRAME pallets to include in runtime.

#[derive_impl(frame_system::config_preludes::SolochainDefaultConfig)]
impl frame_system::Config for Runtime {
	/// The block type for the runtime.
	type Block = Block;
	/// Block & extrinsics weights: base values and limits.
	type BlockWeights = BlockWeights;
	/// The maximum length of a block (in bytes).
	type BlockLength = BlockLength;
	/// The identifier used to distinguish between accounts.
	type AccountId = AccountId;
	/// The aggregated dispatch type that is available for extrinsics.
	type RuntimeCall = RuntimeCall;
	/// The lookup mechanism to get account ID from whatever is passed in dispatchers.
	type Lookup = AccountIdLookup<AccountId, ()>;
	/// The type for storing how many extrinsics an account has signed.
	type Nonce = Nonce;
	/// The type for hashing blocks and tries.
	type Hash = Hash;
	/// The hashing algorithm used.
	type Hashing = BlakeTwo256;
	/// The ubiquitous event type.
	type RuntimeEvent = RuntimeEvent;
	/// The ubiquitous origin type.
	type RuntimeOrigin = RuntimeOrigin;
	/// Maximum number of block number to block hash mappings to keep (oldest pruned first).
	type BlockHashCount = BlockHashCount;
	/// The weight of database operations that the runtime can invoke.
	type DbWeight = RocksDbWeight;
	/// Version of the runtime.
	type Version = Version;
	/// Converts a module to the index of the module in `construct_runtime!`.
	///
	/// This type is being generated by `construct_runtime!`.
	type PalletInfo = PalletInfo;
	/// This is used as an identifier of the chain. 42 is the generic substrate prefix.
	type SS58Prefix = SS58Prefix;
	type MaxConsumers = frame_support::traits::ConstU32<16>;
	/// Weight information for the extrinsics of this pallet.
	type SystemWeightInfo = weights::frame_system::WeightInfo<Runtime>;
	type ExtensionsWeightInfo = weights::frame_system_extensions::WeightInfo<Runtime>;
}

impl pallet_validator_set::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = weights::pallet_validator_set::WeightInfo<Runtime>;
	type AddRemoveOrigin = EnsureRoot<AccountId>;
	type MaxAuthorities = MaxAuthorities;
	type SetKeysCooldownBlocks = SetKeysCooldownBlocks;
}

impl pallet_session::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type ValidatorId = AccountId;
	type ValidatorIdOf = ConvertInto;
	type ShouldEndSession = Babe;
	type NextSessionRotation = Babe;
	type SessionManager = ValidatorSet;
	type SessionHandler = <opaque::SessionKeys as OpaqueKeys>::KeyTypeIdProviders;
	type Keys = opaque::SessionKeys;
	type WeightInfo = pallet_session::weights::SubstrateWeight<Runtime>;
	type Currency = pallets_common::NoCurrency<AccountId, RuntimeHoldReason>;
	type KeyDeposit = ();
	// TODO: nothing for now, maybe in the future.
	type DisablingStrategy = ();
}

impl pallet_session::historical::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type FullIdentification = Self::ValidatorId;
	type FullIdentificationOf = Self::ValidatorIdOf;
}

impl pallet_babe::Config for Runtime {
	type EpochDuration = ConstU64<EPOCH_DURATION_IN_SLOTS>;
	type ExpectedBlockTime = ConstU64<MILLISECS_PER_BLOCK>;
	type EpochChangeTrigger = pallet_babe::ExternalTrigger;
	type DisabledValidators = Session;

	type WeightInfo = ();
	type MaxAuthorities = MaxAuthorities;
	type MaxNominators = ConstU32<0>;
	type KeyOwnerProof =
		<Historical as KeyOwnerProofSystem<(KeyTypeId, pallet_babe::AuthorityId)>>::Proof;
	type EquivocationReportSystem = pallet_babe::EquivocationReportSystem<
		Self,
		Offences,
		Historical,
		EquivocationReportPeriodInBlocks,
	>;
}

impl pallet_grandpa::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;

	type WeightInfo = ();
	type MaxAuthorities = MaxAuthorities;
	type MaxNominators = ConstU32<0>;
	type MaxSetIdSessionEntries = EquivocationReportPeriodInEpochs;

	type KeyOwnerProof = <Historical as KeyOwnerProofSystem<(KeyTypeId, GrandpaId)>>::Proof;
	type EquivocationReportSystem = pallet_grandpa::EquivocationReportSystem<
		Self,
		Offences,
		Historical,
		EquivocationReportPeriodInBlocks,
	>;
}

impl pallet_offences::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type IdentificationTuple = pallet_session::historical::IdentificationTuple<Self>;
	type OnOffenceHandler = ValidatorSet;
}

impl pallet_authorship::Config for Runtime {
	type FindAuthor = pallet_session::FindAccountFromAuthorIndex<Self, Babe>;
	type EventHandler = ();
}

impl pallet_timestamp::Config for Runtime {
	/// A timestamp: milliseconds since the unix epoch.
	type Moment = u64;
	type OnTimestampSet = Babe;
	type MinimumPeriod = ConstU64<{ SLOT_DURATION / 2 }>;
	type WeightInfo = weights::pallet_timestamp::WeightInfo<Runtime>;
}

impl pallet_transaction_storage::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = weights::pallet_transaction_storage::WeightInfo<Runtime>;
	type MaxBlockTransactions = ConstU32<512>;
	/// Max transaction size per block needs to be aligned with [`BlockLength`].
	type MaxTransactionSize = ConstU32<{ 8 * 1024 * 1024 }>;
	type StoragePeriod = StoragePeriod;
	type AuthorizationPeriod = AuthorizationPeriod;
	type Authorizer = EnsureRoot<Self::AccountId>;
	type StoreRenewPriority = StoreRenewPriority;
	type StoreRenewLongevity = StoreRenewLongevity;
	type RemoveExpiredAuthorizationPriority = RemoveExpiredAuthorizationPriority;
	type RemoveExpiredAuthorizationLongevity = RemoveExpiredAuthorizationLongevity;
}

impl pallet_relayer_set::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = weights::pallet_relayer_set::WeightInfo<Runtime>;
	type AddRemoveOrigin = EnsureRoot<AccountId>;
	type BridgeTxFailCooldownBlocks = BridgeTxFailCooldownBlocks;
}

impl pallet_sudo::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type WeightInfo = weights::pallet_sudo::WeightInfo<Runtime>;
}

#[derive(
	Copy,
	Clone,
	Eq,
	PartialEq,
	Ord,
	PartialOrd,
	codec::Encode,
	codec::Decode,
	codec::DecodeWithMemTracking,
	sp_runtime::RuntimeDebug,
	codec::MaxEncodedLen,
	scale_info::TypeInfo,
)]
pub enum ProxyType {
	/// Fully permissioned proxy. Can execute any call on behalf of _proxied_.
	Any,
}
impl Default for ProxyType {
	fn default() -> Self {
		Self::Any
	}
}
impl InstanceFilter<RuntimeCall> for ProxyType {
	fn filter(&self, _c: &RuntimeCall) -> bool {
		true
	}

	fn is_superset(&self, _o: &Self) -> bool {
		true
	}
}

impl pallet_proxy::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type Currency = pallets_common::NoCurrency<AccountId>;
	type ProxyType = ProxyType;
	type ProxyDepositBase = ();
	type ProxyDepositFactor = ();
	type MaxProxies = ConstU32<16>;
	type WeightInfo = weights::pallet_proxy::WeightInfo<Runtime>;
	type MaxPending = ConstU32<0>;
	type CallHasher = BlakeTwo256;
	type AnnouncementDepositBase = ();
	type AnnouncementDepositFactor = ();
	type BlockNumberProvider = frame_system::Pallet<Runtime>;
}

impl<C> frame_system::offchain::CreateTransactionBase<C> for Runtime
where
	RuntimeCall: From<C>,
{
	type Extrinsic = UncheckedExtrinsic;
	type RuntimeCall = RuntimeCall;
}

impl<C> frame_system::offchain::CreateBare<C> for Runtime
where
	RuntimeCall: From<C>,
{
	fn create_bare(call: RuntimeCall) -> UncheckedExtrinsic {
		UncheckedExtrinsic::new_bare(call)
	}
}

construct_runtime!(
	pub struct Runtime {
		System: frame_system = 0,
		// Babe must be called before Session
		Babe: pallet_babe = 1,
		Timestamp: pallet_timestamp = 2,
		// Authorship must be before session in order to note author in the correct session.
		Authorship: pallet_authorship = 10,
		Offences: pallet_offences = 11,
		Historical: pallet_session::historical = 12,
		ValidatorSet: pallet_validator_set = 13,
		Session: pallet_session = 14,
		Grandpa: pallet_grandpa = 15,

		// Storage
		TransactionStorage: pallet_transaction_storage = 40,

		// Bridge
		RelayerSet: pallet_relayer_set = 50,
		BridgePolkadotGrandpa: pallet_bridge_grandpa = 51,
		BridgePolkadotParachains: pallet_bridge_parachains = 52,
		BridgePolkadotMessages: pallet_bridge_messages = 53,

		// Local Root
		Sudo: pallet_sudo = 61,
		Proxy: pallet_proxy = 62,
	}
);

/// The address format for describing accounts.
pub type Address = sp_runtime::MultiAddress<AccountId, ()>;
/// Block header type as expected by this runtime.
pub type Header = generic::Header<BlockNumber, BlakeTwo256>;
/// Block type as expected by this runtime.
pub type Block = generic::Block<Header, UncheckedExtrinsic>;

fn validate_purge_keys(who: &AccountId) -> TransactionValidity {
	// Only allow if account has keys to purge
	if Session::is_registered(who) {
		Ok(ValidTransaction {
			priority: SetPurgeKeysPriority::get(),
			longevity: SetPurgeKeysLongevity::get(),
			..Default::default()
		})
	} else {
		Err(InvalidTransaction::BadSigner.into())
	}
}

/// `ValidateUnsigned` equivalent for signed transactions.
///
/// This chain has no transaction fees, so we require checks equivalent to those performed by
/// `ValidateUnsigned` for all signed transactions.
#[derive(
	Clone,
	PartialEq,
	Eq,
	sp_runtime::RuntimeDebug,
	codec::Encode,
	codec::Decode,
	codec::DecodeWithMemTracking,
	scale_info::TypeInfo,
)]
pub struct ValidateSigned;

impl TransactionExtension<RuntimeCall> for ValidateSigned {
	const IDENTIFIER: &'static str = "ValidateSigned";

	type Implicit = ();
	fn implicit(&self) -> Result<Self::Implicit, TransactionValidityError> {
		Ok(())
	}

	type Val = ();
	/// `Some(who)` if the transaction is a bridge transaction.
	type Pre = Option<AccountId>;

	fn weight(&self, _call: &RuntimeCall) -> Weight {
		Weight::zero()
	}

	fn validate(
		&self,
		origin: RuntimeOrigin,
		call: &RuntimeCall,
		_info: &DispatchInfoOf<RuntimeCall>,
		_len: usize,
		_self_implicit: Self::Implicit,
		_inherited_implication: &impl Implication,
		_source: TransactionSource,
	) -> sp_runtime::traits::ValidateResult<Self::Val, RuntimeCall> {
		let who = origin.as_system_origin_signer().ok_or(InvalidTransaction::BadSigner)?;

		let validity = match call {
			// Transaction storage call
			RuntimeCall::TransactionStorage(inner_call) =>
				TransactionStorage::validate_signed(who, inner_call),

			// Session key management
			RuntimeCall::Session(SessionCall::set_keys { .. }) =>
				ValidatorSet::validate_set_keys(who).map(|()| ValidTransaction {
					priority: SetPurgeKeysPriority::get(),
					longevity: SetPurgeKeysLongevity::get(),
					..Default::default()
				}),
			RuntimeCall::Session(SessionCall::purge_keys {}) => validate_purge_keys(who),

			// Bridge-related calls
			RuntimeCall::BridgePolkadotGrandpa(BridgeGrandpaCall::submit_finality_proof {
				..
			}) |
			RuntimeCall::BridgePolkadotGrandpa(BridgeGrandpaCall::submit_finality_proof_ex {
				..
			}) |
			RuntimeCall::BridgePolkadotParachains(
				BridgeParachainsCall::submit_parachain_heads { .. },
			) |
			RuntimeCall::BridgePolkadotParachains(
				BridgeParachainsCall::submit_parachain_heads_ex { .. },
			) |
			RuntimeCall::BridgePolkadotMessages(BridgeMessagesCall::receive_messages_proof {
				..
			}) |
			RuntimeCall::BridgePolkadotMessages(
				BridgeMessagesCall::receive_messages_delivery_proof { .. },
			) => RelayerSet::validate_bridge_tx(who).map(|()| ValidTransaction {
				priority: BridgeTxPriority::get(),
				longevity: BridgeTxLongevity::get(),
				..Default::default()
			}),

			// Bridge-privileged calls
			RuntimeCall::BridgePolkadotGrandpa(BridgeGrandpaCall::initialize { .. }) =>
				BridgePolkadotGrandpa::ensure_owner_or_root(origin.clone())
					.map_err(|_| InvalidTransaction::BadSigner.into())
					.map(|()| ValidTransaction {
						priority: BridgeTxPriority::get(),
						longevity: BridgeTxLongevity::get(),
						..Default::default()
					}),

			// Sudo calls
			RuntimeCall::Proxy(_call) => Ok(ValidTransaction {
				priority: SudoPriority::get(),
				longevity: BridgeTxLongevity::get(),
				..Default::default()
			}),
			RuntimeCall::Sudo(_call) => Ok(ValidTransaction {
				priority: SudoPriority::get(),
				longevity: BridgeTxLongevity::get(),
				..Default::default()
			}),
			RuntimeCall::System(SystemCall::apply_authorized_upgrade { .. }) =>
				Ok(ValidTransaction {
					priority: SudoPriority::get(),
					longevity: BridgeTxLongevity::get(),
					..Default::default()
				}),

			// All other calls are invalid
			_ => Err(InvalidTransaction::Call.into()),
		}?;

		Ok((validity, (), origin))
	}

	fn prepare(
		self,
		_val: Self::Val,
		origin: &RuntimeOrigin,
		call: &RuntimeCall,
		_info: &DispatchInfoOf<RuntimeCall>,
		_len: usize,
	) -> Result<Self::Pre, TransactionValidityError> {
		let who = origin.as_system_origin_signer().ok_or(InvalidTransaction::BadSigner)?;
		match call {
			// Transaction storage validation
			RuntimeCall::TransactionStorage(inner_call) =>
				TransactionStorage::pre_dispatch_signed(who, inner_call).map(|()| None),

			// Session key management
			RuntimeCall::Session(SessionCall::set_keys { .. }) =>
				ValidatorSet::pre_dispatch_set_keys(who).map(|()| None),
			RuntimeCall::Session(SessionCall::purge_keys {}) =>
				validate_purge_keys(who).map(|_| None),

			// Bridge-related calls
			RuntimeCall::BridgePolkadotGrandpa(BridgeGrandpaCall::submit_finality_proof {
				..
			}) |
			RuntimeCall::BridgePolkadotGrandpa(BridgeGrandpaCall::submit_finality_proof_ex {
				..
			}) |
			RuntimeCall::BridgePolkadotParachains(
				BridgeParachainsCall::submit_parachain_heads { .. },
			) |
			RuntimeCall::BridgePolkadotParachains(
				BridgeParachainsCall::submit_parachain_heads_ex { .. },
			) |
			RuntimeCall::BridgePolkadotMessages(BridgeMessagesCall::receive_messages_proof {
				..
			}) |
			RuntimeCall::BridgePolkadotMessages(
				BridgeMessagesCall::receive_messages_delivery_proof { .. },
			) => RelayerSet::validate_bridge_tx(who).map(|()| Some(who.clone())),

			// Bridge-privileged calls
			RuntimeCall::BridgePolkadotGrandpa(BridgeGrandpaCall::initialize { .. }) =>
				BridgePolkadotGrandpa::ensure_owner_or_root(origin.clone())
					.map_err(|_| InvalidTransaction::BadSigner.into())
					.map(|()| Some(who.clone())),

			// Sudo calls
			RuntimeCall::Proxy(_) => Ok(Some(who.clone())),
			RuntimeCall::Sudo(_) => Ok(Some(who.clone())),
			RuntimeCall::System(SystemCall::apply_authorized_upgrade { .. }) =>
				Ok(Some(who.clone())),

			// All other calls are invalid
			_ => Err(InvalidTransaction::Call.into()),
		}
	}

	fn post_dispatch_details(
		pre: Self::Pre,
		_info: &DispatchInfoOf<RuntimeCall>,
		_post_info: &PostDispatchInfoOf<RuntimeCall>,
		_len: usize,
		result: &DispatchResult,
	) -> Result<Weight, TransactionValidityError> {
		if result.is_err() {
			if let Some(who) = pre {
				RelayerSet::post_dispatch_failed_bridge_tx(&who);
			}
		}
		Ok(Weight::zero())
	}
}

// It'll generate signed extensions to invalidate obsolete bridge transactions before
// they'll be included in the block
generate_bridge_reject_obsolete_headers_and_messages! {
	RuntimeCall, AccountId,
	// Grandpa
	BridgePolkadotGrandpa,
	// Parachains
	BridgePolkadotParachains,
	// Messages
	BridgePolkadotMessages
}

/// The SignedExtension to the basic transaction logic.
pub type TxExtension = (
	frame_system::CheckNonZeroSender<Runtime>,
	frame_system::CheckSpecVersion<Runtime>,
	frame_system::CheckTxVersion<Runtime>,
	frame_system::CheckGenesis<Runtime>,
	frame_system::CheckEra<Runtime>,
	frame_system::CheckNonce<Runtime>,
	frame_system::CheckWeight<Runtime>,
	ValidateSigned,
	BridgeRejectObsoleteHeadersAndMessages,
);

/// Unchecked extrinsic type as expected by this runtime.
pub type UncheckedExtrinsic =
	generic::UncheckedExtrinsic<Address, RuntimeCall, Signature, TxExtension>;
/// The payload being signed in transactions.
pub type SignedPayload = generic::SignedPayload<RuntimeCall, TxExtension>;
/// Executive: handles dispatch to the various modules.
pub type Executive = frame_executive::Executive<
	Runtime,
	Block,
	frame_system::ChainContext<Runtime>,
	Runtime,
	AllPalletsWithSystem,
>;

#[cfg(feature = "runtime-benchmarks")]
mod benches {
	use super::*;

	frame_benchmarking::define_benchmarks!(
		[frame_benchmarking::baseline, Baseline::<Runtime>]
		[frame_system, SystemBench::<Runtime>]
		[frame_system_extensions, SystemExtensionsBench::<Runtime>]
		[pallet_timestamp, Timestamp]
		[pallet_transaction_storage, TransactionStorage]
		[pallet_validator_set, ValidatorSet]
		[pallet_relayer_set, RelayerSet]

		[pallet_bridge_grandpa, BridgePolkadotGrandpa]
		[pallet_bridge_parachains, PolkadotParachains]
		[pallet_bridge_messages, PolkadotMessages]

		[pallet_sudo, Sudo]
		[pallet_proxy, Proxy]
	);

	pub use frame_benchmarking::{baseline::Pallet as Baseline, BenchmarkBatch, BenchmarkList};
	pub use frame_system_benchmarking::{
		extensions::Pallet as SystemExtensionsBench, Pallet as SystemBench,
	};

	pub use frame_support::traits::{StorageInfoTrait, WhitelistedStorageKeys};
	pub use sp_storage::TrackedStorageKey;

	impl frame_system_benchmarking::Config for Runtime {}
	impl frame_benchmarking::baseline::Config for Runtime {}

	use bridge_runtime_common::parachains_benchmarking::prepare_parachain_heads_proof;
	use pallet_bridge_parachains::benchmarking::Config as BridgeParachainsConfig;

	impl BridgeParachainsConfig<bridge_config::WithPolkadotBridgeParachainsInstance> for Runtime {
		fn parachains() -> Vec<bp_polkadot_core::parachains::ParaId> {
			use bp_runtime::Parachain;
			vec![bp_polkadot_core::parachains::ParaId(
				bridge_config::bp_people_polkadot::PeoplePolkadot::PARACHAIN_ID,
			)]
		}

		fn prepare_parachain_heads_proof(
			parachains: &[bp_polkadot_core::parachains::ParaId],
			parachain_head_size: u32,
			proof_params: bp_runtime::UnverifiedStorageProofParams,
		) -> (
			bp_parachains::RelayBlockNumber,
			bp_parachains::RelayBlockHash,
			bp_polkadot_core::parachains::ParaHeadsProof,
			Vec<(bp_polkadot_core::parachains::ParaId, bp_polkadot_core::parachains::ParaHash)>,
		) {
			prepare_parachain_heads_proof::<
				Runtime,
				bridge_config::WithPolkadotBridgeParachainsInstance,
			>(parachains, parachain_head_size, proof_params)
		}
	}

	use bridge_runtime_common::messages_benchmarking::{
		generate_xcm_builder_bridge_message_sample, prepare_message_delivery_proof_from_parachain,
		prepare_message_proof_from_parachain,
	};
	use pallet_bridge_messages::{
		benchmarking::{
			Config as BridgeMessagesConfig, MessageDeliveryProofParams, MessageProofParams,
		},
		LaneIdOf,
	};

	impl BridgeMessagesConfig<bridge_config::WithPeoplePolkadotMessagesInstance> for Runtime {
		fn is_relayer_rewarded(_relayer: &Self::AccountId) -> bool {
			// TODO:
			// no rewards, so we don't care
			true
		}

		fn prepare_message_proof(
			params: MessageProofParams<
				LaneIdOf<Runtime, bridge_config::WithPeoplePolkadotMessagesInstance>,
			>,
		) -> (bridge_config::benchmarking::FromPeoplePolkadotMessagesProof, Weight) {
			prepare_message_proof_from_parachain::<
				Runtime,
				bridge_config::WithPolkadotBridgeGrandpaInstance,
				bridge_config::WithPeoplePolkadotMessagesInstance,
			>(
				params,
				generate_xcm_builder_bridge_message_sample(
					bridge_config::PeoplePolkadotLocation::get().interior().clone(),
				),
			)
		}

		fn prepare_message_delivery_proof(
			params: MessageDeliveryProofParams<
				AccountId,
				LaneIdOf<Runtime, bridge_config::WithPeoplePolkadotMessagesInstance>,
			>,
		) -> bridge_config::benchmarking::ToPeoplePolkadotMessagesDeliveryProof {
			prepare_message_delivery_proof_from_parachain::<
				Runtime,
				bridge_config::WithPolkadotBridgeGrandpaInstance,
				bridge_config::WithPeoplePolkadotMessagesInstance,
			>(params)
		}

		fn is_message_successfully_dispatched(_nonce: bp_messages::MessageNonce) -> bool {
			// TODO:
			// currently we have no means to detect that
			true
		}
	}

	pub type PolkadotParachains = pallet_bridge_parachains::benchmarking::Pallet<
		Runtime,
		bridge_config::WithPolkadotBridgeParachainsInstance,
	>;
	pub type PolkadotMessages = pallet_bridge_messages::benchmarking::Pallet<
		Runtime,
		bridge_config::WithPeoplePolkadotMessagesInstance,
	>;
}

#[cfg(feature = "runtime-benchmarks")]
use benches::*;

impl_runtime_apis! {
	impl sp_api::Core<Block> for Runtime {
		fn version() -> RuntimeVersion {
			VERSION
		}

		fn execute_block(block: <Block as BlockT>::LazyBlock) {
			Executive::execute_block(block);
		}

		fn initialize_block(header: &<Block as BlockT>::Header) -> sp_runtime::ExtrinsicInclusionMode {
			Executive::initialize_block(header)
		}
	}

	impl sp_api::Metadata<Block> for Runtime {
		fn metadata() -> OpaqueMetadata {
			OpaqueMetadata::new(Runtime::metadata().into())
		}

		fn metadata_at_version(version: u32) -> Option<OpaqueMetadata> {
			Runtime::metadata_at_version(version)
		}

		fn metadata_versions() -> sp_std::vec::Vec<u32> {
			Runtime::metadata_versions()
		}
	}

	impl frame_support::view_functions::runtime_api::RuntimeViewFunction<Block> for Runtime {
		fn execute_view_function(id: frame_support::view_functions::ViewFunctionId, input: Vec<u8>) -> Result<Vec<u8>, frame_support::view_functions::ViewFunctionDispatchError> {
			Runtime::execute_view_function(id, input)
		}
	}

	impl sp_block_builder::BlockBuilder<Block> for Runtime {
		fn apply_extrinsic(extrinsic: <Block as BlockT>::Extrinsic) -> ApplyExtrinsicResult {
			Executive::apply_extrinsic(extrinsic)
		}

		fn finalize_block() -> <Block as BlockT>::Header {
			Executive::finalize_block()
		}

		fn inherent_extrinsics(data: sp_inherents::InherentData) -> Vec<<Block as BlockT>::Extrinsic> {
			data.create_extrinsics()
		}

		fn check_inherents(
			block: <Block as BlockT>::LazyBlock,
			data: sp_inherents::InherentData,
		) -> sp_inherents::CheckInherentsResult {
			data.check_extrinsics(&block)
		}
	}

	impl sp_transaction_pool::runtime_api::TaggedTransactionQueue<Block> for Runtime {
		fn validate_transaction(
			source: TransactionSource,
			tx: <Block as BlockT>::Extrinsic,
			block_hash: <Block as BlockT>::Hash,
		) -> TransactionValidity {
			Executive::validate_transaction(source, tx, block_hash)
		}
	}

	impl sp_offchain::OffchainWorkerApi<Block> for Runtime {
		fn offchain_worker(header: &<Block as BlockT>::Header) {
			Executive::offchain_worker(header)
		}
	}

	impl sp_session::SessionKeys<Block> for Runtime {
		fn generate_session_keys(seed: Option<Vec<u8>>) -> Vec<u8> {
			opaque::SessionKeys::generate(seed)
		}

		fn decode_session_keys(
			encoded: Vec<u8>,
		) -> Option<Vec<(Vec<u8>, KeyTypeId)>> {
			opaque::SessionKeys::decode_into_raw_public_keys(&encoded)
		}
	}

	impl sp_consensus_babe::BabeApi<Block> for Runtime {
		fn configuration() -> sp_consensus_babe::BabeConfiguration {
			let epoch_config = Babe::epoch_config().unwrap_or(BABE_GENESIS_EPOCH_CONFIG);
			sp_consensus_babe::BabeConfiguration {
				slot_duration: Babe::slot_duration(),
				epoch_length: EPOCH_DURATION_IN_SLOTS,
				c: epoch_config.c,
				authorities: Babe::authorities().to_vec(),
				randomness: Babe::randomness(),
				allowed_slots: epoch_config.allowed_slots,
			}
		}

		fn current_epoch_start() -> sp_consensus_babe::Slot {
			Babe::current_epoch_start()
		}

		fn current_epoch() -> sp_consensus_babe::Epoch {
			Babe::current_epoch()
		}

		fn next_epoch() -> sp_consensus_babe::Epoch {
			Babe::next_epoch()
		}

		fn generate_key_ownership_proof(
			_slot: sp_consensus_babe::Slot,
			authority_id: sp_consensus_babe::AuthorityId,
		) -> Option<sp_consensus_babe::OpaqueKeyOwnershipProof> {
			use codec::Encode;

			Historical::prove((sp_consensus_babe::KEY_TYPE, authority_id))
				.map(|p| p.encode())
				.map(sp_consensus_babe::OpaqueKeyOwnershipProof::new)
		}

		fn submit_report_equivocation_unsigned_extrinsic(
			equivocation_proof: sp_consensus_babe::EquivocationProof<<Block as BlockT>::Header>,
			key_owner_proof: sp_consensus_babe::OpaqueKeyOwnershipProof,
		) -> Option<()> {
			let key_owner_proof = key_owner_proof.decode()?;

			Babe::submit_unsigned_equivocation_report(
				equivocation_proof,
				key_owner_proof,
			)
		}
	}

	impl sp_consensus_grandpa::GrandpaApi<Block> for Runtime {
		fn grandpa_authorities() -> sp_consensus_grandpa::AuthorityList {
			Grandpa::grandpa_authorities()
		}

		fn current_set_id() -> sp_consensus_grandpa::SetId {
			Grandpa::current_set_id()
		}

		fn submit_report_equivocation_unsigned_extrinsic(
			_equivocation_proof: sp_consensus_grandpa::EquivocationProof<
				<Block as BlockT>::Hash,
				NumberFor<Block>,
			>,
			_key_owner_proof: sp_consensus_grandpa::OpaqueKeyOwnershipProof,
		) -> Option<()> {
			None
		}

		fn generate_key_ownership_proof(
			_set_id: sp_consensus_grandpa::SetId,
			_authority_id: GrandpaId,
		) -> Option<sp_consensus_grandpa::OpaqueKeyOwnershipProof> {
			// NOTE: this is the only implementation possible since we've
			// defined our key owner proof type as a bottom type (i.e. a type
			// with no values).
			None
		}
	}

	impl frame_system_rpc_runtime_api::AccountNonceApi<Block, AccountId, Nonce> for Runtime {
		fn account_nonce(account: AccountId) -> Nonce {
			System::account_nonce(account)
		}
	}

	impl bp_polkadot::PolkadotFinalityApi<Block> for Runtime {
		fn best_finalized() -> Option<bp_runtime::HeaderId<bp_polkadot_core::Hash, bp_polkadot_core::BlockNumber>> {
			BridgePolkadotGrandpa::best_finalized()
		}

		fn synced_headers_grandpa_info(
		) -> Vec<bp_header_chain::StoredHeaderGrandpaInfo<bp_polkadot_core::Header>> {
			BridgePolkadotGrandpa::synced_headers_grandpa_info()
		}

		fn free_headers_interval() -> Option<u32> {
			<Runtime as pallet_bridge_grandpa::Config<
				bridge_config::WithPolkadotBridgeGrandpaInstance
			>>::FreeHeadersInterval::get()
		}
	}

	impl bp_people_polkadot::PeoplePolkadotFinalityApi<Block> for Runtime {
		fn best_finalized() -> Option<bp_runtime::HeaderId<bp_people_polkadot::Hash, bp_people_polkadot::BlockNumber>> {
			BridgePolkadotParachains::best_parachain_head_id::<
				bp_people_polkadot::PeoplePolkadot
			>().unwrap_or(None)
		}

		fn free_headers_interval() -> Option<u32> {
			// "free interval" is not currently used for parachains
			None
		}
	}

	impl bp_people_polkadot::FromPeoplePolkadotInboundLaneApi<Block> for Runtime {
		fn message_details(
			lane: bp_messages::LegacyLaneId,
			messages: Vec<(bp_messages::MessagePayload, bp_messages::OutboundMessageDetails)>,
		) -> Vec<bp_messages::InboundMessageDetails> {
			bridge_runtime_common::messages_api::inbound_message_details::<
				Runtime,
				bridge_config::WithPeoplePolkadotMessagesInstance,
			>(lane, messages)
		}
	}

	impl bp_people_polkadot::ToPeoplePolkadotOutboundLaneApi<Block> for Runtime {
		fn message_details(
			lane: bp_messages::LegacyLaneId,
			begin: bp_messages::MessageNonce,
			end: bp_messages::MessageNonce,
		) -> Vec<bp_messages::OutboundMessageDetails> {
			bridge_runtime_common::messages_api::outbound_message_details::<
				Runtime,
				bridge_config::WithPeoplePolkadotMessagesInstance,
			>(lane, begin, end)
		}
	}

	impl sp_genesis_builder::GenesisBuilder<Block> for Runtime {
		fn build_state(config: Vec<u8>) -> sp_genesis_builder::Result {
			build_state::<RuntimeGenesisConfig>(config)
		}

		fn get_preset(id: &Option<sp_genesis_builder::PresetId>) -> Option<Vec<u8>> {
			get_preset::<RuntimeGenesisConfig>(id, genesis_config_presets::get_preset)
		}

		fn preset_names() -> Vec<sp_genesis_builder::PresetId> {
			genesis_config_presets::preset_names()
		}
	}

	#[cfg(feature = "runtime-benchmarks")]
	impl frame_benchmarking::Benchmark<Block> for Runtime {
		fn benchmark_metadata(extra: bool) -> (
			Vec<frame_benchmarking::BenchmarkList>,
			Vec<frame_support::traits::StorageInfo>,
		) {
			let mut list = Vec::<BenchmarkList>::new();
			list_benchmarks!(list, extra);

			let storage_info = AllPalletsWithSystem::storage_info();
			(list, storage_info)
		}

		#[allow(non_local_definitions)]
		fn dispatch_benchmark(
			config: frame_benchmarking::BenchmarkConfig
		) -> Result<Vec<frame_benchmarking::BenchmarkBatch>, alloc::string::String> {
			let whitelist: Vec<TrackedStorageKey> = AllPalletsWithSystem::whitelisted_storage_keys();
			let mut batches = Vec::<BenchmarkBatch>::new();
			let params = (&config, &whitelist);
			add_benchmarks!(params, batches);

			Ok(batches)
		}
	}

	#[cfg(feature = "try-runtime")]
	impl frame_try_runtime::TryRuntime<Block> for Runtime {
		fn on_runtime_upgrade(checks: frame_try_runtime::UpgradeCheckSelect) -> (Weight, Weight) {
			// NOTE: intentional unwrap: we don't want to propagate the error backwards, and want to
			// have a backtrace here. If any of the pre/post migration checks fail, we shall stop
			// right here and right now.
			let weight = Executive::try_runtime_upgrade(checks).unwrap();
			(weight, BlockWeights::get().max_block)
		}

		fn execute_block(
			block: <Block as BlockT>::LazyBlock,
			state_root_check: bool,
			signature_check: bool,
			select: frame_try_runtime::TryStateSelect
		) -> Weight {
			// NOTE: intentional unwrap: we don't want to propagate the error backwards, and want to
			// have a backtrace here.
			Executive::try_execute_block(block, state_root_check, signature_check, select).expect("execute-block failed")
		}
	}

	impl pallet_transaction_payment_rpc_runtime_api::TransactionPaymentApi<Block, u128> for Runtime {
		fn query_info(
			uxt: <Block as BlockT>::Extrinsic,
			_len: u32,
		) -> pallet_transaction_payment::RuntimeDispatchInfo<u128> {
			let dispatch_info = <<Block as BlockT>::Extrinsic as GetDispatchInfo>::get_dispatch_info(&uxt);
			pallet_transaction_payment::RuntimeDispatchInfo {
				weight: dispatch_info.total_weight(),
				class: dispatch_info.class,
				partial_fee: 0
			}
		}
		fn query_fee_details(
			_uxt: <Block as BlockT>::Extrinsic,
			_len: u32,
		) -> pallet_transaction_payment::FeeDetails<u128> {
			todo!()
		}
		fn query_weight_to_fee(_weight: Weight) -> u128 {
			todo!()
		}
		fn query_length_to_fee(_len: u32) -> u128 {
			todo!()
		}
	}
}
