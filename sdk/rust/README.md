# Bulletin SDK for Rust

Off-chain client SDK for Polkadot Bulletin Chain with **complete transaction submission support**.

## Quick Start

```rust
use bulletin_sdk_rust::prelude::*;

// Create client with your transaction submitter
let submitter = MySubmitter::new("ws://localhost:9944").await?;
let client = AsyncBulletinClient::new(submitter);

// Store data - complete workflow in one call
let result = client.store(b"Hello!".to_vec(), StoreOptions::default()).await?;
println!("Stored with CID: {:?}", result.cid);
```

## Installation

```toml
[dependencies]
bulletin-sdk-rust = { workspace = true }
```

For no_std environments:
```toml
[dependencies]
bulletin-sdk-rust = { workspace = true, default-features = false }
```

## Build & Test

```bash
# Build
cargo build --release --all-features

# Unit tests
cargo test --lib --all-features

# Integration tests (requires running node)
cargo test --test integration_tests --features std -- --ignored --test-threads=1
```

## Examples

See [`examples/`](examples/) for complete working examples:
- `simple_store.rs` - Basic storage with SubxtSubmitter
- `chunked_store.rs` - Large file upload with progress
- `authorization_management.rs` - All authorization operations

Run examples:
```bash
cargo run --example simple_store --features std
```

## Documentation

📚 **Complete documentation**: [`docs/sdk-book`](../../docs/sdk-book/)

The SDK book contains:
- Detailed API reference
- Concepts (authorization, chunking, manifests)
- Usage examples and best practices
- Integration guides
- no_std usage

## Features

- ✅ Complete transaction submission
- ✅ All 8 pallet operations
- ✅ Automatic chunking (default 1 MiB)
- ✅ DAG-PB manifests (IPFS-compatible)
- ✅ Authorization management
- ✅ Progress tracking
- ✅ no_std compatible core
- ✅ ink! smart contract support

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
