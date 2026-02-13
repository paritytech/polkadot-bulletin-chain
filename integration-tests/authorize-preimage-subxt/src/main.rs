use std::{
	env,
	time::{SystemTime, UNIX_EPOCH},
};

use scale_value::{Composite as ScaleComposite, Value as ScaleValue, ValueDef as ScaleValueDef};
use sp_core::hashing::blake2_256;
use std::marker::PhantomData;

use parity_scale_codec::{Decode, Encode};
use scale_info::PortableRegistry;
use subxt::{
	config::{Config, PolkadotConfig},
	dynamic::Value,
	tx::SubmittableTransaction,
	OnlineClient,
};
use subxt_core::{
	client::ClientState,
	config::{
		transaction_extensions::{
			AnyOf, ChargeAssetTxPayment, ChargeTransactionPayment, CheckGenesis, CheckMetadataHash,
			CheckMortality, CheckNonce, CheckSpecVersion, CheckTxVersion, Params as TxParams,
			TransactionExtension,
		},
		DefaultExtrinsicParamsBuilder, ExtrinsicParams, ExtrinsicParamsEncoder,
	},
	error::ExtrinsicParamsError,
	tx,
	utils::Static,
};
use subxt_signer::sr25519::dev;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
enum CidHashingAlgorithm {
	Blake2b256,
	Sha2_256,
	Keccak256,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
struct CidConfig {
	codec: u64,
	hashing: CidHashingAlgorithm,
}

#[derive(Clone, Default)]
struct ProvideCidConfigParams(Option<CidConfig>);

impl ProvideCidConfigParams {
	fn new(config: Option<CidConfig>) -> Self {
		Self(config)
	}
}

impl<T: Config> TxParams<T> for ProvideCidConfigParams {}

struct ProvideCidConfigExtension<T: Config> {
	config: Option<CidConfig>,
	_marker: PhantomData<T>,
}

impl<T: Config> ExtrinsicParams<T> for ProvideCidConfigExtension<T> {
	type Params = ProvideCidConfigParams;

	fn new(_client: &ClientState<T>, params: Self::Params) -> Result<Self, ExtrinsicParamsError> {
		Ok(Self { config: params.0, _marker: PhantomData })
	}
}

impl<T: Config> ExtrinsicParamsEncoder for ProvideCidConfigExtension<T> {
	fn encode_value_to(&self, v: &mut Vec<u8>) {
		self.config.encode_to(v);
	}
}

impl<T: Config> TransactionExtension<T> for ProvideCidConfigExtension<T> {
	type Decoded = Static<Option<CidConfig>>;

