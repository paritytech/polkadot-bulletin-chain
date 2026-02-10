// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Subxt configuration and custom signed extensions for Bulletin Chain.
//!
//! This module provides pre-configured types for using Bulletin Chain with `subxt`.
//! It handles the custom `ProvideCidConfig` transaction extension and provides
//! a ready-to-use `BulletinConfig` type.

use codec::Encode;
use subxt::{
	client::ClientState,
	config::{
		signed_extensions::{
			AnyOf, ChargeAssetTxPayment, ChargeTransactionPayment, CheckGenesis, CheckMetadataHash,
			CheckMortality, CheckNonce, CheckSpecVersion, CheckTxVersion,
		},
		substrate::SubstrateConfig,
		Config, ExtrinsicParams, ExtrinsicParamsEncoder,
	},
	error::ExtrinsicParamsError,
};

/// Custom signed extension for Bulletin Chain's ProvideCidConfig.
///
/// This extension allows specifying custom CID configuration for stored data.
/// In most cases, you'll want to use `None` (the default) to let the pallet
/// use its default CID configuration.
///
/// # Example
///
/// ```ignore
/// use subxt::OnlineClient;
/// use bulletin_sdk_rust::subxt_config::BulletinConfig;
///
/// let client = OnlineClient::<BulletinConfig>::from_url("ws://localhost:9944").await?;
/// ```
#[derive(Debug, Clone)]
pub struct ProvideCidConfig;

impl<T: Config> ExtrinsicParams<T> for ProvideCidConfig {
	type Params = ();

	fn new(_client: &ClientState<T>, _params: Self::Params) -> Result<Self, ExtrinsicParamsError> {
		Ok(ProvideCidConfig)
	}
}

impl ExtrinsicParamsEncoder for ProvideCidConfig {
	fn encode_extra_to(&self, v: &mut Vec<u8>) {
		// Encode Option<CidConfig>::None = 0x00
		// This means "use the pallet's default CID configuration"
		None::<()>.encode_to(v);
	}
}

impl<T: Config> subxt::config::SignedExtension<T> for ProvideCidConfig {
	type Decoded = ();

	fn matches(identifier: &str, _type_id: u32, _types: &scale_info::PortableRegistry) -> bool {
		identifier == "ProvideCidConfig"
	}
}

/// Custom extrinsic params for Bulletin Chain.
///
/// Includes all standard Substrate extensions plus the custom `ProvideCidConfig` extension.
/// Uses `AnyOf` to dynamically select the right extensions based on runtime metadata.
pub type BulletinExtrinsicParams<T> = AnyOf<
	T,
	(
		CheckSpecVersion,
		CheckTxVersion,
		CheckNonce,
		CheckGenesis<T>,
		CheckMortality<T>,
		ChargeAssetTxPayment<T>,
		ChargeTransactionPayment,
		CheckMetadataHash,
		ProvideCidConfig,
	),
>;

/// Pre-configured `Config` type for Bulletin Chain.
///
/// This is a drop-in replacement for `SubstrateConfig` that includes
/// support for Bulletin's custom `ProvideCidConfig` transaction extension.
///
/// # Example
///
/// ```ignore
/// use subxt::OnlineClient;
/// use bulletin_sdk_rust::subxt_config::BulletinConfig;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Connect to Bulletin Chain node
///     let client = OnlineClient::<BulletinConfig>::from_url("ws://localhost:9944").await?;
///
///     // Use the client with subxt's generated code
///     // The ProvideCidConfig extension is handled automatically
///     Ok(())
/// }
/// ```
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum BulletinConfig {}

impl Config for BulletinConfig {
	type Hash = <SubstrateConfig as Config>::Hash;
	type AccountId = <SubstrateConfig as Config>::AccountId;
	type Address = <SubstrateConfig as Config>::Address;
	type Signature = <SubstrateConfig as Config>::Signature;
	type Hasher = <SubstrateConfig as Config>::Hasher;
	type Header = <SubstrateConfig as Config>::Header;
	type ExtrinsicParams = BulletinExtrinsicParams<Self>;
	type AssetId = <SubstrateConfig as Config>::AssetId;
}

#[cfg(test)]
mod tests {
	use super::*;
	use subxt::config::SignedExtension;

	#[test]
	fn test_provide_cid_config_encodes_none() {
		let extension = ProvideCidConfig;
		let mut buf = Vec::new();
		extension.encode_extra_to(&mut buf);

		// Should encode as Option::None (0x00)
		assert_eq!(buf, vec![0x00]);
	}

	#[test]
	fn test_provide_cid_config_matches() {
		use scale_info::{PortableRegistry, Registry};

		// Create an empty registry for testing
		let registry = PortableRegistry::from(Registry::new());

		// ProvideCidConfig matches the identifier "ProvideCidConfig"
		assert!(<ProvideCidConfig as SignedExtension<BulletinConfig>>::matches(
			"ProvideCidConfig",
			0,
			&registry
		));

		// ProvideCidConfig does not match other identifiers
		assert!(!<ProvideCidConfig as SignedExtension<BulletinConfig>>::matches(
			"SomeOtherExtension",
			0,
			&registry
		));
	}
}
