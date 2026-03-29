# Bulletin SDK API Reference

**Version:** 0.1.0
**Date:** 12 Mar 2026

---

## TypeScript SDK (`@bulletin/sdk`)

### Classes

---

#### `AsyncBulletinClient`

Full client with PAPI-based transaction submission to Bulletin Chain.

```typescript
class AsyncBulletinClient {
  api: BulletinTypedApi
  signer: PolkadotSigner
  submit: SubmitFn
  config: Required<AsyncClientConfig>

  constructor(
    api: BulletinTypedApi,
    signer: PolkadotSigner,
    submit: SubmitFn,
    config?: Partial<AsyncClientConfig>
  )

  // Account for authorization checks
  withAccount(account: string): this
  getAccount(): string | undefined

  // Store data (returns fluent builder)
  store(data: Binary | Uint8Array): StoreBuilder

  // Store with custom options (used internally by StoreBuilder)
  storeWithOptions(
    data: Binary | Uint8Array,
    options?: StoreOptions,
    progressCallback?: ProgressCallback
  ): Promise<StoreResult>

  // Store large data with explicit chunking
  storeChunked(
    data: Binary | Uint8Array,
    config?: Partial<ChunkerConfig>,
    options?: StoreOptions,
    progressCallback?: ProgressCallback
  ): Promise<ChunkedStoreResult>

  // Store preimage-authorized content as unsigned transaction
  storeWithPreimageAuth(
    data: Binary | Uint8Array,
    options?: StoreOptions,
    progressCallback?: ProgressCallback
  ): Promise<StoreResult>

  // Authorization (requires sudo)
  authorizeAccount(
    who: string,
    transactions: number,
    bytes: bigint,
    progressCallback?: ProgressCallback
  ): Promise<TransactionReceipt>

  authorizePreimage(
    contentHash: Uint8Array,
    maxSize: bigint,
    progressCallback?: ProgressCallback
  ): Promise<TransactionReceipt>

  // Renew data retention
  renew(
    block: number,
    index: number,
    progressCallback?: ProgressCallback
  ): Promise<TransactionReceipt>

  // Estimate authorization requirements
  estimateAuthorization(dataSize: number): { transactions: number; bytes: number }
}
```

---

#### `StoreBuilder`

Fluent builder for configuring store operations.

```typescript
class StoreBuilder {
  constructor(client: AsyncBulletinClient, data: Binary | Uint8Array)

  withCodec(codec: CidCodec | number): this
  withHashAlgorithm(algorithm: HashAlgorithm): this
  withFinalization(waitFor: WaitFor): this
  withOptions(options: StoreOptions): this
  withCallback(callback: ProgressCallback): this

  send(): Promise<StoreResult>          // Signed transaction (account auth)
  sendUnsigned(): Promise<StoreResult>  // Unsigned transaction (preimage auth)
}
```

---

#### `BulletinClient`

Prepare-only client (no transaction submission). Calculates CIDs and chunks locally.

```typescript
class BulletinClient {
  constructor(config: ClientConfig)

  prepareStore(
    data: Uint8Array,
    options?: StoreOptions
  ): Promise<{ data: Uint8Array; cid: CID }>

  prepareStoreChunked(
    data: Uint8Array,
    config?: Partial<ChunkerConfig>,
    options?: StoreOptions,
    progressCallback?: ProgressCallback
  ): Promise<{ chunks: Chunk[]; manifest?: { data: Uint8Array; cid: CID } }>

  estimateAuthorization(dataSize: number): { transactions: number; bytes: number }
}
```

---

#### `MockBulletinClient`

Mock client for testing without a blockchain connection.

```typescript
class MockBulletinClient {
  constructor(config?: Partial<MockClientConfig>)

  withAccount(account: string): this
  getAccount(): string | undefined
  getOperations(): MockOperation[]
  clearOperations(): void

  store(data: Binary | Uint8Array): MockStoreBuilder

  storeWithOptions(
    data: Binary | Uint8Array,
    options?: StoreOptions,
    progressCallback?: ProgressCallback
  ): Promise<StoreResult>

  authorizeAccount(who: string, transactions: number, bytes: bigint): Promise<TransactionReceipt>
  authorizePreimage(contentHash: Uint8Array, maxSize: bigint): Promise<TransactionReceipt>
  estimateAuthorization(dataSize: number): { transactions: number; bytes: number }
}
```

