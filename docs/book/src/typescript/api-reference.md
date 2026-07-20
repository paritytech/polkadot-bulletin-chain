# API Reference

Reference for the `@parity/bulletin-sdk` package.

## BulletinClient

The primary client. It owns its PAPI connection (built from `providers()[0]`) and exposes storage, authorization, and renewal operations.

```typescript
class BulletinClient implements BulletinClientInterface {
  readonly api: BulletinTypedApi;       // typed PAPI API, for your own queries
  readonly signer?: PolkadotSigner;     // upload signer (undefined → unsigned-only)
  readonly config: ResolvedClientConfig;

  constructor(options: BulletinClientOptions);
}
```

### Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `estimateUpload(input, options?)` | `Promise<StreamEstimate>` | Hash offline; return CIDs + cost. `input`: `UploadItem[]` or `BlobSource` |
| `submit(estimate, source)` | `SubmitBuilder` | Store a prepared estimate, fetching bytes from `source` |
| `authorizeAccount(who, transactions, bytes)` | `AuthCallBuilder` | Authorize one account |
| `authorizeAccount(entries)` | `AuthCallBuilder` | Authorize many atomically (`Utility.batch_all`) |
| `authorizePreimage(contentHash, maxSize)` | `AuthCallBuilder` | Authorize a specific content hash |
| `renew(block, index)` | `CallBuilder` | Renew the item stored at `(block, index)` |
| `refreshAccountAuthorization(who)` | `AuthCallBuilder` | Extend an account authorization's expiry |
| `refreshPreimageAuthorization(contentHash)` | `AuthCallBuilder` | Extend a preimage authorization's expiry |
| `removeExpiredAccountAuthorization(who)` | `CallBuilder` | Remove an expired account authorization |
| `removeExpiredPreimageAuthorization(contentHash)` | `CallBuilder` | Remove an expired preimage authorization |
| `estimateAuthorization(dataSize)` | `{ transactions: number; bytes: number }` | Authorization a data size will consume |
| `destroy()` | `Promise<void>` | Tear down the PAPI connection |

`estimateUpload` + `submit` are the only storage path; see [Basic Storage](./basic-storage.md).

### BulletinClientOptions

Extends `Partial<ClientConfig>`.

```typescript
interface BulletinClientOptions extends Partial<ClientConfig> {
  providers: () => JsonRpcProvider[]; // REQUIRED for uploads; providers()[0] drives chainHead
  uploadSigner?: PolkadotSigner;      // omit for an unsigned-only client
  authorizerSigner?: PolkadotSigner;  // required to call authorize*/refresh*
  descriptor?: unknown;               // papi descriptor; omit → getUnsafeApi()
}
```

### ClientConfig

```typescript
interface ClientConfig {
  defaultChunkSize?: number;          // default 1 MiB (max 2 MiB)
  chunkingThreshold?: number;         // default 2 MiB
  createManifest?: boolean;           // default true
  txTimeout?: number;                 // default 420_000 ms (per tx)
  providers?: () => JsonRpcProvider[];
  authorizerSigner?: PolkadotSigner;
  blockLimits?: BlockLimits;          // chain block weight/length caps
}
```

## Builders

### SubmitBuilder

Returned by `submit()`.

```typescript
class SubmitBuilder {
  withWaitFor(waitFor: WaitFor): this;     // 'in_block' (default) | 'finalized'
  withCallback(cb: UploadCallback): this;  // per-item UploadEvent
  ensureAuthorized(): this;                // pre-flight authorization check
  asUnsigned(): this;                      // preimage-authorized, no signer
  send(): Promise<UploadResult>;
}
```

### CallBuilder

Returned by `renew` and the `removeExpired*` methods.

```typescript
class CallBuilder {
  withWaitFor(waitFor: WaitFor): this;
  withCallback(cb: ProgressCallback): this;
  send(): Promise<TransactionReceipt>;
}
```

### AuthCallBuilder

Returned by the authorization methods. Extends `CallBuilder` with sudo.

```typescript
class AuthCallBuilder extends CallBuilder {
  withSudo(): this; // wrap the call in Sudo.sudo
}
```

