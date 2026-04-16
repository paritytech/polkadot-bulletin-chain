# TypeScript SDK

The `@parity/bulletin-sdk` package provides a modern, type-safe client for Node.js and Browser environments.

## Features

### Core Storage
- **Unified API**: Single `store()` method handles both small and large files (up to 64 MiB)
- **Automatic Chunking**: Files are automatically chunked (up to 2 MiB per chunk)
- **Progress Tracking**: Real-time callbacks for upload progress
- **DAG-PB Manifests**: Standard manifest generation for chunked data
- **CID Support**: Multiple codecs (Raw, DAG-PB, DAG-CBOR) and hash algorithms

### Authorization Management
- **Complete Operations**: Authorize, refresh, and manage authorizations

### Error Handling
- **Typed Error Codes**: `ErrorCode` enum with IDE autocomplete
- **Retryable Detection**: `error.retryable` identifies transient failures
- **Recovery Hints**: `error.recoveryHint` provides actionable suggestions
- **Transaction Events**: Full lifecycle tracking (validated, broadcasted, finalized, etc.)

### Developer Experience
- **Full Type Support**: Written in TypeScript with complete definitions
- **Direct PAPI Integration**: Tightly coupled to Polkadot API for type-safe blockchain interaction
- **Builder Pattern**: Fluent API for configuring store operations
- **Isomorphic**: Works in Node.js, Browsers, and other JS runtimes
- **Mock Support**: `MockBulletinClient` for testing without a blockchain node

## Quick Example

```typescript
import { AsyncBulletinClient } from '@parity/bulletin-sdk';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws';

// Setup PAPI client
const wsProvider = getWsProvider('wss://bulletin-rpc.polkadot.io');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(bulletinDescriptor);

// Create SDK client with PAPI client, signer, and submit function
const client = new AsyncBulletinClient(api, signer, papiClient.submit);

// Store any size file using builder pattern
const data = new Uint8Array(50_000_000); // 50 MB
const result = await client.store(data).send();

console.log('Stored with CID:', result.cid.toString());
```

## Getting Started

Proceed to [Installation](./installation.md) to get started.

## Guides

- [Installation](./installation.md) - Install the SDK
- [Authorization](./authorization.md) - Manage authorization for storage
- [Basic Storage](./basic-storage.md) - Store small files with a single transaction
- [Chunked Uploads](./chunked-uploads.md) - Handle large files with automatic chunking
- [Renewal](./renewal.md) - Extend data retention period
- [Error Handling](./error-handling.md) - Error codes, retry logic, and recovery hints
- [PAPI Integration](./papi-integration.md) - Integrate with Polkadot API
