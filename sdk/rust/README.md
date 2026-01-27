# Bulletin SDK for Rust

Off-chain client SDK for Polkadot Bulletin Chain with **complete transaction submission support**. Store data on Bulletin Chain with one SDK call - from data preparation to finalized transactions.

## Features

- **Complete Transaction Submission**: Handles everything from chunking to blockchain finalization
- **All 8 Pallet Operations**: Full support for store, authorize, renew, refresh, and cleanup operations
- **Automatic Chunking**: Split large files into optimal chunks (default 1 MiB)
- **DAG-PB Manifests**: IPFS-compatible manifest generation for chunked data
- **Authorization Management**: Built-in helpers for account and preimage authorization
- **Progress Tracking**: Callback-based progress events for uploads
- **no_std Compatible**: Core functionality works in constrained environments
- **ink! Support**: Use in smart contracts with the `ink` feature
- **Flexible Integration**: Trait-based design works with any signing method

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
bulletin-sdk-rust = "0.1"
```

For no_std environments:

```toml
[dependencies]
bulletin-sdk-rust = { version = "0.1", default-features = false }
```

For ink! smart contracts:

```toml
[dependencies]
bulletin-sdk-rust = { version = "0.1", default-features = false, features = ["ink"] }
```

## Quick Start

### Complete Store Workflow

```rust
use bulletin_sdk_rust::prelude::*;
use bulletin_sdk_rust::async_client::AsyncBulletinClient;
use bulletin_sdk_rust::submit::TransactionSubmitter;

// 1. Implement TransactionSubmitter with your signing method
struct MySubmitter { /* your subxt client */ }

#[async_trait::async_trait]
impl TransactionSubmitter for MySubmitter {
    async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt> {
        // Your subxt implementation
    }
    // ... implement other methods
}

// 2. Create client
let submitter = MySubmitter::new();
let client = AsyncBulletinClient::new(submitter);

// 3. Store data - complete workflow in one call!
let result = client.store(
    b"Hello, Bulletin!".to_vec(),
    StoreOptions::default(),
).await?;

println!("Stored with CID: {:?}", result.cid);
println!("Block number: {}", result.block_number.unwrap());
```

That's it! The SDK handles:
- ✅ CID calculation
- ✅ Transaction building
- ✅ Signing and submission
- ✅ Waiting for finalization
- ✅ Returning receipt with block info

## Complete API Reference

### All Supported Operations

```rust
// Store operations
client.store(data, options).await?;
client.store_chunked(data, config, options, progress).await?;

// Authorization operations (requires sudo)
client.authorize_account(who, transactions, bytes).await?;
client.authorize_preimage(hash, max_size).await?;
client.refresh_account_authorization(who).await?;
client.refresh_preimage_authorization(hash).await?;

// Maintenance operations
client.renew(block, index).await?;
client.remove_expired_account_authorization(who).await?;
client.remove_expired_preimage_authorization(hash).await?;

// Utilities
let (txs, bytes) = client.estimate_authorization(data_size);
```

## Usage Examples

### 1. Simple Store (< 8 MiB)

```rust
use bulletin_sdk_rust::{async_client::AsyncBulletinClient, prelude::*};

let client = AsyncBulletinClient::new(submitter);
let data = b"Hello, Bulletin!".to_vec();

// Store and wait for finalization
let result = client.store(data, StoreOptions::default()).await?;

println!("✅ Stored!");
println!("   CID: {:?}", result.cid);
println!("   Size: {} bytes", result.size);
println!("   Block: {}", result.block_number.unwrap());
```

### 2. Chunked Store (Large Files)

```rust
use bulletin_sdk_rust::{
    async_client::{AsyncBulletinClient, AsyncClientConfig},
    prelude::*,
};

// Configure chunking
let config = AsyncClientConfig {
    default_chunk_size: 1024 * 1024, // 1 MiB
    max_parallel: 8,
    create_manifest: true,
};

let client = AsyncBulletinClient::with_config(submitter, config);

// Read large file
let data = std::fs::read("large_video.mp4")?;

// Store with progress tracking
let result = client.store_chunked(
    &data,
    None, // use default config
    StoreOptions::default(),
    Some(|event| {
        match event {
            ProgressEvent::ChunkCompleted { index, total, cid } => {
                println!("Chunk {}/{} uploaded", index + 1, total);
            },
            ProgressEvent::ManifestCreated { cid } => {
                println!("Manifest CID: {:?}", cid);
            },
            _ => {}
        }
    }),
).await?;

println!("✅ Uploaded {} chunks", result.num_chunks);
println!("   Manifest CID: {:?}", result.manifest_cid);
```

### 3. Authorization Management

```rust
use bulletin_sdk_rust::async_client::AsyncBulletinClient;
use sp_runtime::AccountId32;

let client = AsyncBulletinClient::new(submitter);

// Estimate authorization needed
let (transactions, bytes) = client.estimate_authorization(10_000_000); // 10 MB

// Authorize account (requires sudo)
client.authorize_account(
    account_id,
    transactions,
    bytes,
).await?;

println!("✅ Account authorized for {} transactions", transactions);

// Refresh before expiry
client.refresh_account_authorization(account_id).await?;
```

### 4. Content-Addressed Authorization

```rust
// Authorize specific content by hash
let data = b"Specific content to authorize";
let content_hash = sp_io::hashing::blake2_256(data);