```typescript
// Atomic multi-account grant (all-or-nothing).
await client.authorizeAccount([
  { who: aliceAddr, transactions: 100, bytes: 100n * 1024n * 1024n },
  { who: bobAddr,   transactions: 50,  bytes: 50n  * 1024n * 1024n },
]).send();
```

## Byte Sources

`submit()` takes a `SeekableSource`; `estimateUpload()` takes any `BlobSource`. Construct them with:

```typescript
function blobFromBytes(data: Uint8Array): SeekableSource;          // a single blob
function blobFromItems(items: { data: Uint8Array }[]): SeekableSource; // discrete items
function blobFromFactory(open: () => AsyncIterable<Uint8Array>): BlobSource; // re-openable stream
```

`blobFromBytes` / `blobFromItems` return a `SeekableSource` (random-access `read` + known `size`) and work with both `estimateUpload` and `submit`. `blobFromFactory` returns a forward-only `BlobSource` — fine for `estimateUpload`, but `submit` needs a `SeekableSource` (implement one for file-backed streaming; see [Chunked Uploads](./chunked-uploads.md)). A `BlobSource.open()` must be callable more than once and yield the same bytes — `estimateUpload` streams it to hash, `submit` reads it again to upload.

## MockBulletinClient

Implements `BulletinClientInterface` with no chain — computes real CIDs and records operations.

```typescript
class MockBulletinClient implements BulletinClientInterface {
  constructor(config?: Partial<MockClientConfig>);
  getOperations(): MockOperation[];
  clearOperations(): void;
}

interface MockClientConfig extends ClientConfig {
  simulateAuthFailure?: boolean;
  simulateStorageFailure?: boolean;
  simulateInsufficientAuth?: boolean;
}
```

## Offline Helpers

### BulletinPreparer

```typescript
class BulletinPreparer {
  constructor(config?: ClientConfig);
  prepareStore(data, options?): Promise<{ data: Uint8Array; cid: CID }>;
  prepareStoreChunked(data, config?, options?): Promise<{ chunks: Chunk[]; manifest?: { data: Uint8Array; cid: CID } }>;
  planStream(source: BlobSource, config?, options?): Promise<ChunkPlan>;
  estimateAuthorization(dataSize: number): { transactions: number; bytes: number };
}
```

### UnixFsDagBuilder

```typescript
class UnixFsDagBuilder {
  build(chunks: Chunk[], hashAlgorithm?: HashAlgorithm): Promise<DagManifest>;
  buildFromParts(chunkCids: CID[], chunkSizes: number[], hashAlgorithm?: HashAlgorithm): Promise<DagManifest>;
  parse(dagBytes: Uint8Array): Promise<{ chunkCids: CID[]; totalSize: number }>;
}
```

### FixedSizeChunker

```typescript
class FixedSizeChunker {
  constructor(config?: Partial<ChunkerConfig>);
  chunk(data: Uint8Array): Chunk[];
  numChunks(dataSize: number): number;
  get chunkSize(): number;
}
```

## BulletinError

```typescript
class BulletinError extends Error {
  readonly code: ErrorCode;
  readonly cause?: unknown;
  get retryable(): boolean;     // true for TRANSACTION_FAILED, TIMEOUT, STORE_STALLED
  get recoveryHint(): string;
}
```

See [Error Handling](./error-handling.md) for the full `ErrorCode` reference.

## Enums

### CidCodec

| Value | Code | Description |
|-------|------|-------------|
| `Raw` | `0x55` | Raw binary (default) |
| `DagPb` | `0x70` | DAG-PB (UnixFS manifests) |
| `DagCbor` | `0x71` | DAG-CBOR |

### HashAlgorithm

| Value | Code | Description |
|-------|------|-------------|
| `Blake2b256` | `0xb220` | BLAKE2b-256 (default) |
| `Sha2_256` | `0x12` | SHA2-256 |
| `Keccak256` | `0x1b` | Keccak-256 |

### UploadStatus

| Value | String |
|-------|--------|
| `ItemStarted` | `"item_started"` |
| `ItemInBlock` | `"item_in_block"` |
| `ItemFinalized` | `"item_finalized"` |
| `ItemFailed` | `"item_failed"` |

### WaitFor

