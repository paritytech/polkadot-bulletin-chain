# Basic Storage

This guide shows how to store data using the `AsyncBulletinClient` with transaction submitters.

## Quick Start

The `store()` method automatically handles both small and large files:

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

    // 2. Prepare and store data
    let data = b"Hello, Bulletin!".to_vec();
    let result = client.store(data, None).await?;

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

### 4. Configure Options

Customize CID generation:

```rust
let options = StoreOptions {
    cid_codec: CidCodec::Raw,           // or DagPb, DagCbor
    hash_algorithm: HashAlgorithm::Blake2b256, // or Sha2_256, etc.
};
```

Or use defaults:

```rust
let options = StoreOptions::default(); // Raw codec, Blake2b-256
```

### 5. Store and Wait

The `store()` method automatically handles everything:
- Validates data size
- Checks authorization (if configured)
- Automatically chunks large files (> 2 MiB by default)
- Calculates CID(s)
- Submits transaction(s)
- Waits for finalization

```rust
// For small files (< 2 MiB): single transaction
// For large files (> 2 MiB): automatic chunking
let result = client.store(data, None).await?;

// With custom options (advanced users)
let result = client.store_with_options(data, options, None).await?;

// With progress tracking for large files
let result = client.store(data, Some(|event| {
    match event {
        ProgressEvent::ChunkCompleted { index, total, .. } => {
            println!("Chunk {}/{} uploaded", index + 1, total);
        }
        ProgressEvent::Completed { .. } => {
            println!("Upload complete!");
        }
        _ => {}
    }
})).await?;
```

### 6. Handle Result

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

    // Store data
    let data = format!("Hello from Rust SDK at {}", chrono::Utc::now());
    let result = client
        .store(data.as_bytes().to_vec(), None)
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
let result = client.store(data, None).await?;
//                       ⬆️ Queries blockchain first, fails fast if insufficient auth
```

### What Gets Checked

Before submitting the transaction, the SDK:
1. **Queries** the blockchain for your current authorization
2. **Validates** you have enough transactions and bytes authorized
3. **Fails immediately** if insufficient (no transaction fees wasted!)
4. **Proceeds** only if authorization is sufficient

### Disable Authorization Checking

If you want to skip the check (e.g., you know authorization exists):

```rust
use bulletin_sdk_rust::async_client::AsyncClientConfig;

let mut config = AsyncClientConfig::default();
config.check_authorization_before_upload = false;  // Disable checking

let client = AsyncBulletinClient::with_config(submitter, config)
    .with_account(account);
```

### Error Example

```rust
// Insufficient authorization fails fast
match client.store(data, None).await {
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
    match client.store(data, None).await {
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
match client.store(data, None).await {
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