	fn matches(identifier: &str, _type_id: u32, _types: &PortableRegistry) -> bool {
		identifier == "ProvideCidConfig"
	}
}

type BulletinExtrinsicParams<T> = AnyOf<
	T,
	(
		subxt_core::config::transaction_extensions::VerifySignature<T>,
		CheckSpecVersion,
		CheckTxVersion,
		CheckNonce,
		CheckGenesis<T>,
		CheckMortality<T>,
		ChargeAssetTxPayment<T>,
		ChargeTransactionPayment,
		CheckMetadataHash,
		ProvideCidConfigExtension<T>,
	),
>;

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
enum BulletinConfig {}

impl Config for BulletinConfig {
	type AccountId = <PolkadotConfig as Config>::AccountId;
	type Signature = <PolkadotConfig as Config>::Signature;
	type Hasher = <PolkadotConfig as Config>::Hasher;
	type Header = <PolkadotConfig as Config>::Header;
	type AssetId = <PolkadotConfig as Config>::AssetId;
	type Address = <PolkadotConfig as Config>::Address;
	type ExtrinsicParams = BulletinExtrinsicParams<Self>;
}

fn bytes_from_scale_value(value: ScaleValue<u32>) -> Option<Vec<u8>> {
	match value.value {
		ScaleValueDef::Composite(ScaleComposite::Unnamed(values)) => {
			let mut bytes = Vec::with_capacity(values.len());
			for item in values {
				let byte = item.as_u128()? as u8;
				bytes.push(byte);
			}
			Some(bytes)
		},
		_ => None,
	}
}

fn extract_content_hash(fields: ScaleComposite<u32>) -> Option<[u8; 32]> {
	let ScaleComposite::Named(values) = fields else {
		return None;
	};
	let value = values
		.into_iter()
		.find(|(name, _)| name == "content_hash")
		.map(|(_, value)| value)?;
	let bytes = bytes_from_scale_value(value)?;
	bytes.try_into().ok()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let ws_url = env::args().nth(1).unwrap_or_else(|| "ws://localhost:10000".to_string());

	println!("Connecting to: {ws_url}");

	let client = OnlineClient::<BulletinConfig>::from_url(ws_url.clone()).await?;
	let sudo_client = OnlineClient::<PolkadotConfig>::from_url(ws_url).await?;

	let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
	let data = format!("Hello, Bulletin with subxt preimage - {now}");
	let data_bytes = data.as_bytes().to_vec();
	let content_hash = blake2_256(&data_bytes);

	// Authorize the preimage using sudo.
	let authorize_preimage = subxt::dynamic::tx(
		"TransactionStorage",
		"authorize_preimage",
		vec![Value::from_bytes(content_hash), Value::u128(data_bytes.len() as u128)],
	);
	let sudo_call = subxt::dynamic::tx("Sudo", "sudo", vec![authorize_preimage.into_value()]);

	let sudo_signer = dev::alice();
	sudo_client
		.tx()
		.sign_and_submit_then_watch_default(&sudo_call, &sudo_signer)
		.await?
		.wait_for_finalized_success()
		.await?;

	// Submit the store call as an unsigned authorized transaction.
	let store_call =
		subxt::dynamic::tx("TransactionStorage", "store", vec![Value::from_bytes(&data_bytes)]);

	let metadata = client.metadata();
	let state = ClientState::<BulletinConfig> {
		metadata: metadata.clone(),
		genesis_hash: client.genesis_hash(),
		runtime_version: client.runtime_version(),
	};

	let supported_versions = metadata.extrinsic().supported_versions();
	if !supported_versions.contains(&5) {
		return Err("Transaction version v5 is required for AuthorizeCall flow".into());
	}

	let (
		verify_sig,
		check_spec,
		check_tx,
		check_nonce,
		check_genesis,
		check_mortality,
		charge_asset,
		charge_tx,
		check_metadata,
	) = DefaultExtrinsicParamsBuilder::<BulletinConfig>::new()
		.immortal()
		.nonce(0)
		.build();
	let cid_config = Some(CidConfig { codec: 0x55, hashing: CidHashingAlgorithm::Blake2b256 });
	let params = (
		verify_sig,
		check_spec,
		check_tx,
		check_nonce,
		check_genesis,
		check_mortality,
		charge_asset,
		charge_tx,
		check_metadata,
		ProvideCidConfigParams::new(cid_config),
	);
	let partial = tx::create_v5_general(&store_call, &state, params)?;
	let tx = partial.to_transaction();
	let submittable = SubmittableTransaction::from_bytes(client.clone(), tx.into_encoded());
	let events = submittable.submit_and_watch().await?.wait_for_finalized_success().await?;
	let mut found = false;
	for event in events.iter() {
		let event = event?;
		if event.pallet_name() == "TransactionStorage" && event.variant_name() == "Stored" {
			if let Some(event_hash) = extract_content_hash(event.field_values()?) {
				if event_hash == content_hash {
					found = true;
					break;
				}
			}
		}
	}
	if !found {
		return Err("Stored event with matching content hash not found".into());
	}

	println!("✅ Preimage authorized unsigned store succeeded (event verified)");
	Ok(())
}