| Value | String | Meaning |
|-------|--------|---------|
| `InBlock` | `"in_block"` | Best-block inclusion (faster) |
| `Finalized` | `"finalized"` | Finalization (safer) |

`AuthorizationScope` (`Account` / `Preimage`) and `ErrorCode` are also exported.

## Types

```typescript
interface UploadItem { data: Uint8Array; codec?: CidCodec; hashAlgo?: HashAlgorithm }

interface UploadResult { cids: CID[] } // one per stored unit, in source order

interface UploadEstimateOptions {
  skipExisting?: boolean; // query the chain and drop items already stored (default false)
  dedupInput?: boolean;   // collapse duplicate content hashes in the input (default true)
}

interface UploadEstimate {
  total: number;                 // input item count
  items: UploadEstimateItem[];   // per-item disposition
  transactions: number;          // store extrinsics that will be submitted
  bytes: bigint;                 // bytes those txs consume
  duplicateIndices: number[];    // input duplicates (collapsed)
  alreadyStored: number[];       // on chain (only if skipExisting)
  toUpload: number[];            // indices that will be submitted
}

interface UploadEstimateItem { index: number; cid: CID; bytes: number; skipReason?: 'duplicate_input' | 'already_on_chain' }

// estimateUpload returns this — a UploadEstimate plus the reusable plan.
interface StreamEstimate extends UploadEstimate { plan: ChunkPlan }

interface ChunkPlan {
  chunkCids: CID[];
  chunkSizes: number[];
  offsets: number[];      // byte offset of each unit into the source
  codecs?: CidCodec[];    // per-unit; omitted → all Raw
  hashAlgos?: HashAlgorithm[]; // per-unit; omitted → all Blake2b-256
  totalSize: number;
  chunkSize: number;      // 0 for an items-as-is plan
  rootCid?: CID;          // manifest root (chunked files only)
  manifestData?: Uint8Array;
}

type UploadEvent =
  | { type: UploadStatus.ItemStarted;   index: number; total: number; cid: CID }
  | { type: UploadStatus.ItemInBlock;   index: number; total: number; cid: CID; blockHash: string; blockNumber: number; extrinsicIndex?: number }
  | { type: UploadStatus.ItemFinalized; index: number; total: number; cid: CID; blockHash: string; blockNumber: number; extrinsicIndex?: number }
  | { type: UploadStatus.ItemFailed;    index: number; total: number; cid: CID; error: Error };

type UploadCallback = (event: UploadEvent) => void;

interface TransactionReceipt { blockHash: string; txHash: string; blockNumber?: number }

interface DagManifest { rootCid: CID; chunkCids: CID[]; totalSize: number; dagBytes: Uint8Array }

interface Chunk { data: Uint8Array; cid?: CID; index: number; totalChunks: number }
```

## Utility Functions

```typescript
function calculateCid(data: Uint8Array, cidCodec?: number, hashAlgorithm?: HashAlgorithm): Promise<CID>;
function convertCid(cid: CID, newCodec: number): CID;
function parseCid(cidString: string): CID;
function cidFromBytes(bytes: Uint8Array): CID;
function cidToBytes(cid: CID): Uint8Array;
function getContentHash(data: Uint8Array, hashAlgorithm: HashAlgorithm): Promise<Uint8Array>;
function estimateAuthorization(dataSize: number, chunkSize: number, createManifest: boolean): { transactions: number; bytes: number };
function reassembleChunks(chunks: Chunk[]): Uint8Array;
function validateChunkSize(chunkSize: number): void;
```

## Constants

| Constant | Value |
|----------|-------|
| `MAX_CHUNK_SIZE` | 2 MiB |
| `MAX_FILE_SIZE` | 64 MiB |
| `DEFAULT_CHUNKER_CONFIG` | `{ chunkSize: 1 MiB, createManifest: true }` |
| `DEFAULT_CLIENT_CONFIG` | chunk 1 MiB, threshold 2 MiB, `txTimeout` 420_000 ms |
| `DEFAULT_STORE_OPTIONS` | `{ cidCodec: Raw, hashingAlgorithm: Blake2b256, waitFor: 'in_block' }` |

The SDK also re-exports `CID` from `multiformats/cid`.