---

#### `FixedSizeChunker`

Splits data into equal-sized chunks.

```typescript
class FixedSizeChunker {
  constructor(config?: Partial<ChunkerConfig>)

  chunk(data: Uint8Array): Chunk[]
  numChunks(dataSize: number): number

  get chunkSize(): number
}

// Constants
const MAX_CHUNK_SIZE: number  // 2 MiB (2,097,152 bytes)
const MAX_FILE_SIZE: number   // 64 MiB (67,108,864 bytes)
```

---

#### `UnixFsDagBuilder`

Builds IPFS-compatible DAG-PB manifests from chunks.

```typescript
class UnixFsDagBuilder {
  build(
    chunks: Chunk[],
    hashAlgorithm?: HashAlgorithm  // default: Blake2b256
  ): Promise<DagManifest>
}
```

---

### Utility Functions

```typescript
// CID calculation
function calculateCid(
  data: Uint8Array,
  cidCodec?: number,             // default: 0x55 (raw)
  hashAlgorithm?: HashAlgorithm  // default: Blake2b256
): Promise<CID>

// Content hashing
function getContentHash(data: Uint8Array, hashAlgorithm: HashAlgorithm): Promise<Uint8Array>

// CID conversion and parsing
function convertCid(cid: CID, newCodec: number): CID
function parseCid(cidString: string): CID
function cidFromBytes(bytes: Uint8Array): CID
function cidToBytes(cid: CID): Uint8Array
function validateChunkSize(size: number): void
```

---

### Enums

```typescript
enum CidCodec {
  Raw    = 0x55,   // Raw binary
  DagPb  = 0x70,   // DAG-PB
  DagCbor = 0x71,  // DAG-CBOR
}

enum HashAlgorithm {
  Blake2b256 = 0xb220,  // BLAKE2b-256
  Sha2_256   = 0x12,    // SHA2-256
  Keccak256  = 0x1b,    // Keccak-256
}

enum ErrorCode {
  EMPTY_DATA, FILE_TOO_LARGE, CHUNK_TOO_LARGE, INVALID_CHUNK_SIZE,
  INVALID_CONFIG, INVALID_CID, INVALID_HASH_ALGORITHM,
  CID_CALCULATION_FAILED, DAG_ENCODING_FAILED,
  INSUFFICIENT_AUTHORIZATION, AUTHORIZATION_FAILED,
  TRANSACTION_FAILED, CHUNK_FAILED, TIMEOUT, UNSUPPORTED_OPERATION,
}

enum AuthorizationScope {
  Account   = "Account",
  Preimage  = "Preimage",
}
```

---

### Interfaces & Types

