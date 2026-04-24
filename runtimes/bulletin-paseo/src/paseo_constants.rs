// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Paseo system-parachain runtime constants.
//!
//! Ported from
//! <https://github.com/paseo-network/runtimes/blob/main/system-parachains/constants/src/paseo.rs>.
//!
//! The upstream file references the `paseo_runtime_constants` crate (the Paseo relay chain
//! constants crate), which is not available in this workspace. Its values are inlined in the
//! private `paseo_runtime_constants` submodule below, mirroring
//! <https://github.com/paseo-network/runtimes/blob/main/relay/paseo/constants/src/lib.rs>.
//!
//! `fee::WeightToFee` is kept as a polynomial (not the upstream `BlockRatioFee`) because this
//! runtime does not implement `pallet_revive::TxConfig`.

/// Inlined subset of the Paseo relay chain runtime constants crate.
mod paseo_runtime_constants {
	/// Paseo relay Treasury pallet index. Used as a `PalletInstance` in XCM locations.
	pub const TREASURY_PALLET_ID: u8 = 19;

	pub mod currency {
		use parachains_common::Balance;

		pub const UNITS: Balance = 10_000_000_000;
		pub const DOLLARS: Balance = UNITS;
		pub const CENTS: Balance = DOLLARS / 100;
		pub const MILLICENTS: Balance = CENTS / 1_000;

		/// Paseo relay chain existential deposit (1 PAS).
		pub const EXISTENTIAL_DEPOSIT: Balance = 100 * CENTS;

		pub const fn deposit(items: u32, bytes: u32) -> Balance {
			items as Balance * 20 * DOLLARS + (bytes as Balance) * 100 * MILLICENTS
		}
	}

	pub mod system_parachain {
		pub const ASSET_HUB_ID: u32 = 1000;
		pub const COLLECTIVES_ID: u32 = 1001;
		pub const BRIDGE_HUB_ID: u32 = 1002;
		pub const PEOPLE_ID: u32 = 1004;
		pub const BROKER_ID: u32 = 1005;
	}
}

/// Universally recognized accounts.
pub mod account {
	use frame_support::PalletId;

	/// Polkadot treasury pallet id, used to convert into AccountId
	pub const POLKADOT_TREASURY_PALLET_ID: PalletId = PalletId(*b"py/trsry");
	/// Alliance pallet ID.
	/// Used as a temporary place to deposit a slashed imbalance before teleporting to the Treasury.
	pub const ALLIANCE_PALLET_ID: PalletId = PalletId(*b"py/allia");
	/// Referenda pallet ID.
	/// Used as a temporary place to deposit a slashed imbalance before teleporting to the Treasury.
	pub const REFERENDA_PALLET_ID: PalletId = PalletId(*b"py/refer");
	/// Ambassador Referenda pallet ID.
	/// Used as a temporary place to deposit a slashed imbalance before teleporting to the Treasury.
	pub const AMBASSADOR_REFERENDA_PALLET_ID: PalletId = PalletId(*b"py/amref");
	/// Identity pallet ID.
	/// Used as a temporary place to deposit a slashed imbalance before teleporting to the Treasury.
	pub const IDENTITY_PALLET_ID: PalletId = PalletId(*b"py/ident");
	/// Fellowship treasury pallet ID
	pub const FELLOWSHIP_TREASURY_PALLET_ID: PalletId = PalletId(*b"py/feltr");
	/// Ambassador treasury pallet ID
	pub const AMBASSADOR_TREASURY_PALLET_ID: PalletId = PalletId(*b"py/ambtr");
}

/// System parachain ids on Paseo.
pub use paseo_runtime_constants::system_parachain;

/// Relay chain Treasury pallet index (used as `PalletInstance` in XCM locations).
pub use paseo_runtime_constants::TREASURY_PALLET_ID;

/// Consensus-related.
pub mod consensus {
	use frame_support::weights::{constants::WEIGHT_REF_TIME_PER_SECOND, Weight};

