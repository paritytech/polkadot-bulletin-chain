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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = TransactionClient::new("ws://localhost:10000").await?;
    let uri = subxt_signer::SecretUri::from_str("//Alice")?;
    let signer = Keypair::from_uri(&uri)?;
    let account = subxt::utils::AccountId32::from(signer.public_key().0);

    // Authorize: 10 transactions, 10 MiB
    let receipt = client.authorize_account(
        account,
        10,                   // transactions
        10 * 1024 * 1024,     // bytes
        &signer,              // must have authorizer privileges
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
).await?;
```

### Check Authorization

Query the current authorization status before submitting a store transaction:

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
        tracing::error!(
            need_bytes = need,
            available_bytes = available,
            "Insufficient authorization"
        );
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
client.refresh_account_authorization(account, &signer).await?;

// Refresh preimage authorization
client.refresh_preimage_authorization(content_hash, &signer).await?;
```

### Remove Expired Authorization

Clean up expired authorizations:

```rust
client.remove_expired_account_authorization(account, &signer).await?;
client.remove_expired_preimage_authorization(content_hash, &signer).await?;
```

## Estimating Authorization

Use `BulletinClient` to calculate authorization requirements before submitting:

```rust
use bulletin_sdk_rust::prelude::*;

let client = BulletinClient::new();
let file_size = 100 * 1024 * 1024; // 100 MiB

// Automatically calculates transactions and bytes needed
// (accounts for chunking and manifest overhead)
let (txs, bytes) = client.estimate_authorization(file_size);
tracing::info!(transactions = txs, bytes = bytes, "Authorization needed");
```

## Automatic Pre-Store Check

`TransactionClient.store()` automatically checks authorization before submitting a transaction. If the account lacks sufficient authorization, it returns an `InsufficientAuthorization` or `AuthorizationNotFound` error immediately — without submitting or paying for a transaction.

```rust
match client.store(data, &signer).await {
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

When using `BulletinClient` with your own subxt client, submit authorization transactions manually:

```rust
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