```typescript
// Configuration
interface AsyncClientConfig {
  defaultChunkSize?: number       // default: 1 MiB
  maxParallel?: number            // default: 8
  createManifest?: boolean        // default: true
  chunkingThreshold?: number      // default: 2 MiB
  checkAuthorizationBeforeUpload?: boolean  // default: true
}

interface ClientConfig extends AsyncClientConfig {
  endpoint: string
}

interface ChunkerConfig {
  chunkSize: number       // default: 1 MiB
  maxParallel: number     // default: 8
  createManifest: boolean // default: true
}

interface StoreOptions {
  cidCodec?: CidCodec | number    // default: Raw (0x55)
  hashingAlgorithm?: HashAlgorithm // default: Blake2b256
  waitFor?: WaitFor               // default: "best_block"
}

// Results
interface StoreResult {
  cid: CID
  size: number
  blockNumber?: number
  extrinsicIndex?: number
  chunks?: ChunkDetails
}

interface ChunkedStoreResult {
  chunkCids: CID[]
  manifestCid?: CID
  totalSize: number
  numChunks: number
}

interface TransactionReceipt {
  blockHash: string
  txHash: string
  blockNumber?: number
}

interface DagManifest {
  rootCid: CID
  chunkCids: CID[]
  totalSize: number
  dagBytes: Uint8Array
}

// Data types
interface Chunk {
  data: Uint8Array
  cid?: CID
  index: number
  totalChunks: number
}

interface ChunkDetails {
  chunkCids: CID[]
  numChunks: number
}

interface Authorization {
  scope: AuthorizationScope
  transactions: number
  maxSize: bigint
  expiresAt?: number
}

// PAPI integration
interface BulletinTypedApi {
  tx: {
    TransactionStorage: {
      store(args: { data: Binary | Uint8Array }): PapiTransaction
      authorize_account(args: { who: string; transactions: number; bytes: bigint }): PapiTransaction
      authorize_preimage(args: { content_hash: Binary | Uint8Array; max_size: bigint }): PapiTransaction
      renew(args: { block: number; index: number }): PapiTransaction
    }
    Sudo: { sudo(args: { call: unknown }): PapiTransaction }
  }
}

type SubmitFn = (transaction: string, at?: string) => Promise<{
  ok: boolean
  block: { hash: string; number: number; index: number }
  txHash: string
  events: Array<{ type: string; value?: { type?: string; value?: unknown } }>
  dispatchError?: { type: string; value: unknown }
}>

type WaitFor = "best_block" | "finalized"

// Events
type ProgressEvent = ChunkProgressEvent | TransactionStatusEvent
type ProgressCallback = (event: ProgressEvent) => void

type ChunkProgressEvent =
  | { type: "chunk_started"; index: number; total: number }
  | { type: "chunk_completed"; index: number; total: number; cid: CID }
  | { type: "chunk_failed"; index: number; total: number; error: Error }
  | { type: "manifest_started" }
  | { type: "manifest_created"; cid: CID }
  | { type: "completed"; manifestCid?: CID }

type TransactionStatusEvent =
  | { type: "signed"; txHash: string }
  | { type: "validated" }
  | { type: "broadcasted"; numPeers?: number }
  | { type: "in_best_block"; blockHash: string; blockNumber: number; txIndex?: number }
  | { type: "finalized"; blockHash: string; blockNumber: number; txIndex?: number }
  | { type: "no_longer_in_best_block" }
  | { type: "invalid"; error: string }
  | { type: "dropped"; error: string }

// Error
class BulletinError extends Error {
  readonly code: ErrorCode | string
  readonly cause?: unknown
  get retryable(): boolean
  get recoveryHint(): string
}
```

---

## Rust SDK (`bulletin-sdk-rust`)

### Structs & Traits

---

#### `BulletinClient`

Prepare-only client (no_std compatible). Prepares operations for submission via subxt.

```rust
pub struct BulletinClient {
    pub config: ClientConfig,
    pub auth_manager: AuthorizationManager,
}

impl BulletinClient {
    pub fn new() -> Self
    pub fn with_config(config: ClientConfig) -> Self
    pub fn with_auth_manager(self, auth_manager: AuthorizationManager) -> Self

    pub fn prepare_store(&self, data: Vec<u8>, options: StoreOptions) -> Result<StorageOperation>
    pub fn prepare_store_chunked(
        &self,
        data: &[u8],
        config: Option<ChunkerConfig>,
        options: StoreOptions,
        progress_callback: Option<ProgressCallback>,
    ) -> Result<(BatchStorageOperation, Option<Vec<u8>>)>

    pub fn prepare_renew(&self, storage_ref: StorageRef) -> Result<RenewalOperation>
    pub fn prepare_renew_raw(&self, block: u32, index: u32) -> Result<RenewalOperation>
    pub fn estimate_authorization(&self, data_size: u64) -> (u32, u64)
}
```

---

#### `TransactionClient` (std only)

Full client with subxt-based transaction submission.

