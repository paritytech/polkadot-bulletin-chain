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
        WaitFor::Finalized,
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

`check_authorization_for_store` checks that the account has an unexpired
authorization with enough transactions and bytes left for a store. The chain
accepts a store beyond the allowance at lower priority (see
[Authorization](../concepts/authorization.md)), so `store` checks only that an
unexpired authorization exists. Call `check_authorization_for_store` to decide
whether to top up the authorization before an upload.

Both checks read the finalized block, so authorize with `WaitFor::Finalized`
before the first store. A client built with `with_wait_for(WaitFor::InBlock)`
reads the best block instead, and an authorization included with
`WaitFor::InBlock` then passes the checks right away.

With `WaitFor::InBlock`, note that the transaction pool validates a new
transaction against its view of the newest best block, and it builds that view a
moment after the block's `InBlock` events fire. A store sent by a newly
authorized account in that moment can fail as an invalid transaction; sending it
again succeeds.

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

`TransactionClient.store()` checks that the account has an unexpired authorization before it submits a transaction. Without one, it returns `AuthorizationNotFound` and does not submit. Like the chain and the TypeScript `.ensureAuthorized()`, it does not reject a store beyond the allowance. On runtimes without the `BulletinTransactionStorageApi` runtime API the check is skipped.

```rust
match client.store(data, &signer, WaitFor::InBlock).await {
    Ok(receipt) => {
        tracing::info!("Stored in block: {}", receipt.block_hash);
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
