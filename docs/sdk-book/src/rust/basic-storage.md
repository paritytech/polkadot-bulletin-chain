# Basic Storage

This guide shows how to store data using the `AsyncBulletinClient` with transaction submitters.

## Quick Start

The `store()` method uses a builder pattern for a clean, fluent API:

### Why Builder Pattern?

The builder pattern provides:
- **Fluent API**: Chain methods for clean, readable code
- **Type safety**: Compile-time validation of options
- **Discoverability**: IDE autocomplete shows all available options
- **Flexibility**: Only specify options you need
- **Clarity**: Intent is clear from method names

Compare:
```rust
// Old API (still available but deprecated)
client.store_with_options(data, StoreOptions { ... }, Some(callback)).await?;

// New builder API (recommended)
client
    .store(data)
    .with_codec(CidCodec::DagPb)
    .with_callback(callback)
    .send()
    .await?;
```

### Basic Example

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Connect to Bulletin Chain
    let ws_url = std::env::var("BULLETIN_WS_URL")
        .unwrap_or_else(|_| "ws://localhost:10000".to_string());
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
    println!("Stored successfully!");
    println!("  CID: {}", hex::encode(&result.cid));
    println!("  Size: {} bytes", result.size);

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
        println!("Progress: {:?}", event);
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
println!("CID: {}", hex::encode(&result.cid));
println!("Size: {} bytes", result.size);
println!("Block: {:?}", result.block_number);
```

## Complete Example

```rust
use bulletin_sdk_rust::prelude::*;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    println!("✅ Stored successfully!");
    println!("   CID: {}", hex::encode(&result.cid));
    println!("   Size: {} bytes", result.size);

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
let account = AccountId32::from(/* your account */);

let client = AsyncBulletinClient::new(submitter)
    .with_account(account);  // Set the account for auth checking

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

### Disable Authorization Checking

You might want to skip pre-flight authorization checking in these scenarios:

- **Performance**: Avoid extra query when doing many sequential uploads (authorization was already verified)
- **Testing**: Test on-chain authorization validation directly
- **Offline preparation**: Prepare transactions offline for later broadcast
- **Batch operations**: Already checked authorization once for the entire batch

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
        eprintln!("Need {} bytes but only {} available", need, available);
        eprintln!("Please authorize your account first!");
    }
    Ok(result) => {
        println!("Success!");
    }
    Err(e) => {
        eprintln!("Error: {:?}", e);
    }
}
```

### Complete Example with Authorization

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    println!("Need authorization for {} txs and {} bytes", txs, bytes);

    // Authorize (if needed)
    // client.authorize_account(account, txs, bytes).await?;

    // Store - will check authorization automatically
    match client.store(data).send().await {
        Ok(result) => {
            println!("✅ Stored: {}", hex::encode(&result.cid));
        }
        Err(Error::InsufficientAuthorization { need, available }) => {
            eprintln!("❌ Insufficient authorization:");
            eprintln!("   Need: {} bytes", need);
            eprintln!("   Have: {} bytes", available);
            return Err("Please authorize your account first".into());
        }
        Err(e) => {
            eprintln!("❌ Error: {:?}", e);
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
        println!("Success! CID: {}", hex::encode(&result.cid));
    }
    Err(Error::EmptyData) => {
        eprintln!("Error: Cannot store empty data");
    }
    Err(Error::SubmissionFailed(msg)) => {
        eprintln!("Submission failed: {}", msg);
    }
    Err(e) => {
        eprintln!("Unexpected error: {:?}", e);
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

let client = BulletinClient::new();
let operation = client.prepare_store(data, options)?;

println!("CID: {}", hex::encode(&operation.cid_bytes));
println!("Data to submit: {} bytes", operation.data.len());
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
