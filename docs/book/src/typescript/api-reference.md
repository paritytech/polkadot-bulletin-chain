# API Reference

Complete reference for the `@parity/bulletin-sdk` TypeScript package.

## Classes

### AsyncBulletinClient

The primary client for interacting with Bulletin Chain. Wraps a PAPI connection and provides high-level storage, authorization, and renewal operations.

```typescript
class AsyncBulletinClient implements BulletinClientInterface {
  api: BulletinTypedApi;
  signer: PolkadotSigner;
  submit: SubmitFn;
  config: Required<ClientConfig>;

  constructor(
    api: BulletinTypedApi,
    signer: PolkadotSigner,
    submit: SubmitFn,
    config?: Partial<ClientConfig>,
  );
}
```

**Methods:**

| Method | Returns | Description |
|--------|---------|-------------|
| `store(data)` | `StoreBuilder` | Start a store operation with builder pattern |
| `storeWithOptions(data, options?, callback?, chunkerConfig?)` | `Promise<StoreResult>` | Store with explicit options (no builder) |
| `storeWithPreimageAuth(data, options?)` | `Promise<StoreResult>` | Store using preimage-based authorization |
| `authorizeAccount(who, transactions, bytes)` | `AuthCallBuilder` | Authorize an account for storage |
| `authorizePreimage(contentHash, maxSize)` | `AuthCallBuilder` | Authorize a specific content hash |
| `renew(block, index)` | `CallBuilder` | Renew storage at a given block/index |
| `refreshAccountAuthorization(who)` | `AuthCallBuilder` | Refresh an account authorization expiry |
| `refreshPreimageAuthorization(contentHash)` | `AuthCallBuilder` | Refresh a preimage authorization expiry |
| `removeExpiredAccountAuthorization(who)` | `CallBuilder` | Remove an expired account authorization |
| `removeExpiredPreimageAuthorization(contentHash)` | `CallBuilder` | Remove an expired preimage authorization |
| `estimateAuthorization(dataSize)` | `{ transactions: number; bytes: number }` | Estimate authorization needed for a given data size |

---

### StoreBuilder

Builder pattern for configuring and executing a store operation. Obtained via `client.store(data)`.

```typescript
class StoreBuilder {
  withCodec(codec: CidCodec | number): this;
  withHashAlgorithm(algorithm: HashAlgorithm): this;
  withWaitFor(waitFor: WaitFor): this;
  withCallback(callback: ProgressCallback): this;
  withChunkSize(chunkSize: number): this;
  withManifest(enabled: boolean): this;
  send(): Promise<StoreResult>;
  sendUnsigned(): Promise<StoreResult>;
}
```

**Example:**

```typescript
const result = await client
  .store(data)
  .withCodec(CidCodec.Raw)
  .withHashAlgorithm(HashAlgorithm.Blake2b256)
  .withWaitFor(WaitFor.Finalized)
  .withCallback((event) => console.log(event))
  .send();
```

---

### CallBuilder

Builder for non-authorization transaction calls (e.g., `renew`, `removeExpired*`).

```typescript
class CallBuilder {
  withWaitFor(waitFor: WaitFor): this;
  withCallback(callback: ProgressCallback): this;
  send(): Promise<TransactionReceipt>;
}
```

---

### AuthCallBuilder

Builder for authorization-related calls. Extends `CallBuilder` with sudo support.

```typescript
class AuthCallBuilder {
  withWaitFor(waitFor: WaitFor): this;
  withCallback(callback: ProgressCallback): this;
  withSudo(): this;
  send(): Promise<TransactionReceipt>;
}
```

**Example:**

```typescript
// Authorize with sudo
await client.authorizeAccount(address, 10, BigInt(1024 * 1024))
  .withSudo()
  .send();
```

---

### BulletinPreparer

Offline data preparation without a blockchain connection. Useful for pre-calculating CIDs and preparing chunks.

```typescript
class BulletinPreparer {
  constructor(config?: ClientConfig);

  prepareStore(
    data: Uint8Array,
    options?: StoreOptions,
  ): Promise<{ data: Uint8Array; cid: CID }>;

  prepareStoreChunked(
    data: Uint8Array,
    config?: Partial<ChunkerConfig>,
    options?: StoreOptions,
  ): Promise<{
    chunks: Chunk[];
    manifest?: { data: Uint8Array; cid: CID };
  }>;

  estimateAuthorization(dataSize: number): { transactions: number; bytes: number };
}
```

---

### MockBulletinClient

A mock client for testing without a blockchain connection. Implements the same interface as `AsyncBulletinClient`.

```typescript
class MockBulletinClient implements BulletinClientInterface {
  constructor(config?: Partial<MockClientConfig>);

  // Same methods as AsyncBulletinClient, plus:
  getOperations(): MockOperation[];
  clearOperations(): void;
}
```

**Mock config options:**

```typescript
interface MockClientConfig extends ClientConfig {
  simulateAuthFailure?: boolean;
  simulateStorageFailure?: boolean;
  simulateInsufficientAuth?: boolean;
}
```

---

### FixedSizeChunker

Splits data into fixed-size chunks for large file uploads.

