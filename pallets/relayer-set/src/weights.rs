use core::marker::PhantomData;
use polkadot_sdk_frame::weights_prelude::*;

/// Weight functions needed for relayer_set.
pub trait WeightInfo {
	fn add_relayer() -> Weight;
	fn remove_relayer() -> Weight;
}

/// Weights for relayer_set using the Substrate node and recommended hardware.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	fn add_relayer() -> Weight {
		Weight::from_parts(1_000, 1_000)
	}
	fn remove_relayer() -> Weight {
		Weight::from_parts(1_000, 1_000)
	}
}

// For backwards compatibility and tests
impl WeightInfo for () {
	fn add_relayer() -> Weight {
		Weight::from_parts(1_000, 1_000)
	}
	fn remove_relayer() -> Weight {
		Weight::from_parts(1_000, 1_000)
	}
}
