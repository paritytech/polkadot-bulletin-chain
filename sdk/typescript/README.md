# Bulletin SDK for TypeScript

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

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
