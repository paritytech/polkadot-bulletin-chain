# Basic Storage

This guide shows how to store data using the `AsyncBulletinClient` with transaction submitters.

> **Note on Logging**: All examples use `tracing` for structured logging. If you're integrating with Substrate runtime/node code, you can use `sp_tracing` instead for better compatibility with Substrate's logging infrastructure.

## Quick Start

The `store()` method uses a builder pattern for a clean, fluent API:

### Why Builder Pattern?

The builder pattern provides:
- **Fluent API**: Chain methods for clean, readable code
- **Type safety**: Compile-time validation of options
- **Discoverability**: IDE autocomplete shows all available options
- **Flexibility**: Only specify options you need
- **Clarity**: Intent is clear from method names

### Basic Example

```rust
use bulletin_sdk_rust::prelude::*;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber for logging
    tracing_subscriber::fmt::init();

    // 1. Connect to Bulletin Chain
    // Available endpoints (see shared/networks.json for full list):
    //   - Local:   ws://localhost:10000
    //   - Westend: wss://westend-bulletin-rpc.polkadot.io
    //   - Paseo:   wss://paseo-bulletin-rpc.polkadot.io
    let ws_url = std::env::var("BULLETIN_WS_URL")
        .unwrap_or_else(|_| "wss://paseo-bulletin-rpc.polkadot.io".to_string());
    let signer = /* your PairSigner */;

    let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
    let client = AsyncBulletinClient::new(submitter);

    // 2. Prepare and store data with builder pattern
    let data = b"Hello, Bulletin!".to_vec();
    let result = client
        .store(data)
        .send()
        .await?;

    // 3. Get results
    info!("Stored successfully!");
    info!(cid = %hex::encode(&result.cid), size = result.size, "Storage complete");

    Ok(())
}
```

## Step-by-Step Explanation

### 1. Setup Connection

First, create a transaction submitter. The submitter handles all blockchain communication:

```rust
use bulletin_sdk_rust::prelude::*;

// Option 1: From URL (recommended)
let ws_url = "ws://localhost:10000";
let submitter = SubxtSubmitter::from_url(ws_url, signer).await?;

// Option 2: For testing without a node
let submitter = MockSubmitter::new();
```

Learn more about submitters in the [Transaction Submitters](./submitters.md) guide.

### 2. Create Client

Wrap the submitter with `AsyncBulletinClient`:

```rust
let client = AsyncBulletinClient::new(submitter);
```

### 3. Prepare Data

```rust
let data = b"Hello, Bulletin!".to_vec();
```

### 4. Store with Builder Pattern

The `store()` method returns a builder for fluent configuration:

```rust
// Simple storage with defaults
let result = client
    .store(data)
    .send()
    .await?;

// Customize CID codec
let result = client
    .store(data)
    .with_codec(CidCodec::DagPb)
    .send()
    .await?;

// Customize hash algorithm
let result = client
    .store(data)
    .with_hash_algorithm(HashAlgorithm::Sha256)
    .send()
    .await?;

// Combine multiple options
let result = client
    .store(data)
    .with_codec(CidCodec::DagCbor)
    .with_hash_algorithm(HashAlgorithm::Blake2b256)
    .with_finalization(true)
    .send()
    .await?;

// With progress callback for chunked uploads
let result = client
    .store(large_data)
    .with_callback(|event| {
        tracing::debug!(?event, "Upload progress");
    })
    .send()
    .await?;
```

The builder automatically handles:
- Data validation
- Authorization checks (if configured)
- Automatic chunking for large files (> 2 MiB by default)
- CID calculation
- Transaction submission
- Finalization wait

### 5. Handle Result

```rust
tracing::info!(
    cid = %hex::encode(&result.cid),
    size = result.size,
    block = ?result.block_number,
    "Storage successful"
);
```

## Complete Example

```rust
use bulletin_sdk_rust::prelude::*;
use std::env;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber
    tracing_subscriber::fmt::init();

    // Get WebSocket URL from environment or CLI
    let ws_url = env::var("BULLETIN_WS_URL")
        .unwrap_or_else(|_| "ws://localhost:10000".to_string());

    // Create signer (use your actual signer)
    let signer = /* your PairSigner from keypair */;

    // Connect and create client
    let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
    let client = AsyncBulletinClient::new(submitter);

    // Store data with builder pattern
    let data = format!("Hello from Rust SDK at {}", chrono::Utc::now());
    let result = client
        .store(data.as_bytes().to_vec())
        .send()
        .await?;

    info!(
        cid = %hex::encode(&result.cid),
        size = result.size,
        "Stored successfully"
    );

    Ok(())
}
```

## Authorization Checking (Fail Fast)

By default, the SDK checks authorization **before** uploading to fail fast and avoid wasted transaction fees.

### How It Works

```rust
use bulletin_sdk_rust::prelude::*;

// 1. Create client with your account
let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;

// Note: In most cases, you can get the account from your signer/keypair.
// We pass it separately to support advanced scenarios (multisig, delegated auth, etc.)
let account = signer.account_id(); // Or AccountId32::from(keypair.public())

let client = AsyncBulletinClient::new(submitter)
    .with_account(account);  // Enable automatic authorization checking

// 2. Upload - authorization is checked automatically
let data = b"Hello, Bulletin!".to_vec();
let result = client
    .store(data)
    .send()
    .await?;
//  ⬆️ Queries blockchain first, fails fast if insufficient auth
```

