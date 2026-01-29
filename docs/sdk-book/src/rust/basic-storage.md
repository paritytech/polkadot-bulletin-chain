# Basic Storage

This guide shows how to store a small piece of data (< 8 MiB) using the `AsyncBulletinClient` with transaction submitters.

## Quick Start

For small data that fits in a single transaction (< 8 MiB):

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
    let result = client.store(data, StoreOptions::default()).await?;

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

The `store()` method does everything:
- Validates data size
- Calculates CID
- Submits transaction
- Waits for finalization

```rust
let result = client.store(data, options).await?;
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
        .store(data.as_bytes().to_vec(), StoreOptions::default())
        .await?;

    println!("✅ Stored successfully!");
    println!("   CID: {}", hex::encode(&result.cid));
    println!("   Size: {} bytes", result.size);

    Ok(())
}
```

## Error Handling

```rust
match client.store(data, options).await {
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
        let result = client.store(data, StoreOptions::default()).await;

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
