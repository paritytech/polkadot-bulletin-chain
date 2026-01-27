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
```

## Examples

Example code is available in the [SDK book documentation](../../docs/sdk-book/).

Rust examples require metadata files from a running Bulletin Chain node and external dependencies (subxt), so they're not included in the repository to avoid CI issues. The SDK book contains complete working examples with detailed explanations.

**Key examples covered in the SDK book:**
- Basic storage with SubxtSubmitter implementation
- Large file upload with progress tracking
- Authorization management (all 8 pallet operations)
- Custom TransactionSubmitter implementations

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
