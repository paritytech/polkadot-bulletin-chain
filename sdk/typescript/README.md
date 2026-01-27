# Bulletin SDK for TypeScript

Off-chain client SDK for Polkadot Bulletin Chain with **complete transaction submission support**.

## Quick Start

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

// Setup PAPI
const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(/* chain descriptors */);

// Create client
const submitter = new PAPITransactionSubmitter(api, signer);
const client = new AsyncBulletinClient(submitter);

// Store data - complete workflow in one call
const result = await client.store(data);
console.log('Stored with CID:', result.cid.toString());
```

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

## Examples

See [`examples/`](examples/) for complete working examples:
- `simple-store.ts` - Basic storage with PAPI
- `large-file.ts` - Chunked upload with progress
- `complete-workflow.ts` - All operations

Run examples:
```bash
npm run build
node examples/simple-store.js
```

## Documentation

📚 **Complete documentation**: [`docs/sdk-book`](../../docs/sdk-book/)

The SDK book contains:
- Detailed API reference
- Concepts (authorization, chunking, manifests)
- Usage examples and best practices
- PAPI integration guide
- Browser & Node.js usage

## Features

- ✅ Complete transaction submission
- ✅ All 8 pallet operations
- ✅ Automatic chunking (default 1 MiB)
- ✅ DAG-PB manifests (IPFS-compatible)
- ✅ Authorization management
- ✅ Progress tracking
- ✅ TypeScript types
- ✅ Browser & Node.js compatible

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
