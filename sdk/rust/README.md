# Bulletin SDK for Rust

> [!WARNING]
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../../LICENSE-APACHE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain).

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
// let tx = your_runtime::tx().transaction_storage().store(operation.data);
// api.tx().sign_and_submit_then_watch_default(&tx, &signer).await?;
```

### Submit via the Transaction Client (std only)

```rust
use bulletin_sdk_rust::prelude::*;

// Prepare locally, then submit over the network
let client = BulletinClient::new();
let operation = client.prepare_store(b"Hello!".to_vec(), StoreOptions::default())?;

let tx_client = TransactionClient::new("wss://paseo-bulletin-rpc.polkadot.io").await?;
let receipt = tx_client.store(operation.data, &signer, WaitFor::InBlock).await?;

println!("Stored in block: {}", receipt.block_hash);
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

- **`BulletinClient`** - Prepares operations (chunking, CID calculation, manifests); `no_std` compatible
- **`TransactionClient`** - Submits store/renew/authorize extrinsics over subxt (`std` only)

Prepare operations with `BulletinClient`, then submit them with `TransactionClient` or via subxt directly.

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
- ✅ Transaction submission via `TransactionClient` (`std` only)
- ✅ no_std compatible core

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

## Security

See the [root README](../../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
