# TypeScript SDK

The `@bulletin/sdk` package provides a modern, type-safe client for Node.js and Browser environments.

## Features

### Core Storage
- **Unified API**: Single `store()` method handles both small and large files
- **Automatic Chunking**: Files larger than 2 MiB are automatically chunked
- **Progress Tracking**: Real-time callbacks for upload progress
- **DAG-PB Manifests**: Standard manifest generation for chunked data
- **CID Support**: Multiple codecs (Raw, DAG-PB, DAG-CBOR) and hash algorithms

### Authorization Management
- **Pre-flight Checking**: Queries blockchain before upload to fail fast
- **Expiration Validation**: Automatically checks if authorization has expired
- **Fail Fast**: Saves transaction fees by validating before submission
- **Complete Operations**: Authorize, refresh, and manage authorizations

### Developer Experience
- **Full Type Support**: Written in TypeScript with complete definitions
- **Direct PAPI Integration**: Tightly coupled to Polkadot API for type-safe blockchain interaction
- **Builder Pattern**: Fluent API for configuring store operations
- **Isomorphic**: Works in Node.js, Browsers, and other JS runtimes
- **Mock Support**: `MockBulletinClient` for testing without a blockchain node

## Quick Example

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

// Setup PAPI client
const wsProvider = getWsProvider('wss://bulletin-rpc.polkadot.io');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(bulletinDescriptor);

// Create SDK client with PAPI client and signer
const client = new AsyncBulletinClient(api, signer)
    .withAccount(account);  // Enable authorization checking

// Store any size file using builder pattern
const data = new Uint8Array(50_000_000); // 50 MB
const result = await client.store(data).send();

console.log('Stored with CID:', result.cid.toString());
```

## Getting Started

Proceed to [Installation](./installation.md) to get started.

## Guides

- [Basic Storage](./basic-storage.md) - Store small files with a single transaction
- [Chunked Uploads](./chunked-uploads.md) - Handle large files with automatic chunking
- [PAPI Integration](./papi-integration.md) - Integrate with Polkadot API
