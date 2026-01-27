# Bulletin SDK for Rust

Off-chain client SDK for Polkadot Bulletin Chain that simplifies data storage with automatic chunking, authorization management, and DAG-PB manifest generation.

## Features

- **Automatic Chunking**: Split large files into optimal chunks (default 1 MiB)
- **DAG-PB Manifests**: IPFS-compatible manifest generation for chunked data
- **Authorization Management**: Helper functions for account and preimage authorization
- **Progress Tracking**: Callback-based progress events for uploads
- **no_std Compatible**: Core functionality works in no_std environments
- **ink! Support**: Use in smart contracts with the `ink` feature

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

## Usage

### Simple Store (< 8 MiB)

```rust
use bulletin_sdk_rust::prelude::*;

let client = BulletinClient::new();
let data = b"Hello, Bulletin!".to_vec();
let options = StoreOptions::default();

// Prepare the storage operation
let operation = client.prepare_store(data, options)?;

// Calculate CID
let cid_data = operation.calculate_cid()?;
println!("CID: {:?}", cid_data);

// Submit operation.data using subxt to TransactionStorage.store
// See examples/ for full integration
```

### Chunked Store (large files)

```rust
use bulletin_sdk_rust::prelude::*;

let client = BulletinClient::new();
let large_data = vec![0u8; 100_000_000]; // 100 MB

let config = ChunkerConfig {
    chunk_size: 1024 * 1024, // 1 MiB
    max_parallel: 8,
    create_manifest: true,
};

let (batch, manifest) = client.prepare_store_chunked(
    &large_data,
    Some(config),
    StoreOptions::default(),
    Some(|event| {
        match event {
            ProgressEvent::ChunkStarted { index, total } => {
                println!("Starting chunk {}/{}", index + 1, total);
            },
            ProgressEvent::ChunkCompleted { index, total, cid } => {
                println!("Completed chunk {}/{}: {:?}", index + 1, total, cid);
            },
            ProgressEvent::ManifestCreated { cid } => {
                println!("Manifest created: {:?}", cid);
            },
            _ => {}
        }
    }),
)?;

// Submit each chunk in batch.operations using subxt
// Then submit the manifest data if present
```

### Authorization Estimation

```rust
use bulletin_sdk_rust::prelude::*;

let client = BulletinClient::new();

// Estimate authorization needed for 100 MB
let (num_transactions, total_bytes) = client.estimate_authorization(100_000_000);
println!("Need authorization for {} transactions, {} bytes", num_transactions, total_bytes);

// Use these values to call TransactionStorage.authorize_account
```

### Custom CID Configuration

```rust
use bulletin_sdk_rust::prelude::*;

let options = StoreOptions {
    cid_codec: CidCodec::DagPb,
    hash_algorithm: HashAlgorithm::Sha2_256,
    wait_for_finalization: true,
};

let client = BulletinClient::new();
let operation = client.prepare_store(b"data".to_vec(), options)?;
```

## Architecture

The SDK is organized into several modules:

- `types`: Core types and errors
- `chunker`: Data chunking utilities
- `cid`: CID calculation and utilities
- `dag`: DAG-PB manifest building
- `authorization`: Authorization management helpers
- `storage`: Storage operation builders
- `client`: High-level client API

## Integration with subxt

The SDK provides data preparation and CID calculation, but actual blockchain submission requires `subxt`. Here's a complete example:

```rust
use bulletin_sdk_rust::prelude::*;
use subxt::{OnlineClient, PolkadotConfig};

#[subxt::subxt(runtime_metadata_path = "metadata.scale")]
pub mod bulletin {}

async fn store_data(data: Vec<u8>) -> Result<CidData> {
    // Initialize SDK client
    let sdk_client = BulletinClient::new();

    // Prepare operation
    let operation = sdk_client.prepare_store(data, StoreOptions::default())?;
    let cid_data = operation.calculate_cid()?;

    // Connect to chain
    let api = OnlineClient::<PolkadotConfig>::new().await?;

    // Build transaction (placeholder - adjust based on your runtime)
    // let tx = bulletin::tx()
    //     .transaction_storage()
    //     .store(operation.data);

    // Submit and wait
    // let result = api.tx()
    //     .sign_and_submit_then_watch_default(&tx, &signer)
    //     .await?
    //     .wait_for_finalized_success()
    //     .await?;

    Ok(cid_data)
}
```

## Feature Flags

- `std` (default): Enable standard library support
- `ink`: Enable ink! smart contract compatibility
- `serde-support`: Enable serialization support for DAG structures

## no_std Support

The SDK core is no_std compatible for use in constrained environments like ink! smart contracts:

```rust
#![no_std]

use bulletin_sdk_rust::prelude::*;

// All core functionality available
let chunker = FixedSizeChunker::default_config();
let chunks = chunker.chunk(data)?;
```

## Examples

See the `examples/` directory for complete working examples:

- `simple_store.rs`: Basic data storage
- `chunked_store.rs`: Large file storage with chunking

Run examples:

```bash
cargo run --example simple_store
cargo run --example chunked_store -- large-file.bin
```

## Testing

Run all tests:

```bash
cargo test
```

Run tests in no_std mode:

```bash
cargo test --no-default-features
```

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
