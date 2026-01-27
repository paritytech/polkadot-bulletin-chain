# Bulletin Chain SDK

Multi-language client SDKs for Polkadot Bulletin Chain with complete transaction submission support.

## Available SDKs

- **[Rust](rust/)** - no_std compatible, works in native apps and ink! smart contracts
- **[TypeScript](typescript/)** - Browser and Node.js compatible

Both SDKs provide complete transaction submission with automatic chunking, authorization management, and DAG-PB manifest generation.

## Quick Build & Test

### Build All SDKs

```bash
# From repository root

# Build Rust SDK
cd sdk/rust
cargo build --release --all-features

# Build TypeScript SDK
cd ../typescript
npm install
npm run build
```

### Test All SDKs

```bash
# Rust unit tests
cd sdk/rust
cargo test --lib --all-features

# TypeScript unit tests
cd sdk/typescript
npm run test:unit

# Integration tests (requires running node at ws://localhost:9944)
# Start node first:
./target/release/polkadot-bulletin-chain --dev --tmp

# Then run integration tests:
cd sdk/rust && cargo test --test integration_tests --features std -- --ignored --test-threads=1
cd sdk/typescript && npm run test:integration
```

### Build Script

Use the provided build script:

```bash
cd sdk
./build-all.sh
```

## Documentation

📚 **Complete SDK documentation**: [`docs/sdk-book`](../docs/sdk-book/)

The SDK book contains comprehensive guides including:
- **Concepts**: Authorization, chunking, DAG-PB manifests
- **Rust SDK**: Installation, API reference, no_std usage, examples
- **TypeScript SDK**: Installation, API reference, PAPI integration, examples
- **Best practices** and troubleshooting

## Features

Both SDKs provide:

- ✅ **Complete transaction submission** (not just data preparation)
- ✅ **All 8 pallet operations** (store, renew, authorize, refresh, cleanup)
- ✅ **Automatic chunking** with configurable chunk size (default 1 MiB)
- ✅ **DAG-PB manifests** (IPFS-compatible)
- ✅ **Authorization management** (account and preimage)
- ✅ **Progress tracking** via callbacks
- ✅ **Comprehensive examples** and tests

## Examples

Each SDK includes working examples:

**Rust** (`sdk/rust/examples/`):
- `simple_store.rs` - Basic storage with SubxtSubmitter
- `chunked_store.rs` - Large file upload with progress
- `authorization_management.rs` - All authorization operations

**TypeScript** (`sdk/typescript/examples/`):
- `simple-store.ts` - Basic storage with PAPI
- `large-file.ts` - Chunked upload with progress
- `complete-workflow.ts` - All operations demonstration

## Quick Start

### Rust

```rust
use bulletin_sdk_rust::prelude::*;

let client = AsyncBulletinClient::new(submitter);
let result = client.store(data, StoreOptions::default()).await?;
```

See [rust/README.md](rust/README.md) for details.

### TypeScript

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';

const client = new AsyncBulletinClient(submitter);
const result = await client.store(data);
```

See [typescript/README.md](typescript/README.md) for details.

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
