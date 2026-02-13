// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Custom subxt Config for the bulletin chain.
//!
//! The bulletin chain's fix runtime introduces a `ProvideCidConfig` transaction extension
//! that isn't in the standard `SubstrateConfig`. This module defines a `BulletinConfig`
//! that includes it, allowing subxt to construct transactions for both pre- and post-upgrade
//! runtimes (the `AnyOf` type dynamically selects extensions based on chain metadata).

use subxt::{
	client::ClientState,
	config::{
		substrate::SubstrateConfig,
		transaction_extensions::{self, TransactionExtension},
		Config, DefaultExtrinsicParamsBuilder, ExtrinsicParams, ExtrinsicParamsEncoder,
		ExtrinsicParamsError,
	},
};

// --- ProvideCidConfig extension ---

/// Subxt-side implementation of the bulletin chain's `ProvideCidConfig` extension.
/// Always sends `None` (no custom CID config), which is the default for all calls
/// except `store` with explicit CID parameters.
pub struct ProvideCidConfigExt;

impl<T: Config> ExtrinsicParams<T> for ProvideCidConfigExt {
	type Params = ();

	fn new(_client: &ClientState<T>, _params: Self::Params) -> Result<Self, ExtrinsicParamsError> {
		Ok(ProvideCidConfigExt)
	}
}

impl ExtrinsicParamsEncoder for ProvideCidConfigExt {
	fn encode_value_to(&self, v: &mut Vec<u8>) {
		// SCALE-encode Option::<CidConfig>::None = 0x00
		v.push(0x00);
	}
}

impl<T: Config> TransactionExtension<T> for ProvideCidConfigExt {
	type Decoded = ();

	fn matches(identifier: &str, _type_id: u32, _types: &scale_info::PortableRegistry) -> bool {
		identifier == "ProvideCidConfig"
	}
}

// --- BulletinConfig ---

/// Subxt Config for the bulletin chain. Identical to `SubstrateConfig` but with
/// `ProvideCidConfig` in the `AnyOf` extension tuple.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum BulletinConfig {}

/// Extrinsic params type: standard 9 extensions + ProvideCidConfig.
pub type BulletinExtrinsicParams = transaction_extensions::AnyOf<
	BulletinConfig,
	(
		transaction_extensions::VerifySignature<BulletinConfig>,
		transaction_extensions::CheckSpecVersion,
		transaction_extensions::CheckTxVersion,
		transaction_extensions::CheckNonce,
		transaction_extensions::CheckGenesis<BulletinConfig>,
		transaction_extensions::CheckMortality<BulletinConfig>,
		transaction_extensions::ChargeAssetTxPayment<BulletinConfig>,
		transaction_extensions::ChargeTransactionPayment,
		transaction_extensions::CheckMetadataHash,
		ProvideCidConfigExt,
	),
>;

impl Config for BulletinConfig {
	type AccountId = <SubstrateConfig as Config>::AccountId;
	type Address = <SubstrateConfig as Config>::Address;
	type Signature = <SubstrateConfig as Config>::Signature;
	type Hasher = <SubstrateConfig as Config>::Hasher;
	type Header = <SubstrateConfig as Config>::Header;
	type ExtrinsicParams = BulletinExtrinsicParams;
	type AssetId = <SubstrateConfig as Config>::AssetId;
}

// --- Params builder ---

/// Builder for `BulletinConfig` extrinsic params. Wraps the standard builder and
/// appends `()` for the `ProvideCidConfig` extension.
pub struct BulletinExtrinsicParamsBuilder(DefaultExtrinsicParamsBuilder<BulletinConfig>);

impl BulletinExtrinsicParamsBuilder {
	pub fn new() -> Self {
		Self(DefaultExtrinsicParamsBuilder::new())
	}

	pub fn nonce(mut self, nonce: u64) -> Self {
		self.0 = self.0.nonce(nonce);
		self
	}

	/// Build params for `BulletinExtrinsicParams` (10-tuple).
	pub fn build(self) -> <BulletinExtrinsicParams as ExtrinsicParams<BulletinConfig>>::Params {
		let (a, b, c, d, e, f, g, h, i) = self.0.build();
		(a, b, c, d, e, f, g, h, i, ())
	}
}