```rust
pub struct TransactionClient {
    api: OnlineClient<PolkadotConfig>,
}

impl TransactionClient {
    pub async fn new(endpoint: &str) -> Result<Self>
    pub fn from_client(api: OnlineClient<PolkadotConfig>) -> Self
    pub fn api(&self) -> &OnlineClient<PolkadotConfig>

    // Store
    pub async fn store(&self, data: Vec<u8>, signer: &Keypair) -> Result<StoreReceipt>
    pub async fn store_with_progress(
        &self,
        data: Vec<u8>,
        signer: &Keypair,
        progress_callback: Option<ProgressCallback>,
    ) -> Result<StoreReceipt>

    // Authorization
    pub async fn authorize_account(
        &self,
        who: AccountId32,
        transactions: u32,
        bytes: u64,
        signer: &Keypair,
    ) -> Result<AuthorizationReceipt>

    pub async fn authorize_preimage(
        &self,
        content_hash: ContentHash,
        max_size: u64,
        signer: &Keypair,
    ) -> Result<PreimageAuthorizationReceipt>

    // Renewal
    pub async fn renew(
        &self,
        block: u32,
        index: u32,
        signer: &Keypair,
    ) -> Result<RenewReceipt>

    // Authorization lifecycle
    pub async fn refresh_account_authorization(&self, who: AccountId32, signer: &Keypair) -> Result<()>
    pub async fn refresh_preimage_authorization(&self, content_hash: ContentHash, signer: &Keypair) -> Result<()>
    pub async fn remove_expired_account_authorization(&self, who: AccountId32, signer: &Keypair) -> Result<()>
    pub async fn remove_expired_preimage_authorization(&self, content_hash: ContentHash, signer: &Keypair) -> Result<()>
}
```

---

#### `StorageOperation`

A prepared single-item store operation.

```rust
pub struct StorageOperation {
    pub data: Vec<u8>,
    pub cid_config: CidConfig,
    pub wait_finalization: bool,
}

impl StorageOperation {
    pub fn new(data: Vec<u8>, options: StoreOptions) -> Result<Self>
    pub fn calculate_cid(&self) -> Result<CidData>
    pub fn size(&self) -> usize
    pub fn validate(&self) -> Result<()>
}
```

---

#### `BatchStorageOperation`

A prepared batch of chunk store operations.

```rust
pub struct BatchStorageOperation {
    pub operations: Vec<StorageOperation>,
    pub wait_finalization: bool,
}

impl BatchStorageOperation {
    pub fn new(chunks: &[Chunk], options: StoreOptions) -> Result<Self>
    pub fn from_chunks(chunk_data: Vec<Vec<u8>>, options: StoreOptions) -> Result<Self>
    pub fn len(&self) -> usize
    pub fn is_empty(&self) -> bool
    pub fn total_size(&self) -> usize
    pub fn calculate_cids(&self) -> Result<Vec<CidData>>
}
```

---

#### `FixedSizeChunker`

Fixed-size chunker implementing the `Chunker` trait.

```rust
pub trait Chunker {
    fn chunk(&self, data: &[u8]) -> Result<Vec<Chunk>>;
    fn validate_chunk_size(&self, size: usize) -> Result<()>;
}

pub struct FixedSizeChunker { /* ... */ }

impl FixedSizeChunker {
    pub fn new(config: ChunkerConfig) -> Result<Self>
    pub fn default_config() -> Self
    pub fn chunk_size(&self) -> usize
    pub fn num_chunks(&self, data_size: usize) -> usize
}

// Free function
pub fn reassemble_chunks(chunks: &[Chunk]) -> Result<Vec<u8>>

// Constants
pub const MAX_CHUNK_SIZE: usize   // 2 MiB
pub const MAX_FILE_SIZE: usize    // 64 MiB
pub const DEFAULT_CHUNK_SIZE: usize // 1 MiB
```

---

#### `UnixFsDagBuilder`

DAG-PB manifest builder implementing the `DagBuilder` trait.

```rust
pub trait DagBuilder {
    fn build(&self, chunks: &[Chunk], hash_algo: HashingAlgorithm) -> Result<DagManifest>;
}

pub struct UnixFsDagBuilder;

impl UnixFsDagBuilder {
    pub fn new() -> Self
}

pub struct DagManifest {
    pub root_cid: CidData,
    pub chunk_cids: Vec<CidData>,
    pub total_size: u64,
    pub dag_bytes: Vec<u8>,
}

pub const MAX_MANIFEST_CHUNKS: usize  // 1,000,000
```

---

#### `AuthorizationManager`

Client-side authorization checking and requirements calculation.

