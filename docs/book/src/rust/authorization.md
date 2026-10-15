# Authorization

Before storing data on Bulletin Chain, accounts must be authorized with a transaction quota and byte allowance.

## Using TransactionClient (Recommended)

`TransactionClient` provides methods for all authorization operations:

### Authorize an Account

```rust
use bulletin_sdk_rust::prelude::*;
use subxt_signer::sr25519::Keypair;
use std::str::FromStr;
use tracing::info;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let client = TransactionClient::new("ws://localhost:10000").await?;
    let uploader = Keypair::from_uri(&subxt_signer::SecretUri::from_str("//Alice")?)?;
    let authorizer = Keypair::from_uri(&subxt_signer::SecretUri::from_str("//Eve")?)?;
    let account = subxt::utils::AccountId32::from(uploader.public_key().0);

    // Authorize: 10 transactions, 10 MiB
    let receipt = client.authorize_account(
        account,
        10,                   // transactions
        10 * 1024 * 1024,     // bytes
        &authorizer,          // a registered authorizer (//Eve in the dev preset)
        WaitFor::InBlock,
    ).await?;

    info!("Authorization granted in block: {}", receipt.block_hash);
    Ok(())
}
```

### Authorize a Preimage

Authorize specific content by its hash (for unsigned submissions):

```rust
let receipt = client.authorize_preimage(
    content_hash,     // ContentHash of the data
    max_size,         // Maximum size in bytes
    &signer,
    WaitFor::InBlock,
).await?;
```

### Check Authorization

Query the current authorization status before submitting a store transaction.
The check reads the best block, so an authorization included with
`WaitFor::InBlock` passes it right away. To read the finalized block instead,
build the client with `with_authorization_confirmation(WaitFor::Finalized)`.
`store` runs the same check before it submits.

The transaction pool validates a new transaction against its view of the newest
best block, and it builds that view a moment after the block's `InBlock` events
fire. A store sent by a newly authorized account in that moment can fail as an
invalid transaction; sending it again succeeds.

```rust
let account = subxt::utils::AccountId32::from(signer.public_key().0);

match client.check_authorization_for_store(&account, 1, data.len() as u64).await {
    Ok(()) => {
        tracing::info!("Authorization sufficient");
    }
    Err(Error::AuthorizationNotFound(_)) => {
        tracing::error!("No authorization found — call authorize_account() first");
    }
    Err(Error::InsufficientAuthorization { need, available }) => {
        // Transactions are checked first: `need`/`available` count transactions
        // when the transaction quota is short, bytes otherwise.
        tracing::error!(need, available, "Insufficient authorization");
    }
    Err(e) => {
        tracing::error!(?e, "Authorization check failed");
    }
}
```

### Refresh Authorization

Extend the expiry of an existing authorization:

```rust
// Refresh account authorization
client.refresh_account_authorization(account, &signer, WaitFor::InBlock).await?;

// Refresh preimage authorization
client.refresh_preimage_authorization(content_hash, &signer, WaitFor::InBlock).await?;
```

### Remove Expired Authorization

Clean up expired authorizations:

```rust
client.remove_expired_account_authorization(account, &signer, WaitFor::InBlock).await?;
client.remove_expired_preimage_authorization(content_hash, &signer, WaitFor::InBlock).await?;
```

## Estimating Authorization

`estimate_upload` returns the exact `transactions` and `bytes` of an upload plan (chunks plus manifest):

```rust
use bulletin_sdk_rust::prelude::*;
use std::sync::Arc;

let source: Arc<dyn SeekableSource> = Arc::new(blob_from_bytes(file_bytes));
let estimate = client
    .estimate_upload(UploadInput::Source(source), UploadEstimateOptions::default())
    .await?;
tracing::info!(
    transactions = estimate.base.transactions,
    bytes = estimate.base.bytes,
    "Authorization needed"
);
```

`BulletinClient::estimate_authorization(size)` is an offline approximation: it assumes 1 MiB chunks and estimates the manifest as 10 bytes per chunk plus 1000 bytes.

## Automatic Pre-Store Check

`TransactionClient.store()` automatically checks authorization before submitting a transaction. If the account lacks sufficient authorization, it returns an `InsufficientAuthorization` or `AuthorizationNotFound` error immediately, without submitting a transaction. On runtimes without the `BulletinTransactionStorageApi` runtime API the check is skipped.

```rust
match client.store(data, &signer, WaitFor::InBlock).await {
    Ok(receipt) => {
        tracing::info!("Stored in block: {}", receipt.block_hash);
    }
    Err(Error::InsufficientAuthorization { need, available }) => {
        tracing::error!(need, available, "Need more authorization");
    }
    Err(Error::AuthorizationNotFound(_)) => {
        tracing::error!("No authorization — call authorize_account() first");
    }
    Err(e) => {
        tracing::error!(?e, "Store failed");
    }
}
```

## Two-Step Approach (Advanced)

When using `BulletinClient` with your own subxt client, submit authorization transactions manually. The SDK's generated chain module is private, so generate your own from the chain metadata (for example `sdk/metadata.scale`):

```rust
#[subxt::subxt(runtime_metadata_path = "metadata.scale")]
pub mod bulletin {}

let tx = bulletin::tx().transaction_storage().authorize_account(
    target_account,
    txs,
    bytes,
);

let result = api.tx()
    .sign_and_submit_then_watch_default(&tx, &signer)
    .await?;
```

## Next Steps

- [Basic Storage](./basic-storage.md) - Store data after authorization
- [Chunked Uploads](./chunked-uploads.md) - Large file handling
- [Error Handling](./error-handling.md) - Authorization error codes and recovery
