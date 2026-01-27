# Bulletin SDK for TypeScript

Off-chain client SDK for Polkadot Bulletin Chain with **complete transaction submission support**. Store data on Bulletin Chain with one SDK call - from data preparation to finalized transactions.

## Features

- **Complete Transaction Submission**: Handles everything from chunking to blockchain finalization
- **All 8 Pallet Operations**: Full support for store, authorize, renew, refresh, and cleanup operations
- **Automatic Chunking**: Split large files into optimal chunks (default 1 MiB)
- **DAG-PB Manifests**: IPFS-compatible manifest generation for chunked data
- **Authorization Management**: Built-in helpers for account and preimage authorization
- **Progress Tracking**: Callback-based progress events for uploads
- **TypeScript**: Full TypeScript support with type definitions
- **Browser & Node.js**: Compatible with both environments
- **PAPI Integration**: Seamless integration with Polkadot API

## Installation

```bash
npm install @bulletin/sdk
# or
yarn add @bulletin/sdk
# or
pnpm add @bulletin/sdk
```

## Quick Start

### Complete Store Workflow

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import { getPolkadotSigner } from 'polkadot-api/signer';
import { DEV_PHRASE } from '@polkadot-labs/hdkd-helpers';

// 1. Setup connection and signer
const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(/* your chain descriptors */);

const keyring = sr25519CreateDerive(DEV_PHRASE);
const signer = getPolkadotSigner(keyring.derive("//Alice"), "Alice", 42);

// 2. Create transaction submitter
const submitter = new PAPITransactionSubmitter(api, signer);

// 3. Create client
const client = new AsyncBulletinClient(submitter);

// 4. Store data - complete workflow in one call!
const data = new TextEncoder().encode('Hello, Bulletin!');
const result = await client.store(data);

console.log('Stored with CID:', result.cid.toString());
console.log('Block number:', result.blockNumber);
```

That's it! The SDK handles:
- ✅ CID calculation
- ✅ Transaction building
- ✅ Signing and submission
- ✅ Waiting for finalization
- ✅ Returning receipt with block info

## Complete API Reference

### All Supported Operations

```typescript
// Store operations
await client.store(data, options);
await client.storeChunked(data, config, options, progressCallback);

// Authorization operations (requires sudo)
await client.authorizeAccount(who, transactions, bytes);
await client.authorizePreimage(hash, maxSize);
await client.refreshAccountAuthorization(who);
await client.refreshPreimageAuthorization(hash);

// Maintenance operations
await client.renew(block, index);
await client.removeExpiredAccountAuthorization(who);
await client.removeExpiredPreimageAuthorization(hash);

// Utilities
const estimate = client.estimateAuthorization(dataSize);
```

## Usage Examples

### 1. Simple Store (< 8 MiB)

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';

const submitter = new PAPITransactionSubmitter(api, signer);
const client = new AsyncBulletinClient(submitter);

const data = new TextEncoder().encode('Hello, Bulletin!');

// Store and wait for finalization
const result = await client.store(data);

console.log('✅ Stored!');
console.log('   CID:', result.cid.toString());
console.log('   Size:', result.size, 'bytes');
console.log('   Block:', result.blockNumber);
```

### 2. Chunked Store (Large Files)

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';
import { readFile } from 'fs/promises';

// Create client with custom config
const client = new AsyncBulletinClient(submitter, {
  defaultChunkSize: 1024 * 1024, // 1 MiB
  maxParallel: 8,
  createManifest: true,
});

// Read large file
const data = await readFile('large_video.mp4');

// Store with progress tracking
const result = await client.storeChunked(
  data,
  undefined, // use default config
  undefined, // use default options
  (event) => {
    switch (event.type) {
      case 'chunk_completed':
        console.log(`Chunk ${event.index + 1}/${event.total} uploaded`);
        break;
      case 'manifest_created':
        console.log('Manifest CID:', event.cid.toString());
        break;
    }
  }
);

console.log('✅ Uploaded', result.numChunks, 'chunks');
console.log('   Manifest CID:', result.manifestCid?.toString());
```

### 3. Authorization Management

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';

const client = new AsyncBulletinClient(submitter);

// Estimate authorization needed
const estimate = client.estimateAuthorization(10_000_000); // 10 MB

// Authorize account (requires sudo)
await client.authorizeAccount(
  accountAddress,
  estimate.transactions,
  BigInt(estimate.bytes)
);

console.log('✅ Account authorized for', estimate.transactions, 'transactions');

// Refresh before expiry
await client.refreshAccountAuthorization(accountAddress);
```

### 4. Content-Addressed Authorization

```typescript
import { blake2b256 } from '@noble/hashes/blake2b';

// Authorize specific content by hash
const data = new TextEncoder().encode('Specific content to authorize');
const contentHash = blake2b256(data);

// Authorize preimage (requires sudo)
await client.authorizePreimage(
  contentHash,
  BigInt(data.length)
);

// Now anyone can store this specific content
const result = await client.store(data);
```

### 5. Renew Stored Data

```typescript
// Extend retention period for stored data
await client.renew(blockNumber, transactionIndex);

console.log('✅ Data retention period extended');
```