// Authorize preimage (requires sudo)
client.authorize_preimage(
    content_hash,
    data.len() as u64,
).await?;

// Now anyone can store this specific content
let result = client.store(data.to_vec(), StoreOptions::default()).await?;
```

### 5. Renew Stored Data

```rust
// Extend retention period for stored data
client.renew(block_number, transaction_index).await?;

println!("✅ Data retention period extended");
```

### 6. Custom CID Configuration

```rust
let options = StoreOptions {
    cid_codec: CidCodec::DagPb,
    hash_algorithm: HashAlgorithm::Sha2_256,
    wait_for_finalization: true,
};

let result = client.store(data, options).await?;
```

## Integration with subxt

The SDK uses a trait-based approach, allowing you to integrate with any signing method. Here's a complete subxt implementation:

```rust
use bulletin_sdk_rust::submit::{TransactionReceipt, TransactionSubmitter};
use subxt::{OnlineClient, PolkadotConfig, tx::PairSigner};
use sp_core::sr25519::Pair;

#[subxt::subxt(runtime_metadata_path = "metadata.scale")]
pub mod bulletin {}

struct SubxtSubmitter {
    api: OnlineClient<PolkadotConfig>,
    signer: PairSigner<PolkadotConfig, Pair>,
}

#[async_trait::async_trait]
impl TransactionSubmitter for SubxtSubmitter {
    async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt> {
        let tx = bulletin::tx().transaction_storage().store(data);

        let result = self.api
            .tx()
            .sign_and_submit_then_watch_default(&tx, &self.signer)
            .await?
            .wait_for_finalized_success()
            .await?;

        Ok(TransactionReceipt {
            block_hash: format!("{:?}", result.block_hash()),
            extrinsic_hash: format!("{:?}", result.extrinsic_hash()),
            block_number: None,
        })
    }

    // Implement other methods similarly...
}
```

## Architecture

The SDK is organized into modular components:

- **types**: Core types and errors
- **chunker**: Data chunking utilities
- **cid**: CID calculation and utilities (re-exports from pallet)
- **dag**: DAG-PB manifest building
- **authorization**: Authorization management helpers
- **storage**: Storage operation builders
- **submit**: Transaction submission traits
- **async_client**: High-level async client with full transaction support
- **client**: Preparation-only client (for advanced use cases)

## Examples

See the `examples/` directory for complete working examples:

- **`simple_store.rs`**: Basic data storage with transaction submission
- **`chunked_store.rs`**: Large file upload with progress tracking
- **`authorization_management.rs`**: All authorization operations

Run examples:

```bash
# Simple store
cargo run --example simple_store --features std

# Chunked store with file
cargo run --example chunked_store --features std large_file.bin

# Authorization management
cargo run --example authorization_management --features std
```

## Feature Flags

- **`std`** (default): Enable standard library support and async client
- **`ink`**: Enable ink! smart contract compatibility
- **`serde-support`**: Enable serialization support for DAG structures

## no_std Support

Core functionality is no_std compatible:

```rust
#![no_std]

use bulletin_sdk_rust::prelude::*;

// All core functionality available
let chunker = FixedSizeChunker::default_config();
let chunks = chunker.chunk(data)?;
```

## Testing

Run all tests:

```bash
cargo test --all-features
```

Run tests in no_std mode:

```bash
cargo test --no-default-features
```

## Before vs After

**Before (manual integration):**
```rust
// 1. Calculate CID manually
let cid = calculate_cid(data)?;

// 2. Build transaction manually
let tx = api.tx().transaction_storage().store(data);

// 3. Sign and submit manually
let result = api.tx()
    .sign_and_submit_then_watch_default(&tx, &signer)
    .await?
    .wait_for_finalized_success()
    .await?;

// 4. Process result manually
let block_hash = result.block_hash();
```

**After (SDK handles everything):**
```rust
let result = client.store(data, StoreOptions::default()).await?;
// Done! CID calculated, transaction submitted, finalized, receipt returned
```

## Best Practices

1. **Authorization First**: Authorize accounts before storing to ensure capacity
2. **Account vs Preimage**: Use account authorization for dynamic content, preimage for known content
3. **Refresh Early**: Refresh authorizations before they expire to maintain access
4. **Renew Important Data**: Renew stored data before retention period ends
5. **Clean Up**: Remove expired authorizations to free storage
6. **Progress Tracking**: Use callbacks for long uploads to provide user feedback
7. **Error Handling**: Always handle errors appropriately

## Performance Tips

- **Chunk Size**: Default 1 MiB works well; adjust for your network conditions
- **Parallel Uploads**: Increase `max_parallel` for faster uploads (default: 8)
- **Manifests**: Enable for files > 8 MiB to allow IPFS gateway retrieval
- **Batch Operations**: Use `store_chunked` for multiple chunks in one call

## Troubleshooting

**Q: Transaction fails with "InsufficientAuthorization"**
A: Ensure the account is authorized first using `authorize_account()` or `authorize_preimage()`

**Q: Chunk upload fails with "ChunkTooLarge"**
A: Each chunk must be ≤ 8 MiB. Use SDK's automatic chunking with `store_chunked()`

**Q: Can't retrieve via IPFS**
A: Ensure `create_manifest: true` was used for chunked uploads

**Q: Authorization expired**
A: Refresh authorizations using `refresh_account_authorization()` before they expire

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
