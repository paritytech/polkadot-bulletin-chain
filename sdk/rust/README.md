# Bulletin SDK for Rust

Off-chain client SDK for Polkadot Bulletin Chain with **complete transaction submission support**.

## Quick Start

### With Subxt

```rust
use bulletin_sdk_rust::prelude::*;

// Get WebSocket URL from environment or use default
let ws_url = std::env::var("BULLETIN_WS_URL")
    .unwrap_or_else(|_| "ws://localhost:10000".to_string());

let signer = /* your PairSigner */;

// Create submitter with URL - it connects automatically
let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
let client = AsyncBulletinClient::new(submitter);

// Store data - complete workflow in one call
let result = client.store(b"Hello!".to_vec(), StoreOptions::default()).await?;
println!("Stored with CID: {:?}", result.cid);
```

### For Testing (Mock Submitter)

```rust
use bulletin_sdk_rust::prelude::*;

// Create client with mock submitter (for testing)
let submitter = MockSubmitter::new();
let client = AsyncBulletinClient::new(submitter);

// Test without connecting to a node
let result = client.store(b"Hello!".to_vec(), StoreOptions::default()).await?;
```

## Installation

From crates.io:
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

From workspace (if in the polkadot-bulletin-chain repository):
```toml
[dependencies]
bulletin-sdk-rust = { workspace = true }
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

## Transaction Submitters

The SDK supports multiple transaction submitter implementations:

### Built-in Submitters

- **`SubxtSubmitter`** - Uses the `subxt` library for type-safe blockchain interaction
  - Status: Requires metadata generation (see docs)
  - Best for: Production applications, full type safety

- **`MockSubmitter`** - Mock implementation for testing
  - Status: Ready to use
  - Best for: Unit tests, development without a node

### Custom Submitters

You can implement your own submitter for any blockchain client library:

```rust
use bulletin_sdk_rust::submit::{TransactionSubmitter, TransactionReceipt};
use async_trait::async_trait;

pub struct MyCustomSubmitter {
    // Your client fields
}

#[async_trait]
impl TransactionSubmitter for MyCustomSubmitter {
    async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt> {
        // Your implementation
    }
    // ... implement other methods
}
```

See [`src/submitters/README.md`](src/submitters/README.md) for detailed guidance.

## Features

- ✅ Complete transaction submission
- ✅ Multiple submitter implementations (Subxt, Mock, Custom)
- ✅ All 8 pallet operations
- ✅ Automatic chunking (default 1 MiB)
- ✅ DAG-PB manifests (IPFS-compatible)
- ✅ Authorization management
- ✅ Progress tracking
- ✅ no_std compatible core
- ✅ ink! smart contract support

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
