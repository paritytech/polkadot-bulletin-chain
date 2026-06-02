# TypeScript SDK

The `@parity/bulletin-sdk` package is a type-safe client for Node.js and the browser.

## Features

- **One submission API**: `submit(estimate, source)` covers single items, batches, and chunked files — signed or unsigned.
- **Estimate first**: `estimateUpload()` returns the CIDs, transaction count, and byte cost up front, so a UI can preview a store before paying.
- **Streamed chunking**: large files are chunked and given a DAG-PB manifest without holding the whole file in memory.
- **CID control**: per-item codec (Raw, DAG-PB, DAG-CBOR) and hash algorithm (Blake2b-256, SHA2-256, Keccak-256).
- **Authorization**: `authorizeAccount`, `authorizePreimage`, refresh, and remove-expired.
- **Progress callbacks**: per-item `ItemStarted` / `ItemInBlock` / `ItemFinalized` / `ItemFailed` events.
- **Typed errors**: `BulletinError` with an `ErrorCode` enum, `retryable`, and `recoveryHint`.
- **Mock client**: `MockBulletinClient` for tests without a node.

## Quick Example

```typescript
import { BulletinClient, blobFromBytes } from '@parity/bulletin-sdk';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

// The SDK owns its PAPI connection — give it a providers factory.
const client = new BulletinClient({
  providers: () => [getWsProvider('wss://bulletin-rpc.polkadot.io')],
  uploadSigner: signer,
});

const src = blobFromBytes(new Uint8Array(50_000_000)); // 50 MB
const { cids } = await client.submit(await client.estimateUpload(src), src).send();

// Last CID is the retrieval id: the manifest root, or the lone chunk's CID.
console.log('Stored with CID:', cids[cids.length - 1].toString());
```

## Guides

- [Installation](./installation.md) - Install the SDK
- [Authorization](./authorization.md) - Manage authorization for storage
- [Basic Storage](./basic-storage.md) - Store items with `estimateUpload` → `submit`
- [Chunked Uploads](./chunked-uploads.md) - Large files, manifests, and progress
- [Renewal](./renewal.md) - Extend data retention
- [Error Handling](./error-handling.md) - Error codes, retry logic, and recovery hints
- [PAPI Integration](./papi-integration.md) - Providers, light clients, and signers
- [API Reference](./api-reference.md) - Full type and method reference
