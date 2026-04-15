//! Elastic scaling configuration — overrides SDK consensus defaults.
//!
//! With 6 cores and 6s relay chain slots the effective parachain block time is 1s.
//! Collators must run with `--authoring slot-based` and the parachain needs
//! 6 bulk coretime slots assigned on the relay chain.
//!
//! Constants not overridden here (MAXIMUM_BLOCK_WEIGHT, SLOT_DURATION, etc.)
//! are re-exported from the SDK's testnet constants.

// Re-export unchanged SDK consensus constants.
pub use testnet_parachains_constants::westend::consensus::{
	MAXIMUM_BLOCK_WEIGHT, RELAY_CHAIN_SLOT_DURATION_MILLIS, SLOT_DURATION,
};

/// Build blocks with an offset of 1 behind the relay chain (required for elastic scaling).
pub const RELAY_PARENT_OFFSET: u32 = 1;

/// Number of parachain blocks produced per relay chain block (= number of cores).
pub const BLOCK_PROCESSING_VELOCITY: u32 = 6;

/// Maximum unincluded blocks the runtime will accept simultaneously.
/// Formula: (3 + RELAY_PARENT_OFFSET) * BLOCK_PROCESSING_VELOCITY.
pub const UNINCLUDED_SEGMENT_CAPACITY: u32 =
	(3 + RELAY_PARENT_OFFSET) * BLOCK_PROCESSING_VELOCITY;
