use anyhow::{Context, Result};
use subxt::{
	config::{
		substrate::SubstrateConfig, transaction_extensions, Config, DefaultExtrinsicParamsBuilder,
		ExtrinsicParams,
	},
	OnlineClient,
};

// --- BulletinConfig ---

/// Subxt Config for the bulletin chain. Uses standard Substrate extensions.
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

	pub fn mortal(mut self, period: u64) -> Self {
		self.0 = self.0.mortal(period);
		self
	}

	pub fn immortal(mut self) -> Self {
		self.0 = self.0.immortal();
		self
	}

	pub fn build(self) -> <BulletinExtrinsicParams as ExtrinsicParams<BulletinConfig>>::Params {
		self.0.build()
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
	tracing::debug!("Connecting to {ws_url}");

	let rpc_client = connect_ws(ws_url).await?;
	let client = OnlineClient::<BulletinConfig>::from_rpc_client(rpc_client).await?;
	tracing::debug!("Connected to {ws_url}");
	Ok(client)
}

/// Raw WebSocket RPC client (for fire-and-forget submissions).
pub async fn connect_ws(ws_url: &str) -> Result<jsonrpsee::ws_client::WsClient> {
	let client = jsonrpsee::ws_client::WsClientBuilder::default()
		.max_request_size(MAX_RPC_MESSAGE_SIZE)
		.max_response_size(MAX_RPC_MESSAGE_SIZE)
		.build(ws_url)
		.await?;
	Ok(client)
}

/// Compute blake2b-256 hash (same as the runtime's `content_hash` for `store` calls).
pub fn blake2b_256(data: &[u8]) -> [u8; 32] {
	use blake2::digest::{consts::U32, Digest};
	let hash = blake2::Blake2b::<U32>::digest(data);
	let mut out = [0u8; 32];
	out.copy_from_slice(&hash);
	out
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

/// Ready + future transaction count from the node (lightweight `txpool_status` when available).
pub async fn fetch_txpool_pending_total(ws_url: &str) -> Result<usize> {
	use jsonrpsee::{core::client::ClientT, rpc_params, ws_client::WsClientBuilder};

	let client = WsClientBuilder::default().build(ws_url).await?;

	if let Ok(v) = client.request::<serde_json::Value, _>("txpool_status", rpc_params![]).await {
		let n = match &v {
			serde_json::Value::Array(arr) if arr.len() >= 2 =>
				arr[0].as_u64().unwrap_or(0).saturating_add(arr[1].as_u64().unwrap_or(0)),
			serde_json::Value::Object(map) => map
				.get("ready")
				.and_then(|x| x.as_u64())
				.unwrap_or(0)
				.saturating_add(map.get("future").and_then(|x| x.as_u64()).unwrap_or(0)),
			_ => anyhow::bail!("unexpected txpool_status JSON: {v}"),
		};
		return Ok(n as usize);
	}

	let pending: Vec<serde_json::Value> = client
		.request("author_pendingExtrinsics", rpc_params![])
		.await
		.context("author_pendingExtrinsics RPC")?;
	Ok(pending.len())
}
