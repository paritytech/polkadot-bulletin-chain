use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use sp_core::hashing::blake2_256;
use subxt::config::PolkadotConfig;
use subxt::dynamic::Value;
use subxt::tx::SubmittableTransaction;
use subxt::{OnlineClient};
use subxt_core::client::ClientState;
use subxt_core::config::DefaultExtrinsicParamsBuilder;
use subxt_core::tx;
use subxt_signer::sr25519::dev;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ws_url = env::args()
        .nth(1)
        .unwrap_or_else(|| "ws://localhost:10000".to_string());

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
        vec![
            Value::from_bytes(content_hash),
            Value::u128(data_bytes.len() as u128),
        ],
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
    let store_call = subxt::dynamic::tx(
        "TransactionStorage",
        "store",
        vec![Value::from_bytes(&data_bytes)],
    );

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
    submittable
        .submit_and_watch()
        .await?
        .wait_for_finalized_success()
        .await?;

    println!("✅ Preimage authorized unsigned store succeeded");
    Ok(())
}


