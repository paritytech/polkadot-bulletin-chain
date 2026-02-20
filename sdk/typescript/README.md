# Bulletin SDK for TypeScript

Off-chain client SDK for Polkadot Bulletin Chain with PAPI integration.

## Quick Start

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

// Setup PAPI
const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(/* chain descriptors */);

// Create client
const client = new AsyncBulletinClient(api, signer);

// Store data using builder pattern
const result = await client.store(data).send();
console.log('Stored with CID:', result.cid.toString());
```

> **Note**: Transaction submission via `store().send()` is not yet fully implemented.
> Authorization (`authorizeAccount`, `authorizePreimage`) and `renew` operations work.
> CID calculation, chunking, and DAG-PB manifest generation are fully functional.
> See the [examples](../../examples/) directory for current working patterns using PAPI directly.

## Installation

```bash
npm install @bulletin/sdk
# or
yarn add @bulletin/sdk
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
