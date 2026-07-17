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
- ✅ **Store transaction submission**
- ✅ **Automatic chunking** with configurable chunk size (default 1 MiB)
- ✅ **DAG-PB manifests** (IPFS-compatible)
- ✅ **Authorization management** (account and preimage)
- ✅ **Progress tracking** via callbacks
- ✅ **Comprehensive examples** and tests

## Examples

**Rust**: See [`examples/rust/authorize-and-store/`](../examples/rust/authorize-and-store/) for a working SDK-based example, and the [SDK book documentation](../docs/book/) for more.

**TypeScript**: See [`examples/typescript/`](../examples/typescript/) for working integration examples that use the SDK's chunker, CID calculation, and DAG-PB manifest generation with PAPI for transaction submission.

## Quick Start

### Rust

```rust
use bulletin_sdk_rust::prelude::*;

// Prepare locally (chunking, CID, manifest) - no network calls
let client = BulletinClient::new();
let data = b"Hello, Bulletin!".to_vec();
let operation = client.prepare_store(data, StoreOptions::default())?;

// Submit via the transaction client
let tx_client = TransactionClient::new("wss://paseo-bulletin-next-rpc.polkadot.io").await?;
let receipt = tx_client.store(operation.data, &signer, WaitFor::InBlock).await?;
```

See [rust/README.md](rust/README.md) for details.

### TypeScript

```typescript
import { AsyncBulletinClient } from '@parity/bulletin-sdk';

const client = new AsyncBulletinClient(api, signer, papiClient.submit);
const data = new TextEncoder().encode("Hello, Bulletin!");
const result = await client.store(data).send();
```

See [typescript/README.md](typescript/README.md) for details.

## Release & Publishing

📦 **Release automation**: [`.github/workflows/release-sdk.yml`](../.github/workflows/release-sdk.yml)

Automated pipeline for publishing the SDKs.

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines.

## Security

See the [root README](../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
