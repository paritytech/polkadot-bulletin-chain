//! Expose the auto generated weight files.

use frame_support::weights::Weight;
use pallet_bridge_relayers::WeightInfo;

pub mod bridge_polkadot_grandpa;
pub mod bridge_polkadot_messages;
pub mod bridge_polkadot_parachains;
pub mod bridge_polkadot_relayers;

impl pallet_bridge_grandpa::WeightInfoExt for bridge_polkadot_grandpa::WeightInfo<crate::Runtime> {
	fn submit_finality_proof_overhead_from_runtime() -> Weight {
		// our signed extension:
		// 1) checks whether relayer registration is active from validate/pre_dispatch;
		// 2) may slash and deregister relayer from post_dispatch
		// (2) includes (1), so (2) is the worst case
		bridge_polkadot_relayers::WeightInfo::<crate::Runtime>::slash_and_deregister()
	}
}

impl pallet_bridge_parachains::WeightInfoExt
	for bridge_polkadot_parachains::WeightInfo<crate::Runtime>
{
	fn expected_extra_storage_proof_size() -> u32 {
		// TODO: (clean up) https://github.com/paritytech/polkadot-bulletin-chain/issues/22
		#[cfg(feature = "rococo")]
		{ bp_bridge_hub_rococo::EXTRA_STORAGE_PROOF_SIZE }
		#[cfg(feature = "polkadot")]
		{ bp_people_hub_polkadot::EXTRA_STORAGE_PROOF_SIZE }
	}

	fn submit_parachain_heads_overhead_from_runtime() -> Weight {
		// our signed extension:
		// 1) checks whether relayer registration is active from validate/pre_dispatch;
		// 2) may slash and deregister relayer from post_dispatch
		// (2) includes (1), so (2) is the worst case
		bridge_polkadot_relayers::WeightInfo::<crate::Runtime>::slash_and_deregister()
	}
}

impl pallet_bridge_messages::WeightInfoExt
	for bridge_polkadot_messages::WeightInfo<crate::Runtime>
{
	fn expected_extra_storage_proof_size() -> u32 {
		// TODO: (clean up) https://github.com/paritytech/polkadot-bulletin-chain/issues/22
		#[cfg(feature = "rococo")]
		{ bp_bridge_hub_rococo::EXTRA_STORAGE_PROOF_SIZE }
		#[cfg(feature = "polkadot")]
		{ bp_people_hub_polkadot::EXTRA_STORAGE_PROOF_SIZE }
	}

	fn receive_messages_proof_overhead_from_runtime() -> Weight {
		Weight::zero()
	}

	fn receive_messages_delivery_proof_overhead_from_runtime() -> Weight {
		Weight::zero()
	}
}