### 6. Custom CID Configuration

```typescript
import { CidCodec, HashAlgorithm } from '@bulletin/sdk';

const options = {
  cidCodec: CidCodec.DagPb,
  hashingAlgorithm: HashAlgorithm.Sha2_256,
  waitForFinalization: true,
};

const result = await client.store(data, options);
```

## Integration with Polkadot API (PAPI)

The SDK uses a flexible transaction submission interface. Here's how to integrate with PAPI:

### PAPITransactionSubmitter Implementation

The SDK provides a complete `PAPITransactionSubmitter` implementation:

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';
import { getPolkadotSigner } from 'polkadot-api/signer';

// 1. Setup PAPI connection
const wsProvider = getWsProvider('ws://localhost:9944');
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(/* your chain descriptors */);

// 2. Create signer
const signer = getPolkadotSigner(/* your key */, "YourName", 42);

// 3. Create transaction submitter
const submitter = new PAPITransactionSubmitter(api, signer);

// 4. Create Bulletin client
const client = new AsyncBulletinClient(submitter);

// 5. Use any operation - it's all handled!
const result = await client.store(data);
```

### Custom Submitter Implementation

You can implement your own transaction submitter for custom signing logic:

```typescript
import { TransactionSubmitter, TransactionReceipt } from '@bulletin/sdk';

class CustomSubmitter implements TransactionSubmitter {
  async submitStore(data: Uint8Array): Promise<TransactionReceipt> {
    const tx = this.api.tx.TransactionStorage.store({ data });

    const result = await tx.signAndSubmit(this.signer);
    const finalized = await result.waitFor('finalized');

    return {
      blockHash: finalized.blockHash,
      txHash: finalized.txHash,
      blockNumber: finalized.blockNumber,
    };
  }

  async submitAuthorizeAccount(who: string, transactions: number, bytes: bigint): Promise<TransactionReceipt> {
    const tx = this.api.tx.TransactionStorage.authorize_account({
      who,
      transactions,
      bytes,
    });

    const result = await tx.signAndSubmit(this.signer);
    const finalized = await result.waitFor('finalized');

    return {
      blockHash: finalized.blockHash,
      txHash: finalized.txHash,
      blockNumber: finalized.blockNumber,
    };
  }

  // Implement other methods similarly...
}
```

## Architecture

The SDK is organized into modular components:

- **types**: Core TypeScript types and interfaces
- **chunker**: Data chunking utilities
- **cid**: CID calculation and utilities
- **dag**: DAG-PB manifest building
- **authorization**: Authorization management helpers
- **storage**: Storage operation builders
- **transaction**: Transaction submission interfaces
- **async-client**: High-level async client with full transaction support
- **client**: Preparation-only client (for advanced use cases)

## Examples

See the `examples/` directory for complete working examples:

- **`simple-store.ts`**: Basic data storage with transaction submission
- **`large-file.ts`**: Large file upload with progress tracking
- **`complete-workflow.ts`**: All authorization and storage operations

Run examples:

```bash
# Install dependencies
npm install

# Build
npm run build

# Simple store
node examples/simple-store.js

# Large file upload
node examples/large-file.js large_file.bin

# Complete workflow
node examples/complete-workflow.js
```

## Before vs After

**Before (manual integration):**
```typescript
// 1. Calculate CID manually
const cid = await calculateCid(data);

// 2. Build transaction manually
const tx = api.tx.TransactionStorage.store({ data });

// 3. Sign and submit manually
const result = await tx.signAndSubmit(signer);

// 4. Wait for finalization manually
const finalized = await result.waitFor('finalized');