### What Gets Checked

Before submitting the transaction, the SDK:
1. **Queries** the blockchain for your current authorization
2. **Validates** you have enough transactions and bytes authorized
3. **Fails immediately** if insufficient (no transaction fees wasted!)
4. **Proceeds** only if authorization is sufficient

### Disable Authorization Checking (Advanced)

> **Note**: Authorization checking is enabled by default and is recommended for most use cases. It prevents wasted transaction fees by failing early if you lack authorization.

In rare scenarios, you might want to skip pre-flight authorization checking:

- **High-frequency uploads**: When uploading many small files sequentially and the query overhead matters (< 100ms per upload). You've already verified authorization manually and trust it's sufficient.
- **Testing**: When you specifically want to test on-chain authorization validation errors.
- **Offline signing**: When preparing transactions offline for later broadcast where the blockchain isn't accessible.

**Trade-off**: Disabling the check saves ~100ms per upload but risks submitting transactions that fail on-chain (wasting transaction fees).

```rust
use bulletin_sdk_rust::async_client::AsyncClientConfig;

let mut config = AsyncClientConfig::default();
config.check_authorization_before_upload = false;  // Disable pre-flight checking

let client = AsyncBulletinClient::with_config(submitter, config)
    .with_account(account);

// Authorization will still be validated on-chain during transaction execution
```

### Error Example

```rust
// Insufficient authorization fails fast
match client.store(data).send().await {
    Err(Error::InsufficientAuthorization { need, available }) => {
        tracing::error!(
            need_bytes = need,
            available_bytes = available,
            "Insufficient authorization - please authorize your account first"
        );
    }
    Ok(result) => {
        tracing::info!("Storage successful");
    }
    Err(e) => {
        tracing::error!(?e, "Storage failed");
    }
}
```

### Complete Example with Authorization

```rust
use bulletin_sdk_rust::prelude::*;
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    let ws_url = std::env::var("BULLETIN_WS_URL")
        .unwrap_or_else(|_| "ws://localhost:10000".to_string());
    let signer = /* your signer */;
    let account = /* your AccountId32 */;

    // Connect
    let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
    let client = AsyncBulletinClient::new(submitter)
        .with_account(account.clone());

    // Estimate what's needed
    let data = std::fs::read("myfile.dat")?;
    let (txs, bytes) = client.estimate_authorization(data.len() as u64);
    info!(transactions = txs, bytes = bytes, "Authorization needed");

    // Authorize (if needed)
    // client.authorize_account(account, txs, bytes).await?;

    // Store - will check authorization automatically
    match client.store(data).send().await {
        Ok(result) => {
            info!(cid = %hex::encode(&result.cid), "Stored successfully");
        }
        Err(Error::InsufficientAuthorization { need, available }) => {
            error!(
                need_bytes = need,
                available_bytes = available,
                "Insufficient authorization - please authorize your account first"
            );
            return Err("Please authorize your account first".into());
        }
        Err(e) => {
            error!(?e, "Storage failed");
            return Err(e.into());
        }
    }

    Ok(())
}
```

## Error Handling

```rust
match client.store(data).send().await {
    Ok(result) => {
        tracing::info!(cid = %hex::encode(&result.cid), "Storage successful");
    }
    Err(Error::EmptyData) => {
        tracing::error!("Cannot store empty data");
    }
    Err(Error::SubmissionFailed(msg)) => {
        tracing::error!(reason = %msg, "Submission failed");
    }
    Err(e) => {
        tracing::error!(?e, "Unexpected error");
    }
}
```

## Testing Without a Node

Use `MockSubmitter` for unit tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bulletin_sdk_rust::prelude::*;

    #[tokio::test]
    async fn test_store() {
        let submitter = MockSubmitter::new();
        let client = AsyncBulletinClient::new(submitter);

        let data = b"test data".to_vec();
        let result = client.store(data, None).await;

        assert!(result.is_ok());
    }
}
```

## Next Steps

- [Chunked Uploads](./chunked-uploads.md) - For files > 8 MiB
- [Authorization](./authorization.md) - Managing storage authorization
- [Transaction Submitters](./submitters.md) - Deep dive into submitters

## Two-Step Approach (Advanced)

If you need more control, use the two-step approach:

### Step 1: Prepare Operation

```rust
use bulletin_sdk_rust::client::BulletinClient;
use tracing::info;

let client = BulletinClient::new();
let operation = client.prepare_store(data, options)?;

info!(
    cid = %hex::encode(&operation.cid_bytes),
    size = operation.data.len(),
    "Prepared store operation"
);
```

### Step 2: Submit Manually

```rust
// Submit using your own method
let receipt = submitter.submit_store(operation.data).await?;
```

This is useful when:
- You need the CID before submission
- You're batching multiple operations
- You're using a custom submission method