```rust
pub struct AuthorizationManager {
    pub default_scope: AuthorizationScope,
    pub auto_refresh: bool,
}

impl AuthorizationManager {
    pub fn new() -> Self
    pub fn with_account_auth() -> Self
    pub fn with_preimage_auth() -> Self
    pub fn with_auto_refresh(self, enabled: bool) -> Self

    pub fn check_authorization(
        &self,
        available: &Authorization,
        required_size: u64,
        num_transactions: u32,
    ) -> Result<()>

    pub fn calculate_requirements(
        &self,
        total_size: u64,
        num_chunks: usize,
        include_manifest: bool,
    ) -> (u32, u64)

    pub fn estimate_authorization(&self, data_size: u64, create_manifest: bool) -> (u32, u64)
}

pub struct Authorization {
    pub scope: AuthorizationScope,
    pub transactions: u32,
    pub max_size: u64,
    pub expires_at: Option<u32>,
}

// std-only helpers
pub mod helpers {
    pub fn build_account_auth_params(data_size: u64, num_chunks: usize, include_manifest: bool) -> (u32, u64)
    pub fn build_preimage_auth_params(content_hash: [u8; 32], data_size: u64) -> ([u8; 32], u64)
}
```

---

#### `RenewalTracker`

Tracks stored entries for renewal management.

```rust
pub struct RenewalOperation {
    pub storage_ref: StorageRef,
}

impl RenewalOperation {
    pub fn new(storage_ref: StorageRef) -> Self
    pub fn from_raw(block: u32, index: u32) -> Self
    pub fn validate(&self) -> Result<()>
    pub fn block(&self) -> u32
    pub fn index(&self) -> u32
}

pub struct RenewalTracker { /* ... */ }

impl RenewalTracker {
    pub fn new() -> Self
    pub fn track(&mut self, storage_ref: StorageRef, content_hash: Vec<u8>, size: u64, retention_period: u32)
    pub fn update_after_renewal(&mut self, old_ref: StorageRef, new_ref: StorageRef, retention_period: u32) -> bool
    pub fn expiring_before(&self, block: u32) -> Vec<&TrackedEntry>
    pub fn entries(&self) -> &[TrackedEntry]
    pub fn remove_by_content_hash(&mut self, content_hash: &[u8]) -> bool
    pub fn clear(&mut self)
    pub fn len(&self) -> usize
    pub fn is_empty(&self) -> bool
}

pub struct TrackedEntry {
    pub storage_ref: StorageRef,
    pub content_hash: Vec<u8>,
    pub size: u64,
    pub expires_at: u32,
}
```

---

### CID Utilities

```rust
// Re-exported from transaction-storage-primitives
pub fn calculate_cid(data: &[u8], config: CidConfig) -> core::result::Result<CidData, CidError>

// SDK convenience functions
pub fn calculate_cid_with_config(data: &[u8], codec: CidCodec, hash_algo: HashingAlgorithm) -> Result<CidData>
pub fn calculate_cid_default(data: &[u8]) -> Result<CidData>
pub fn cid_to_bytes(cid_data: &CidData) -> Result<Cid>
pub fn multihash_code_to_algorithm(code: u64) -> Option<HashingAlgorithm>

// std-only
pub fn cid_from_bytes(bytes: &[u8]) -> Result<cid::Cid>
pub fn cid_to_string(cid: &cid::Cid) -> String
pub fn cid_from_string(s: &str) -> Result<cid::Cid>
```

---

### Enums & Types