// 5. Process result manually
const blockHash = finalized.blockHash;
```

**After (SDK handles everything):**
```typescript
const result = await client.store(data);
// Done! CID calculated, transaction submitted, finalized, receipt returned
```

## Best Practices

1. **Authorization First**: Authorize accounts before storing to ensure capacity
2. **Account vs Preimage**: Use account authorization for dynamic content, preimage for known content
3. **Refresh Early**: Refresh authorizations before they expire to maintain access
4. **Renew Important Data**: Renew stored data before retention period ends
5. **Clean Up**: Remove expired authorizations to free storage
6. **Progress Tracking**: Use callbacks for long uploads to provide user feedback
7. **Error Handling**: Always handle errors appropriately

## Performance Tips

- **Chunk Size**: Default 1 MiB works well; adjust for your network conditions
- **Parallel Uploads**: Increase `maxParallel` for faster uploads (default: 8)
- **Manifests**: Enable for files > 8 MiB to allow IPFS gateway retrieval
- **Batch Operations**: Use `storeChunked` for multiple chunks in one call

## Troubleshooting

**Q: Transaction fails with "InsufficientAuthorization"**
A: Ensure the account is authorized first using `authorizeAccount()` or `authorizePreimage()`

**Q: Chunk upload fails with "ChunkTooLarge"**
A: Each chunk must be ≤ 8 MiB. Use SDK's automatic chunking with `storeChunked()`

**Q: Can't retrieve via IPFS**
A: Ensure `createManifest: true` was used for chunked uploads

**Q: Authorization expired**
A: Refresh authorizations using `refreshAccountAuthorization()` before they expire

## API Documentation

### AsyncBulletinClient

Main client class for complete Bulletin Chain operations.

#### Constructor

```typescript
new AsyncBulletinClient(
  submitter: TransactionSubmitter,
  config?: Partial<AsyncClientConfig>
)
```

#### Store Operations

##### `store(data, options?)`

Store data with complete transaction submission.

**Parameters:**
- `data: Uint8Array` - Data to store
- `options?: StoreOptions` - Storage options

**Returns:** `Promise<StoreResult>`
- `cid: CID` - Content identifier
- `size: number` - Data size in bytes
- `blockNumber?: number` - Block number where stored

##### `storeChunked(data, config?, options?, progressCallback?)`

Store large data with automatic chunking and manifest creation.

**Parameters:**
- `data: Uint8Array` - Data to store
- `config?: Partial<ChunkerConfig>` - Chunking configuration
- `options?: StoreOptions` - Storage options
- `progressCallback?: ProgressCallback` - Progress tracking callback

**Returns:** `Promise<ChunkedStoreResult>`
- `chunkCids: CID[]` - CIDs of all chunks
- `manifestCid?: CID` - CID of DAG-PB manifest
- `totalSize: number` - Total data size
- `numChunks: number` - Number of chunks

#### Authorization Operations

##### `authorizeAccount(who, transactions, bytes)`

Authorize an account to store data (requires sudo).

**Parameters:**
- `who: string` - Account address (SS58 format)
- `transactions: number` - Number of transactions to authorize
- `bytes: bigint` - Total bytes to authorize

**Returns:** `Promise<TransactionReceipt>`

##### `authorizePreimage(hash, maxSize)`

Authorize specific content by hash (requires sudo).

**Parameters:**
- `hash: Uint8Array` - Content hash (32 bytes)
- `maxSize: bigint` - Maximum size in bytes

**Returns:** `Promise<TransactionReceipt>`

##### `refreshAccountAuthorization(who)`

Refresh account authorization to extend expiry.

**Parameters:**
- `who: string` - Account address

**Returns:** `Promise<TransactionReceipt>`

##### `refreshPreimageAuthorization(hash)`

Refresh preimage authorization to extend expiry.

**Parameters:**
- `hash: Uint8Array` - Content hash (32 bytes)

**Returns:** `Promise<TransactionReceipt>`

#### Maintenance Operations

##### `renew(block, index)`

Renew stored data to extend retention period.

**Parameters:**
- `block: number` - Block number where data was stored
- `index: number` - Transaction index in block

**Returns:** `Promise<TransactionReceipt>`

##### `removeExpiredAccountAuthorization(who)`

Remove expired account authorization (cleanup).

**Parameters:**
- `who: string` - Account address

**Returns:** `Promise<TransactionReceipt>`

##### `removeExpiredPreimageAuthorization(hash)`

Remove expired preimage authorization (cleanup).

**Parameters:**
- `hash: Uint8Array` - Content hash (32 bytes)

**Returns:** `Promise<TransactionReceipt>`

#### Utility Methods

##### `estimateAuthorization(dataSize)`

Calculate required authorization for given data size.

**Parameters:**
- `dataSize: number` - Data size in bytes

**Returns:** `{ transactions: number, bytes: number }`

### Types

#### `StoreOptions`

```typescript
interface StoreOptions {
  cidCodec?: CidCodec;
  hashingAlgorithm?: HashAlgorithm;
  waitForFinalization?: boolean;
}
```

#### `ChunkerConfig`

```typescript
interface ChunkerConfig {
  chunkSize: number;
  maxParallel: number;
  createManifest: boolean;
}
```

#### `AsyncClientConfig`

```typescript
interface AsyncClientConfig {
  defaultChunkSize: number;
  maxParallel: number;
  createManifest: boolean;
}
```

#### `TransactionReceipt`

```typescript
interface TransactionReceipt {
  blockHash: string;
  txHash: string;
  blockNumber?: number;
}
```

#### `ProgressEvent`

```typescript
type ProgressEvent =
  | { type: 'chunk_started'; index: number; total: number }
  | { type: 'chunk_completed'; index: number; total: number; cid: CID }
  | { type: 'chunk_failed'; index: number; total: number; error: Error }
  | { type: 'manifest_started' }
  | { type: 'manifest_created'; cid: CID }
  | { type: 'completed'; manifestCid?: CID };
```

### Enums

#### `CidCodec`

```typescript
enum CidCodec {
  Raw = 0x55,
  DagPb = 0x70,
  DagCbor = 0x71,
}
```

#### `HashAlgorithm`

```typescript
enum HashAlgorithm {
  Blake2b256 = 0xb220,
  Sha2_256 = 0x12,
  Keccak256 = 0x1b,
}
```

## Development

```bash
# Install dependencies
npm install

# Build
npm run build

# Run tests
npm test

# Run type checking
npm run typecheck

# Lint
npm run lint
```

## Testing

```bash
# Run all tests
npm test

# Run unit tests
npm run test:unit

# Run integration tests (requires local node)
npm run test:integration
```

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
