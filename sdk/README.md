# Bulletin Chain SDK

> [!WARNING]
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../LICENSE-APACHE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain).

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
# Start node first (see root README quickstart):
# just binaries-polkadot && just chain-spec westend
# $(just binaries-polkadot)/polkadot-omni-node --chain ./zombienet/bulletin-westend-spec.json --dev

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

let client = BulletinClient::new(api);
let result = client.store(data).send().await?;
```

See [rust/README.md](rust/README.md) for details.

### TypeScript

```typescript
import { BulletinClient } from '@parity/bulletin-sdk';

const client = new BulletinClient(api, signer);
const result = await client.store(data).send();
```

See [typescript/README.md](typescript/README.md) for details.

## Release & Publishing

📦 **Release automation**: [RELEASE_AUTOMATION_SUMMARY.md](RELEASE_AUTOMATION_SUMMARY.md)

Complete automated release pipeline for publishing both SDKs to crates.io, npm, and GitHub Releases with version validation, testing, and automated tagging.

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines.

## Security

See the [root README](../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