```typescript
class FixedSizeChunker {
  constructor(config?: Partial<ChunkerConfig>);

  chunk(data: Uint8Array): Chunk[];
  numChunks(dataSize: number): number;
  get chunkSize(): number;
}
```

---

### UnixFsDagBuilder

Builds DAG-PB (UnixFS) manifests from chunks, creating an IPFS-compatible content graph.

```typescript
class UnixFsDagBuilder {
  constructor();

  build(
    chunks: Chunk[],
    hashAlgorithm?: HashAlgorithm,
  ): Promise<DagManifest>;

  parse(dagBytes: Uint8Array): Promise<{
    chunkCids: CID[];
    totalSize: number;
  }>;
}
```

---

### BulletinError

Custom error class with structured error codes, retry guidance, and recovery hints.

```typescript
class BulletinError extends Error {
  readonly code: ErrorCode;
  readonly cause?: unknown;

  constructor(message: string, code: ErrorCode, cause?: unknown);

  get retryable(): boolean;
  get recoveryHint(): string;
}
```

---

## Enums

### CidCodec

Content identifier codec types.

| Value | Code | Description |
|-------|------|-------------|
| `CidCodec.Raw` | `0x55` | Raw binary data |
| `CidCodec.DagPb` | `0x70` | DAG-PB (UnixFS manifests) |
| `CidCodec.DagCbor` | `0x71` | DAG-CBOR |

### HashAlgorithm

Hash algorithms for CID calculation.

| Value | Code | Description |
|-------|------|-------------|
| `HashAlgorithm.Blake2b256` | `0xb220` | BLAKE2b-256 (default, Substrate-native) |
| `HashAlgorithm.Sha2_256` | `0x12` | SHA2-256 |
| `HashAlgorithm.Keccak256` | `0x1b` | Keccak-256 |

### AuthorizationScope

| Value | Description |
|-------|-------------|
| `AuthorizationScope.Account` | Account-based authorization |
| `AuthorizationScope.Preimage` | Preimage-based authorization |

### ErrorCode

| Value | Description |
|-------|-------------|
| `ErrorCode.EMPTY_DATA` | Data is empty |
| `ErrorCode.DATA_TOO_LARGE` | Data exceeds 64 MiB limit |
| `ErrorCode.CHUNK_TOO_LARGE` | Chunk exceeds 2 MiB limit |
| `ErrorCode.INVALID_CHUNK_SIZE` | Invalid chunk size configuration |
| `ErrorCode.INVALID_CONFIG` | Invalid client configuration |
| `ErrorCode.INVALID_CID` | Malformed or invalid CID |
| `ErrorCode.INVALID_HASH_ALGORITHM` | Unsupported hash algorithm |
| `ErrorCode.CID_CALCULATION_FAILED` | CID calculation error |
| `ErrorCode.DAG_ENCODING_FAILED` | DAG-PB encoding error |
| `ErrorCode.INSUFFICIENT_AUTHORIZATION` | Not enough authorization |
| `ErrorCode.AUTHORIZATION_FAILED` | Authorization call failed |
| `ErrorCode.TRANSACTION_FAILED` | Transaction submission failed |
| `ErrorCode.CHUNK_FAILED` | Chunk upload failed |
| `ErrorCode.MISSING_CHUNK` | Expected chunk not found |
| `ErrorCode.TIMEOUT` | Operation timed out |
| `ErrorCode.UNSUPPORTED_OPERATION` | Operation not supported |

### TxStatus

Transaction lifecycle states.

| Value | Description |
|-------|-------------|
| `TxStatus.Signed` | Transaction signed |
| `TxStatus.Validated` | Transaction validated by node |
| `TxStatus.Broadcasted` | Broadcasted to network |
| `TxStatus.InBlock` | Included in a block |
| `TxStatus.Finalized` | Block finalized |
| `TxStatus.NoLongerInBlock` | Block retracted |
| `TxStatus.Invalid` | Transaction invalid |
| `TxStatus.Dropped` | Transaction dropped |

### ChunkStatus

Chunk upload lifecycle states.

| Value | Description |
|-------|-------------|
| `ChunkStatus.ChunkStarted` | Chunk upload started |
| `ChunkStatus.ChunkCompleted` | Chunk upload completed |
| `ChunkStatus.ChunkFailed` | Chunk upload failed |
| `ChunkStatus.ManifestStarted` | Manifest creation started |
| `ChunkStatus.ManifestCreated` | Manifest created |
| `ChunkStatus.Completed` | All chunks and manifest uploaded |

### WaitFor

Controls when a transaction is considered complete.

| Value | Description |
|-------|-------------|
| `WaitFor.InBlock` | Wait for block inclusion (faster) |
| `WaitFor.Finalized` | Wait for block finalization (safer) |

---

## Types & Interfaces

### StoreOptions

```typescript
interface StoreOptions {
  cidCodec?: CidCodec | number;        // default: CidCodec.Raw
  hashingAlgorithm?: HashAlgorithm;    // default: HashAlgorithm.Blake2b256
  waitFor?: WaitFor;                   // default: "in_block"
}
```

### ClientConfig

