# Rust SDK

The `bulletin-sdk-rust` crate provides a robust client for interacting with the Bulletin Chain. It is designed to be:

- **Type-Safe**: Leverages Rust's type system to prevent common errors.
- **Flexible**: Works with `std` (standard library) and `no_std` (WASM/embedded) environments.
- **Modular**: Use only what you need (chunking, CID calculation, or full client).

## Key Features

- **Complete Transaction Support**: Built-in submitters for `subxt` and mock testing
- **Flexible Architecture**: Use `AsyncBulletinClient` for full automation or prepare operations manually
- **Multiple Submitter Options**: SubxtSubmitter, MockSubmitter, or create your own
- **Connection Management**: Simple `from_url()` constructor handles WebSocket connections
- **Testing Support**: MockSubmitter allows testing without a blockchain node

## Modules

- `async_client`: High-level async client with transaction submission (`AsyncBulletinClient`)
- `client`: Core client for operation preparation (`BulletinClient`)
- `submitters`: Transaction submitter implementations (SubxtSubmitter, MockSubmitter)
- `chunker`: Splits data into chunks (`FixedSizeChunker`)
- `cid`: CID calculation utilities
- `storage`: Transaction preparation helpers
- `authorization`: Authorization management
- `submit`: TransactionSubmitter trait definition

## Quick Start

```rust
use bulletin_sdk_rust::prelude::*;

let ws_url = std::env::var("BULLETIN_WS_URL")
    .unwrap_or_else(|_| "ws://localhost:10000".to_string());
let signer = /* your PairSigner */;

// Connect and create client in one step
let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
let client = AsyncBulletinClient::new(submitter);

// Store data - complete workflow
let result = client.store(data, StoreOptions::default()).await?;
```

Proceed to [Installation](./installation.md) to get started.
