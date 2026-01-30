# TypeScript SDK

The `@bulletin/sdk` package provides a modern, type-safe client for Node.js and Browser environments.

## Features

### Core Storage
- **Unified API**: Single `store()` method handles both small and large files
- **Automatic Chunking**: Files larger than 2 MiB are automatically chunked
- **Progress Tracking**: Real-time callbacks for upload progress
- **DAG-PB Manifests**: IPFS-compatible manifest generation
- **CID Support**: Multiple codecs (Raw, DAG-PB, DAG-CBOR) and hash algorithms

### Authorization Management
- **Pre-flight Checking**: Queries blockchain before upload to fail fast
- **Expiration Validation**: Automatically checks if authorization has expired
- **Fail Fast**: Saves transaction fees by validating before submission
- **Complete Operations**: Authorize, refresh, and manage authorizations

### Developer Experience
- **Full Type Support**: Written in TypeScript with complete definitions
- **PAPI Integration**: Designed to work seamlessly with the Polkadot API (PAPI)
- **Isomorphic**: Works in Node.js, Browsers, and other JS runtimes
- **Mock Support**: Easy testing without a blockchain node

## Quick Example

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';

// Create client
const submitter = new PAPITransactionSubmitter(api, signer);
const client = new AsyncBulletinClient(submitter)
    .withAccount(account);  // Enable authorization checking

// Store any size file (automatically chunks if > 2 MiB)
const data = new Uint8Array(50_000_000); // 50 MB
const result = await client.store(data);

console.log('Stored with CID:', result.cid.toString());
```

## Getting Started

Proceed to [Installation](./installation.md) to get started.

## Guides

- [Basic Storage](./basic-storage.md) - Store small files with a single transaction
- [Chunked Uploads](./chunked-uploads.md) - Handle large files with automatic chunking
- [PAPI Integration](./papi-integration.md) - Integrate with Polkadot API