	/// Maximum number of blocks simultaneously accepted by the Runtime, not yet included
	/// into the relay chain.
	pub const UNINCLUDED_SEGMENT_CAPACITY: u32 = 1;
	/// How many parachain blocks are processed by the relay chain per parent. Limits the
	/// number of blocks authored per slot.
	pub const BLOCK_PROCESSING_VELOCITY: u32 = 1;
	/// Relay chain slot duration, in milliseconds.
	pub const RELAY_CHAIN_SLOT_DURATION_MILLIS: u32 = 6000;

	/// Average expected block time targeted by the parachain. Picked up by `pallet_timestamp` and
	/// `pallet_aura`.
	pub const MILLISECS_PER_BLOCK: u64 = 6000;
	pub const SLOT_DURATION: u64 = MILLISECS_PER_BLOCK;

	/// 2 seconds of compute with a 6 second average block.
	pub const MAXIMUM_BLOCK_WEIGHT: Weight = Weight::from_parts(
		WEIGHT_REF_TIME_PER_SECOND.saturating_mul(2),
		cumulus_primitives_core::relay_chain::MAX_POV_SIZE as u64,
	);

	/// Parameters enabling async backing functionality.
	///
	/// Once all system chains have migrated to the new async backing mechanism, the parameters
	/// in this namespace will replace those currently defined in `super::*`.
	pub mod async_backing {
		/// Maximum number of blocks simultaneously accepted by the Runtime, not yet included into
		/// the relay chain.
		pub const UNINCLUDED_SEGMENT_CAPACITY: u32 = 3;
	}

	/// Parameters enabling elastic scaling functionality.
	pub mod elastic_scaling {
		/// Build with an offset behind the relay chain.
		#[cfg(not(feature = "try-runtime"))]
		pub const RELAY_PARENT_OFFSET: u32 = 1;
		#[cfg(feature = "try-runtime")]
		pub const RELAY_PARENT_OFFSET: u32 = 0;

		/// The upper limit of how many parachain blocks are processed by the relay chain per
		/// parent. Limits the number of blocks authored per slot. This determines the minimum
		/// block time of the parachain:
		/// `RELAY_CHAIN_SLOT_DURATION_MILLIS/BLOCK_PROCESSING_VELOCITY`
		pub const BLOCK_PROCESSING_VELOCITY: u32 = 3;

		/// Maximum number of blocks simultaneously accepted by the Runtime, not yet included
		/// into the relay chain.
		pub const UNINCLUDED_SEGMENT_CAPACITY: u32 =
			(3 + RELAY_PARENT_OFFSET) * BLOCK_PROCESSING_VELOCITY;
	}
}

/// Time-related.
pub mod time {
	use parachains_common::BlockNumber;

	pub const MINUTES: BlockNumber =
		60_000 / (super::consensus::MILLISECS_PER_BLOCK as BlockNumber);
	pub const HOURS: BlockNumber = MINUTES * 60;
	pub const DAYS: BlockNumber = HOURS * 24;
}

/// Constants relating to PAS.
pub mod currency {
	use parachains_common::Balance;

	/// The default existential deposit for system chains. 1/10th of the Relay Chain's existential
	/// deposit. Individual system parachains may modify this in special cases.
	pub const EXISTENTIAL_DEPOSIT: Balance =
		super::paseo_runtime_constants::currency::EXISTENTIAL_DEPOSIT / 10;

	/// One "PAS" that a UI would show a user.
	pub const UNITS: Balance = 10_000_000_000;
	pub const DOLLARS: Balance = UNITS; // 10_000_000_000
	pub const GRAND: Balance = DOLLARS * 1_000; // 10_000_000_000_000
	pub const CENTS: Balance = DOLLARS / 100; // 100_000_000
	pub const MILLICENTS: Balance = CENTS / 1_000; // 100_000

	/// Deposit rate for stored data. 1/100th of the Relay Chain's deposit rate. `items` is the
	/// number of keys in storage and `bytes` is the size of the value.
	pub const fn deposit(items: u32, bytes: u32) -> Balance {
		super::paseo_runtime_constants::currency::deposit(items, bytes) / 100
	}
}

