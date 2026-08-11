# Bulletin SDK for TypeScript

> [!WARNING]
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain).

Off-chain client SDK for Polkadot Bulletin Chain with PAPI integration.

## Quick Start

```typescript
import { BulletinClient, blobFromBytes } from '@parity/bulletin-sdk';
import { getWsProvider } from 'polkadot-api/ws';

// The SDK owns its PAPI connection.
const client = new BulletinClient({
  providers: () => [getWsProvider('ws://localhost:9944')],
  uploadSigner: signer,
  // descriptor: bulletinDescriptor, // optional; omit to use getUnsafeApi()
});

// Estimate (preview cost / size authorization), then submit.
const src = blobFromBytes(data);
const { cids } = await client.submit(await client.estimateUpload(src), src).send();
// Last CID is the retrieval id: the manifest root, or the lone chunk's CID.
console.log('Stored with CID:', cids[cids.length - 1].toString());
```

> See the [examples](../../examples/) directory for more usage patterns.

## Installation

```bash
npm install @parity/bulletin-sdk
# or
yarn add @parity/bulletin-sdk
```

## Build & Test

```bash
# Install dependencies
npm install

# Build
npm run build

# Unit tests
npm run test:unit

# Integration tests (requires running node)
npm run test:integration
```

## Documentation

Complete documentation: [`docs/book`](../../docs/book/)

The SDK book contains:
- Detailed API reference
- Concepts (authorization, chunking, manifests)
- Usage examples and best practices
- PAPI integration guide
- Browser & Node.js usage

## Features

- CID calculation (Raw, DAG-PB, DAG-CBOR codecs)
- Automatic chunking (default 1 MiB, configurable)
- DAG-PB manifest generation (IPFS-compatible)
- Authorization management (`authorizeAccount`, `authorizePreimage`)
- Data renewal (`renew`)
- Progress tracking callbacks
- Builder pattern API
- Mock client for testing
- TypeScript types throughout
- Browser & Node.js compatible

## Security

See the [root README](../../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
