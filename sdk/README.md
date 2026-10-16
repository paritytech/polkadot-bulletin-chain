# Bulletin Chain SDK

> [!WARNING]
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../LICENSE-APACHE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain).

Multi-language client SDKs for Polkadot Bulletin Chain.

## Available SDKs

- **[Rust](rust/)** - native apps; the core (CID, chunking, DAG-PB) is no_std compatible
- **[TypeScript](typescript/)** - Browser and Node.js compatible

Both SDKs provide CID calculation, automatic chunking, authorization management, and DAG-PB manifest generation.

## Architecture: Bring Your Own Transport

Both SDKs own their chain client. You supply the transport and the signer:

- **TypeScript**: a `providers` factory (WebSocket or smoldot providers) and an `uploadSigner`. The SDK builds the PAPI client and exposes it as `client.api`.
- **Rust**: an endpoint URL (`TransactionClient::new`) or pre-built subxt `RpcClient`s (`TransactionClient::from_rpc_clients`, WebSocket or smoldot). The SDK builds the subxt client and exposes it as `client.api()`.

**This enables:**
- ✅ **Light client support** - Use smoldot instead of RPC endpoints
- ✅ **Multi-provider broadcast** - Transactions go to every configured provider
- ✅ **Browser wallets** - TypeScript SDK works with Talisman, SubWallet, PolkaGate, etc.
- ✅ **Custom transports** - Any PAPI `JsonRpcProvider` or subxt `RpcClient`

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
cd ../typescript
npm run test:unit

# Integration tests (requires running node at ws://localhost:9944)
# Start node first (see root README quickstart):
# just binaries-polkadot && just chain-spec westend
# $(just binaries-polkadot)/polkadot-omni-node --chain ./zombienet/bulletin-westend-spec.json --dev

# TypeScript integration tests:
npm run test:integration
```

For the Rust live tests, see [rust/README.md](rust/README.md#build--test).

## Documentation

📚 **Complete SDK documentation**: [`docs/book`](../docs/book/)

The SDK book contains comprehensive guides including:
- **Concepts**: Authorization, chunking, DAG-PB manifests
- **Rust SDK**: Installation, API reference, no_std usage, examples
- **TypeScript SDK**: Installation, API reference, PAPI integration, examples
- **Best practices** and troubleshooting

## Features

Both SDKs provide:

- ✅ **Store submission** through an exactly-once upload pipeline (estimate, then submit)
- ✅ **Authorization operations** (authorize account/preimage, refresh, remove expired)
- ✅ **Renewal** (`renew`, force renew; recurring auto-renew in TypeScript)
- ✅ **Automatic chunking** with configurable chunk size (default 2 MiB)
- ✅ **DAG-PB manifests** (IPFS-compatible)
- ✅ **Progress tracking** via callbacks
- ✅ **Comprehensive examples** and tests

## Examples

**Rust**: See [`examples/rust/authorize-and-store/`](../examples/rust/authorize-and-store/) for a runnable example that authorizes an account and stores data, including a chunked upload.

**TypeScript**: See [`examples/typescript/`](../examples/typescript/) for an integration example that stores via `client.submit(await client.estimateUpload(src), src).send()`.

## Quick Start

### Rust

```rust
use bulletin_sdk_rust::prelude::*;

let client = TransactionClient::new("ws://localhost:10000").await?;
let receipt = client.store(data, &signer, WaitFor::InBlock).await?;
```

See [rust/README.md](rust/README.md) for details.

### TypeScript

```typescript
import { BulletinClient, blobFromBytes } from '@parity/bulletin-sdk';
import { getWsProvider } from 'polkadot-api/ws';

const client = new BulletinClient({
  providers: () => [getWsProvider('ws://localhost:9944')],
  uploadSigner: signer,
});
const src = blobFromBytes(data);
const { cids } = await client.submit(await client.estimateUpload(src), src).send();
```

See [typescript/README.md](typescript/README.md) for details.

## Release & Publishing

Pushing an `sdk-v*` tag runs [`.github/workflows/release-sdk.yml`](../.github/workflows/release-sdk.yml), which creates a GitHub Release and publishes `@parity/bulletin-sdk` to npm. The Rust SDK is not published (`publish = false`).

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines.

## Security

See the [root README](../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