/// Constants related to Paseo fee payment.
pub mod fee {
	use frame_support::{
		pallet_prelude::Weight,
		weights::{
			constants::ExtrinsicBaseWeight, FeePolynomial, WeightToFeeCoefficient,
			WeightToFeeCoefficients, WeightToFeePolynomial,
		},
	};
	use parachains_common::Balance;
	use smallvec::smallvec;
	pub use sp_runtime::Perbill;

	/// The block saturation level. Fees will be updated based on this value.
	pub const TARGET_BLOCK_FULLNESS: Perbill = Perbill::from_percent(25);

	/// Cost of every transaction byte at Paseo system parachains. Relay chain
	/// `TRANSACTION_BYTE_FEE` is `10 * MILLICENTS`; system parachains use 1/20 of that.
	pub const TRANSACTION_BYTE_FEE: Balance = super::currency::MILLICENTS / 2;

	/// Maps a weight scalar to a fee. Mirrors the relay chain mapping
	/// (`extrinsic_base_weight -> 1/10 CENT`), scaled to 1/100 of that rate for system parachains.
	pub struct WeightToFee;
	impl frame_support::weights::WeightToFee for WeightToFee {
		type Balance = Balance;

		fn weight_to_fee(weight: &Weight) -> Self::Balance {
			let time_poly: FeePolynomial<Balance> = RefTimeToFee::polynomial().into();
			let proof_poly: FeePolynomial<Balance> = ProofSizeToFee::polynomial().into();
			time_poly.eval(weight.ref_time()).max(proof_poly.eval(weight.proof_size()))
		}
	}

	/// Maps the reference time component of `Weight` to a fee.
	pub struct RefTimeToFee;
	impl WeightToFeePolynomial for RefTimeToFee {
		type Balance = Balance;
		fn polynomial() -> WeightToFeeCoefficients<Self::Balance> {
			let p = super::currency::CENTS;
			let q = 100 * Balance::from(ExtrinsicBaseWeight::get().ref_time());
			smallvec![WeightToFeeCoefficient {
				degree: 1,
				negative: false,
				coeff_frac: Perbill::from_rational(p % q, q),
				coeff_integer: p / q,
			}]
		}
	}

	/// Maps the proof size component of `Weight` to a fee.
	pub struct ProofSizeToFee;
	impl WeightToFeePolynomial for ProofSizeToFee {
		type Balance = Balance;
		fn polynomial() -> WeightToFeeCoefficients<Self::Balance> {
			// Map 10kb proof to 1 CENT.
			let p = super::currency::CENTS;
			let q = 10_000;
			smallvec![WeightToFeeCoefficient {
				degree: 1,
				negative: false,
				coeff_frac: Perbill::from_rational(p % q, q),
				coeff_integer: p / q,
			}]
		}
	}
}

pub mod locations {
	use frame_support::{parameter_types, traits::Contains};
	use xcm::latest::prelude::{Junction::*, Location, NetworkId};

	use super::paseo_runtime_constants;

	parameter_types! {
		pub RelayChainLocation: Location = Location::parent();
		pub AssetHubLocation: Location =
			Location::new(1, Parachain(paseo_runtime_constants::system_parachain::ASSET_HUB_ID));
		pub PeopleLocation: Location =
			Location::new(1, Parachain(paseo_runtime_constants::system_parachain::PEOPLE_ID));

		pub GovernanceLocation: Location =
			Location::new(1, Parachain(paseo_runtime_constants::system_parachain::ASSET_HUB_ID));

		pub EthereumNetwork: NetworkId = NetworkId::Ethereum { chain_id: 11155111 };
	}

	/// `Contains` implementation for the asset hub location pluralities.
	pub struct AssetHubPlurality;
	impl Contains<Location> for AssetHubPlurality {
		fn contains(loc: &Location) -> bool {
			matches!(
				loc.unpack(),
				(
					1,
					[
						Parachain(paseo_runtime_constants::system_parachain::ASSET_HUB_ID),
						Plurality { .. }
					]
				)
			)
		}
	}
}

/// Default XCM version for genesis config.
pub mod xcm_version {
	pub const SAFE_XCM_VERSION: u32 = xcm::prelude::XCM_VERSION;
}