```rust
pub enum CidCodec {
    Raw,               // 0x55
    DagPb,             // 0x70
    DagCbor,           // 0x71
    Custom(u64),
}

// Re-exported from primitives
pub enum HashingAlgorithm {
    Blake2b256,   // 0xb220
    Sha2_256,     // 0x12
    Keccak256,    // 0x1b
}

pub enum AuthorizationScope { Account, Preimage }

pub enum Error {
    ChunkTooLarge(u64),
    FileTooLarge(u64),
    EmptyData,
    InvalidCid(String),
    AuthorizationNotFound(String),
    InsufficientAuthorization { need: u64, available: u64 },
    AuthorizationExpired { expired_at: u32, current_block: u32 },
    StorageFailed(String),
    DagEncodingFailed(String),
    NetworkError(String),
    InvalidConfig(String),
    ChunkingFailed(String),
    RetrievalFailed(String),
    RenewalNotFound { block: u32, index: u32 },
    RenewalFailed(String),
    CidCalculationFailed(String),
    TransactionFailed(String),
    InvalidChunkSize(String),
}

impl Error {
    pub fn code(&self) -> &'static str
    pub fn is_retryable(&self) -> bool
    pub fn recovery_hint(&self) -> &'static str
}

// Config types
pub struct ClientConfig { pub default_chunk_size: u32, pub max_parallel: u32, pub create_manifest: bool }
pub struct ChunkerConfig { pub chunk_size: u32, pub max_parallel: u32, pub create_manifest: bool }
pub struct StoreOptions { pub cid_codec: CidCodec, pub hash_algorithm: HashingAlgorithm, pub wait_for_finalization: bool }
pub struct StorageRef { pub block: u32, pub index: u32 }
pub struct CidConfig { pub codec: u64, pub hashing: HashingAlgorithm }

// Result types
pub struct StoreResult { pub cid: Vec<u8>, pub size: u64, pub block_number: Option<u32>, pub chunks: Option<ChunkDetails> }
pub struct ChunkedStoreResult { pub chunk_cids: Vec<Vec<u8>>, pub manifest_cid: Option<Vec<u8>>, pub total_size: u64, pub num_chunks: u32 }
pub struct ChunkDetails { pub chunk_cids: Vec<Vec<u8>>, pub num_chunks: u32 }
pub struct RenewalResult { pub new_ref: StorageRef, pub content_hash: Vec<u8>, pub size: u64 }
pub struct Chunk { pub data: Vec<u8>, pub index: u32, pub total_chunks: u32 }

// Receipt types (std only)
pub struct StoreReceipt { pub block_hash: String, pub extrinsic_hash: String, pub data_size: u64 }
pub struct AuthorizationReceipt { pub account: AccountId32, pub transactions: u32, pub bytes: u64, pub block_hash: String }
pub struct PreimageAuthorizationReceipt { pub content_hash: ContentHash, pub max_size: u64, pub block_hash: String }
pub struct RenewReceipt { pub original_block: u32, pub transaction_index: u32, pub block_hash: String }

// Event types
pub enum ChunkProgressEvent {
    ChunkStarted { index: u32, total: u32 },
    ChunkCompleted { index: u32, total: u32, cid: Vec<u8> },
    ChunkFailed { index: u32, total: u32, error: String },
    ManifestStarted,
    ManifestCreated { cid: Vec<u8> },
    Completed { manifest_cid: Option<Vec<u8>> },
}

pub enum TransactionStatusEvent {
    Validated,
    Broadcasted,
    InBestBlock { block_hash: String, block_number: Option<u32>, extrinsic_index: Option<u32> },
    Finalized { block_hash: String, block_number: Option<u32>, extrinsic_index: Option<u32> },
    NoLongerInBestBlock,
    Invalid { error: String },
    Dropped { error: String },
}

pub enum ProgressEvent {
    Chunk(ChunkProgressEvent),
    Transaction(TransactionStatusEvent),
}

// Convenience constructors
impl ProgressEvent {
    pub fn chunk_started(index: u32, total: u32) -> Self
    pub fn chunk_completed(index: u32, total: u32, cid: Vec<u8>) -> Self
    pub fn chunk_failed(index: u32, total: u32, error: String) -> Self
    pub fn manifest_started() -> Self
    pub fn manifest_created(cid: Vec<u8>) -> Self
    pub fn completed(manifest_cid: Option<Vec<u8>>) -> Self
    pub fn tx_validated() -> Self
    pub fn tx_broadcasted() -> Self
    pub fn tx_in_best_block(block_hash: String, block_number: Option<u32>, extrinsic_index: Option<u32>) -> Self
    pub fn tx_finalized(block_hash: String, block_number: Option<u32>, extrinsic_index: Option<u32>) -> Self
}

pub type ProgressCallback = Arc<dyn Fn(ProgressEvent) + Send + Sync>
pub type Result<T> = core::result::Result<T, Error>
```

---

## Feature Flags (Rust)

| Flag | Default | Description |
|------|---------|-------------|
| `std` | Yes | Standard library support, enables `TransactionClient` and subxt |
| `serde-support` | No | Serialization support for DAG structures |

The core SDK (`BulletinClient`, chunker, CID, DAG builder) is `no_std` compatible.
