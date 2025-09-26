//! Expose the auto generated weight files.

use ::pallet_bridge_grandpa::WeightInfoExt as GrandpaWeightInfoExt;
use ::pallet_bridge_messages::WeightInfoExt as MessagesWeightInfoExt;
use ::pallet_bridge_parachains::WeightInfoExt as ParachainsWeightInfoExt;
use ::pallet_bridge_relayers::WeightInfo as _;
use frame_support::weights::Weight;

pub mod bridge_polkadot_relayers;
pub mod frame_system;
pub mod pallet_bridge_grandpa;
pub mod pallet_bridge_messages;
pub mod pallet_bridge_parachains;
pub mod pallet_relayer_set;
pub mod pallet_timestamp;
pub mod pallet_transaction_storage;
pub mod pallet_validator_set;

impl GrandpaWeightInfoExt for pallet_bridge_grandpa::WeightInfo<crate::Runtime> {
	fn submit_finality_proof_overhead_from_runtime() -> Weight {
		// our signed extension:
		// 1) checks whether relayer registration is active from validate/pre_dispatch;
		// 2) may slash and deregister relayer from post_dispatch
		// (2) includes (1), so (2) is the worst case
		bridge_polkadot_relayers::WeightInfo::<crate::Runtime>::slash_and_deregister()
	}
}

impl ParachainsWeightInfoExt for pallet_bridge_parachains::WeightInfo<crate::Runtime> {
	fn expected_extra_storage_proof_size() -> u32 {
		crate::bp_people_polkadot::EXTRA_STORAGE_PROOF_SIZE
	}

	fn submit_parachain_heads_overhead_from_runtime() -> Weight {
		// our signed extension:
		// 1) checks whether relayer registration is active from validate/pre_dispatch;
		// 2) may slash and deregister relayer from post_dispatch
		// (2) includes (1), so (2) is the worst case
		bridge_polkadot_relayers::WeightInfo::<crate::Runtime>::slash_and_deregister()
	}
}

impl MessagesWeightInfoExt for pallet_bridge_messages::WeightInfo<crate::Runtime> {
	fn expected_extra_storage_proof_size() -> u32 {
		crate::bp_people_polkadot::EXTRA_STORAGE_PROOF_SIZE
	}

	fn receive_messages_proof_overhead_from_runtime() -> Weight {
		Weight::zero()
	}

	fn receive_messages_delivery_proof_overhead_from_runtime() -> Weight {
		Weight::zero()
	}
}
