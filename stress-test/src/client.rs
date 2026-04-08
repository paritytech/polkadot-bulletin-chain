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

/// jsonrpsee defaults to **10 MiB**; large `author_pendingExtrinsics` / submit payloads need more.
/// Align with the node’s RPC max frame settings if you still hit size errors.
const MAX_RPC_WS_FRAME: u32 = 50 * 1024 * 1024;

/// WebSocket JSON-RPC client with request/response size limits above the jsonrpsee default (10
/// MiB).
pub fn ws_client_builder() -> jsonrpsee::ws_client::WsClientBuilder {
	jsonrpsee::ws_client::WsClientBuilder::default()
		.max_request_size(MAX_RPC_WS_FRAME)
		.max_response_size(MAX_RPC_WS_FRAME)
}

pub async fn connect(ws_url: &str) -> Result<OnlineClient<BulletinConfig>> {
	log::info!("Connecting to {ws_url}");

	let rpc_client = ws_client_builder().build(ws_url).await?;

	let client = OnlineClient::<BulletinConfig>::from_rpc_client(rpc_client).await?;
	log::info!("Connected successfully");
	Ok(client)
}

/// Discover the node's P2P listen addresses and peer ID via a separate RPC call.
/// Returns (peer_id, listen_addresses).
pub async fn discover_p2p_info(ws_url: &str) -> Result<(String, Vec<String>)> {
	use jsonrpsee::core::client::ClientT;

	let client = ws_client_builder().build(ws_url).await?;

	let peer_id: String = client.request("system_localPeerId", jsonrpsee::rpc_params![]).await?;

	let addresses: Vec<String> =
		client.request("system_localListenAddresses", jsonrpsee::rpc_params![]).await?;

	Ok((peer_id, addresses))
}

/// Ready + future transaction count from the node (lightweight `txpool_status` when available).
pub async fn fetch_txpool_pending_total(ws_url: &str) -> Result<usize> {
	use jsonrpsee::{core::client::ClientT, rpc_params};

	let client = ws_client_builder().build(ws_url).await?;

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
