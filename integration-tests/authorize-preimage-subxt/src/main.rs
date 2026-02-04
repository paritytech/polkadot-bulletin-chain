use std::{
	env,
	time::{SystemTime, UNIX_EPOCH},
};

use scale_value::{Composite as ScaleComposite, Value as ScaleValue, ValueDef as ScaleValueDef};
use sp_core::hashing::blake2_256;
use subxt::{config::PolkadotConfig, dynamic::Value, tx::SubmittableTransaction, OnlineClient};
use subxt_core::{client::ClientState, config::DefaultExtrinsicParamsBuilder, tx};
use subxt_signer::sr25519::dev;

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

	let client = OnlineClient::<PolkadotConfig>::from_url(ws_url).await?;

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
	client
		.tx()
		.sign_and_submit_then_watch_default(&sudo_call, &sudo_signer)
		.await?
		.wait_for_finalized_success()
		.await?;

	// Submit the store call as an unsigned authorized transaction.
	let store_call =
		subxt::dynamic::tx("TransactionStorage", "store", vec![Value::from_bytes(&data_bytes)]);

	let metadata = client.metadata();
	let state = ClientState::<PolkadotConfig> {
		metadata: metadata.clone(),
		genesis_hash: client.genesis_hash(),
		runtime_version: client.runtime_version(),
	};

	let supported_versions = metadata.extrinsic().supported_versions();
	if !supported_versions.contains(&5) {
		return Err("Transaction version v5 is required for AuthorizeCall flow".into());
	}

	let params = DefaultExtrinsicParamsBuilder::<PolkadotConfig>::new()
		.immortal()
		.nonce(0)
		.build();
	let partial = tx::create_v5_general(&store_call, &state, params)?;
	let tx = partial.to_transaction();
	let submittable = SubmittableTransaction::from_bytes(client.clone(), tx.into_encoded());
	let in_block = submittable.submit_and_watch().await?.wait_for_finalized_success().await?;

	let events = in_block.fetch_events().await?;
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
