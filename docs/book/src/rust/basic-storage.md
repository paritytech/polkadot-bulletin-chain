# Basic Storage

This guide shows how to store data on Bulletin Chain using the SDK.

> **Note on Logging**: All examples use `tracing` for structured logging. If you're integrating with Substrate runtime/node code, you can use `sp_tracing` instead for better compatibility with Substrate's logging infrastructure.

> **Complete Working Example**: See [`examples/rust/authorize-and-store`](https://github.com/paritytech/polkadot-bulletin-chain/tree/main/examples/rust/authorize-and-store) for a complete runnable example that demonstrates authorization, storage, and chunked uploads with DAG-PB manifests.

## Using TransactionClient (Recommended)

The simplest way to store data is using `TransactionClient`:

```rust
use bulletin_sdk_rust::prelude::*;
use subxt_signer::sr25519::Keypair;
use std::str::FromStr;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Connect to Bulletin Chain
    let client = TransactionClient::new("ws://localhost:10000").await?;
    info!("Connected to Bulletin Chain");

    // Create signer from seed phrase or dev account
    let uri = subxt_signer::SecretUri::from_str("//Alice")?;
    let signer = Keypair::from_uri(&uri)?;
    let account = subxt::utils::AccountId32::from(signer.public_key().0);

    // Step 1: Authorize account (requires sudo/admin privileges)
    let auth_receipt = client.authorize_account(
        account.clone(),
        10,                    // 10 transactions
        10 * 1024 * 1024,      // 10 MiB
        &signer,
    ).await?;
    info!("Account authorized in block: {}", auth_receipt.block_hash);

    // Step 2: Store data
    let data = b"Hello, Bulletin!".to_vec();
    let receipt = client.store(data.clone(), &signer).await?;

    info!("Stored {} bytes in block: {}", receipt.data_size, receipt.block_hash);
    info!("Extrinsic hash: {}", receipt.extrinsic_hash);

    Ok(())
}
```

### With Progress Tracking

Track transaction progress with callbacks:

```rust
use std::sync::Arc;

let receipt = client.store_with_progress(
    data,
    &signer,
    Some(Arc::new(|event| {
        match event {
            ProgressEvent::Transaction(status) => {
                info!("Transaction status: {:?}", status);
            }
            ProgressEvent::Chunk(chunk_event) => {
                info!("Chunk progress: {:?}", chunk_event);
            }
        }
    })),
).await?;
```

### Pre-Calculate CID

Use `BulletinClient` to calculate the CID before submission:

```rust
use bulletin_sdk_rust::prelude::*;

// BulletinClient for local operations (no network)
let sdk_client = BulletinClient::new();
let options = StoreOptions {
    cid_codec: CidCodec::Raw,
    hash_algorithm: HashingAlgorithm::Blake2b256,
    wait_for_finalization: true,
};

let operation = sdk_client.prepare_store(data.clone(), options)?;
let cid_data = operation.calculate_cid()?;
let cid_bytes = cid_to_bytes(&cid_data)?;
info!("CID: {}", hex::encode(&cid_bytes));

// Then store using TransactionClient
let receipt = client.store(data, &signer).await?;
```

## Error Handling

```rust
use bulletin_sdk_rust::prelude::*;

match client.store(data, &signer).await {
    Ok(receipt) => {
        tracing::info!("Stored in block: {}", receipt.block_hash);
    }
    Err(Error::EmptyData) => {
        tracing::error!("Cannot store empty data");
    }
    Err(Error::InsufficientAuthorization { need, available }) => {
        tracing::error!(
            need_bytes = need,
            available_bytes = available,
            "Insufficient authorization - authorize your account first"
        );
    }
    Err(e) if e.is_retryable() => {
        tracing::warn!(
            code = e.code(),
            hint = e.recovery_hint(),
            "Transient error, consider retrying: {}",
            e
        );
    }
    Err(e) => {
        tracing::error!(code = e.code(), hint = e.recovery_hint(), "Error: {}", e);
    }
}
```

See the [Error Handling](./error-handling.md) guide for the full error reference.

## Two-Step Approach (Advanced)

If you need more control (e.g., CID before submission, batching, custom submission), use the two-step approach:

### Step 1: Prepare Operation

```rust
use bulletin_sdk_rust::prelude::*;

let client = BulletinClient::new();
let operation = client.prepare_store(data, StoreOptions::default())?;

tracing::info!(
    cid = %hex::encode(&operation.cid_bytes),
    size = operation.data.len(),
    "Prepared store operation"
);
```

### Step 2: Submit via Subxt

```rust
// Submit using your own subxt client
let tx = bulletin::tx().transaction_storage().store(
    operation.data,
);
let result = tx.sign_and_submit_then_watch_default(&api, &signer).await?;
```

This is useful when:
- You need the CID before submission
- You're batching multiple operations
- You're using a custom submission method (light client, etc.)

## Next Steps

- [Chunked Uploads](./chunked-uploads.md) - For files > 2 MiB
- [Authorization](./authorization.md) - Managing storage authorization
- [Error Handling](./error-handling.md) - Error codes, retry logic, recovery hints
