# Bulletin Chain SDK

Multi-language client SDKs for Polkadot Bulletin Chain.

## Available SDKs

- **[Rust](rust/)** - no_std compatible, works in native apps and ink! smart contracts
- **[TypeScript](typescript/)** - Browser and Node.js compatible

Both SDKs provide CID calculation, automatic chunking, authorization management, and DAG-PB manifest generation.

## Architecture: Bring Your Own Client

Both SDKs follow a **BYOC (Bring Your Own Client)** pattern - you provide the blockchain client and signer:

```
┌─────────────────────────────────────────┐
│           Your Application              │
├─────────────────────────────────────────┤
│  Bulletin SDK  │  Your Other Code       │
│       │        │       │                │
│       └────────┴───────┘                │
│               │                         │
│      ┌────────▼────────┐                │
│      │  Shared Client  │ ◄── You create │
│      │  (PAPI/subxt)   │                │
│      └────────┬────────┘                │
└───────────────┼─────────────────────────┘
                ▼
    ┌───────────────────────┐
    │  RPC / Light Client   │ ◄── Your choice!
    └───────────────────────┘
```

**This enables:**
- ✅ **Light client support** - Use smoldot instead of RPC endpoints
- ✅ **Connection reuse** - Share one client across your entire app
- ✅ **Browser wallets** - TypeScript SDK works with Talisman, SubWallet, etc.
- ✅ **Custom transports** - HTTP, WebSocket, or any compatible provider
- ✅ **No hidden connections** - You control all network access

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

# TypeScript integration tests:
cd sdk/typescript && npm run test:integration
```

### Build Script

Use the provided build script:

```bash
cd sdk
./build-all.sh
```

## Documentation

📚 **Complete SDK documentation**: [`docs/book`](../docs/book/)

The SDK book contains comprehensive guides including:
- **Concepts**: Authorization, chunking, DAG-PB manifests
- **Rust SDK**: Installation, API reference, no_std usage, examples
- **TypeScript SDK**: Installation, API reference, PAPI integration, examples
- **Best practices** and troubleshooting

## Features

Both SDKs provide:

- ✅ **Authorization operations** (authorizeAccount, authorizePreimage, renew)
- ⚠️ **Store transaction submission** (not yet implemented — use PAPI directly for now)
- ✅ **Automatic chunking** with configurable chunk size (default 1 MiB)
- ✅ **DAG-PB manifests** (IPFS-compatible)
- ✅ **Authorization management** (account and preimage)
- ✅ **Progress tracking** via callbacks
- ✅ **Comprehensive examples** and tests

## Examples

**Rust**: Example code is available in the [SDK book documentation](../docs/book/). Rust examples require metadata files from a running node, so they're not included in the repository. See the SDK book for complete working examples and instructions.

**TypeScript**: See [`examples/typescript/`](../examples/typescript/) for working integration examples that use the SDK's chunker, CID calculation, and DAG-PB manifest generation with PAPI for transaction submission.

## Quick Start

### Rust

```rust
use bulletin_sdk_rust::prelude::*;

let client = AsyncBulletinClient::new(api);
let result = client.store(data).send().await?;
```

See [rust/README.md](rust/README.md) for details.

### TypeScript

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';

const client = new AsyncBulletinClient(api, signer);
const result = await client.store(data).send();
```

See [typescript/README.md](typescript/README.md) for details.

## Release & Publishing

📦 **Release automation**: [RELEASE_AUTOMATION_SUMMARY.md](RELEASE_AUTOMATION_SUMMARY.md)

Complete automated release pipeline for publishing both SDKs to crates.io, npm, and GitHub Releases with version validation, testing, and automated tagging.

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
