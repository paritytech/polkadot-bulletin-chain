# Bulletin SDK for Rust

Off-chain client SDK for Polkadot Bulletin Chain with automatic chunking, DAG-PB manifest generation, and authorization management.

## Quick Start

### Prepare and Submit via Subxt (Recommended)

The SDK prepares storage operations; you submit them via subxt with your runtime metadata:

```rust
use bulletin_sdk_rust::prelude::*;

// Prepare the operation (no network calls)
let client = BulletinClient::new();
let data = b"Hello, Bulletin!".to_vec();
let operation = client.prepare_store(data, StoreOptions::default())?;

// Submit via your subxt setup
// let tx = your_runtime::tx().transaction_storage().store(operation.data, None);
// api.tx().sign_and_submit_then_watch_default(&tx, &signer).await?;
```

### For Testing (Mock Client)

```rust
use bulletin_sdk_rust::prelude::*;

// Create mock client for testing without a node
let client = MockBulletinClient::new();

let result = client
    .store(b"Hello!".to_vec())
    .send()
    .await?;

println!("Mock CID: {:?}", result.cid);
```

## Installation

```toml
[dependencies]
bulletin-sdk-rust = { path = "sdk/rust" }
```

For no_std environments:
```toml
[dependencies]
bulletin-sdk-rust = { path = "sdk/rust", default-features = false }
```

## Architecture

The SDK is split into layers:

- **`BulletinClient`** - Prepares operations (chunking, CID calculation, manifests)
- **`AsyncBulletinClient`** - ⚠️ Experimental, placeholder implementation
- **`MockBulletinClient`** - Mock client for testing

For production use, prepare operations with `BulletinClient` and submit via subxt directly.

## Build & Test

```bash
# Build
cargo build --release --all-features

# Run tests
cargo test --all-features
```

## Features

- ✅ Automatic chunking (default 1 MiB, max 2 MiB)
- ✅ DAG-PB manifests (IPFS-compatible)
- ✅ CID calculation (Blake2b-256, SHA2-256, Keccak-256)
- ✅ Authorization estimation
- ✅ Progress callbacks with closure support
- ✅ Mock client for testing
- ✅ no_std compatible core
- ⚠️ Direct transaction submission (experimental)

## API Overview

### BulletinClient (Core)

```rust
let client = BulletinClient::new();

// Simple store (< 2 MiB)
let op = client.prepare_store(data, StoreOptions::default())?;

// Chunked store (large files)
let (batch, manifest) = client.prepare_store_chunked(
    &large_data,
    Some(ChunkerConfig::default()),
    StoreOptions::default(),
    Some(Arc::new(|event| println!("{:?}", event))),
)?;

// Estimate authorization needed
let (txs, bytes) = client.estimate_authorization(data_size);
```

### Progress Callbacks

Callbacks support closures with captured state:

```rust
use std::sync::{Arc, atomic::{AtomicU32, Ordering}};

let counter = Arc::new(AtomicU32::new(0));
let counter_clone = counter.clone();

let callback: ProgressCallback = Arc::new(move |event| {
    counter_clone.fetch_add(1, Ordering::SeqCst);
    println!("Event: {:?}", event);
});
```

## Examples

See the [`examples/`](../../examples/) directory for integration examples.

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