```typescript
interface ClientConfig {
  defaultChunkSize?: number;           // default: 1 MiB
  createManifest?: boolean;            // default: true
  chunkingThreshold?: number;          // default: 2 MiB
  txTimeout?: number;                  // default: 420_000 (per tx, in ms)
}
```

### ChunkerConfig

```typescript
interface ChunkerConfig {
  chunkSize: number;                   // Size of each chunk in bytes
  createManifest: boolean;             // Whether to create DAG-PB manifest
}
```

### StoreResult

```typescript
interface StoreResult {
  cid?: CID;                           // Primary CID (undefined for chunked without manifest)
  size: number;                        // Total data size in bytes
  blockNumber?: number;                // Block where data was stored
  extrinsicIndex?: number;             // Extrinsic index within block
  chunks?: ChunkDetails;               // Present only for chunked uploads
}
```

### ChunkDetails

```typescript
interface ChunkDetails {
  chunkCids: CID[];
  numChunks: number;
}
```

### Chunk

```typescript
interface Chunk {
  data: Uint8Array;
  cid?: CID;
  index: number;
  totalChunks: number;
}
```

### DagManifest

```typescript
interface DagManifest {
  rootCid: CID;
  chunkCids: CID[];
  totalSize: number;
  dagBytes: Uint8Array;
}
```

### TransactionReceipt

```typescript
interface TransactionReceipt {
  blockHash: string;
  txHash: string;
  blockNumber?: number;
}
```

### CallOptions

```typescript
interface CallOptions {
  onProgress?: ProgressCallback;
  waitFor?: WaitFor;                   // default: "in_block"
}
```

### AuthCallOptions

```typescript
interface AuthCallOptions extends CallOptions {
  sudo?: boolean;
}
```

### ProgressEvent

Union of all progress events:

```typescript
type ProgressEvent = ChunkProgressEvent | TransactionStatusEvent;
type ProgressCallback = (event: ProgressEvent) => void;
```

### ChunkProgressEvent

```typescript
type ChunkProgressEvent =
  | { type: "chunk_started"; index: number; total: number }
  | { type: "chunk_completed"; index: number; total: number; cid: CID }
  | { type: "chunk_failed"; index: number; total: number; error: Error }
  | { type: "manifest_started" }
  | { type: "manifest_created"; cid: CID }
  | { type: "completed"; manifestCid?: CID };
```

### TransactionStatusEvent

```typescript
type TransactionStatusEvent =
  | { type: "signed"; txHash: string; chunkIndex?: number }
  | { type: "validated"; chunkIndex?: number }
  | { type: "broadcasted"; chunkIndex?: number }
  | { type: "in_block"; blockHash: string; blockNumber: number; txIndex?: number; chunkIndex?: number }
  | { type: "finalized"; blockHash: string; blockNumber: number; txIndex?: number; chunkIndex?: number }
  | { type: "no_longer_in_block"; chunkIndex?: number }
  | { type: "invalid"; error: string; chunkIndex?: number }
  | { type: "dropped"; error: string; chunkIndex?: number };
```

---

## Utility Functions

### CID Functions

```typescript
// Calculate a CID from data
async function calculateCid(
  data: Uint8Array,
  cidCodec?: number,           // default: 0x55 (Raw)
  hashAlgorithm?: HashAlgorithm, // default: Blake2b256
): Promise<CID>;

// Convert a CID to a different codec
function convertCid(cid: CID, newCodec: number): CID;

// Parse a CID from its string representation
function parseCid(cidString: string): CID;

// Decode a CID from raw bytes
function cidFromBytes(bytes: Uint8Array): CID;

// Encode a CID to raw bytes
function cidToBytes(cid: CID): Uint8Array;
```

### Hashing

```typescript
// Get the raw content hash for a given algorithm
async function getContentHash(
  data: Uint8Array,
  hashAlgorithm: HashAlgorithm,
): Promise<Uint8Array>;
```

### Authorization

```typescript
// Estimate authorization needed for a given data size
function estimateAuthorization(
  dataSize: number,
  chunkSize: number,
  createManifest: boolean,
): { transactions: number; bytes: number };
```

### Data Helpers

```typescript
// Convert Binary | Uint8Array to Uint8Array
function toBytes(data: Binary | Uint8Array): Uint8Array;

// Reassemble ordered chunks back into original data
function reassembleChunks(chunks: Chunk[]): Uint8Array;
```

---

## Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `MAX_CHUNK_SIZE` | `2 * 1024 * 1024` (2 MiB) | Maximum size of a single chunk |
| `MAX_FILE_SIZE` | `64 * 1024 * 1024` (64 MiB) | Maximum total file size |
| `DEFAULT_CHUNKER_CONFIG` | `{ chunkSize: 1048576, createManifest: true }` | Default chunking configuration |
| `DEFAULT_STORE_OPTIONS` | `{ cidCodec: Raw, hashingAlgorithm: Blake2b256, waitFor: "in_block" }` | Default store options |
| `VERSION` | `"0.1.0"` | SDK version string |

---

## Re-exports

The SDK re-exports the `CID` class from `multiformats`:

```typescript
export { CID } from "multiformats/cid";
```
