use anyhow::Result;
use subxt::{
	client::ClientState,
	config::{
		substrate::SubstrateConfig,
		transaction_extensions::{self, TransactionExtension},
		Config, DefaultExtrinsicParamsBuilder, ExtrinsicParams, ExtrinsicParamsEncoder,
		ExtrinsicParamsError,
	},
	OnlineClient,
};

// --- ProvideCidConfig extension ---

/// Subxt-side implementation of the bulletin chain's `ProvideCidConfig` extension.
/// Always sends `None` (no custom CID config).
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

/// Subxt Config for the bulletin chain. Identical to `SubstrateConfig` but includes
/// `ProvideCidConfig` in the `AnyOf` extension tuple.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum BulletinConfig {}

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

pub struct BulletinExtrinsicParamsBuilder(DefaultExtrinsicParamsBuilder<BulletinConfig>);

impl BulletinExtrinsicParamsBuilder {
	pub fn new() -> Self {
		Self(DefaultExtrinsicParamsBuilder::new())
	}

	pub fn nonce(mut self, nonce: u64) -> Self {
		self.0 = self.0.nonce(nonce);
		self
	}

	pub fn build(self) -> <BulletinExtrinsicParams as ExtrinsicParams<BulletinConfig>>::Params {
		let (a, b, c, d, e, f, g, h, i) = self.0.build();
		(a, b, c, d, e, f, g, h, i, ())
	}
}

impl Default for BulletinExtrinsicParamsBuilder {
	fn default() -> Self {
		Self::new()
	}
}

// --- Connection ---

/// 50 MB — enough for 8 MB payloads after hex encoding + JSON-RPC wrapping.
const MAX_RPC_MESSAGE_SIZE: u32 = 50 * 1024 * 1024;

pub async fn connect(ws_url: &str) -> Result<OnlineClient<BulletinConfig>> {
	log::info!("Connecting to {ws_url}");

	// Build a WS client with larger message size limits (default is 10 MB,
	// which is too small for 8 MB payloads after hex encoding).
	let rpc_client = jsonrpsee::ws_client::WsClientBuilder::default()
		.max_request_size(MAX_RPC_MESSAGE_SIZE)
		.max_response_size(MAX_RPC_MESSAGE_SIZE)
		.build(ws_url)
		.await?;

	let client = OnlineClient::<BulletinConfig>::from_rpc_client(rpc_client).await?;
	log::info!("Connected successfully");
	Ok(client)
}

/// Discover the node's P2P listen addresses and peer ID via a separate RPC call.
/// Returns (peer_id, listen_addresses).
pub async fn discover_p2p_info(ws_url: &str) -> Result<(String, Vec<String>)> {
	use jsonrpsee::{core::client::ClientT, ws_client::WsClientBuilder};

	let client = WsClientBuilder::default().build(ws_url).await?;

	let peer_id: String = client.request("system_localPeerId", jsonrpsee::rpc_params![]).await?;

	let addresses: Vec<String> =
		client.request("system_localListenAddresses", jsonrpsee::rpc_params![]).await?;

	Ok((peer_id, addresses))
}
