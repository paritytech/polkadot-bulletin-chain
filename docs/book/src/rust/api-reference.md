# API Reference

Complete reference for the `bulletin-sdk-rust` crate. All public items are available via the `prelude` module:

```rust
use bulletin_sdk_rust::prelude::*;
```

## Clients

### TransactionClient <small>(std only)</small>

High-level async client that handles chain connection, transaction submission, and progress tracking. This is the recommended way to interact with Bulletin Chain.

```rust
impl TransactionClient {
    // Connect to a Bulletin Chain node
    pub async fn new(endpoint: &str) -> Result<Self>;

    // Create from an existing subxt client
    pub fn from_client(api: OnlineClient<PolkadotConfig>) -> Self;

    // Access the underlying subxt client
    pub fn api(&self) -> &OnlineClient<PolkadotConfig>;
}
```

**Store Operations:**

| Method | Returns | Description |
|--------|---------|-------------|
| `store(data, signer)` | `Result<StoreReceipt>` | Store data (auto-chunks if > 2 MiB) |
| `store_with_progress(data, signer, callback)` | `Result<StoreReceipt>` | Store with progress tracking |

**Authorization:**

| Method | Returns | Description |
|--------|---------|-------------|
| `authorize_account(who, transactions, bytes, signer)` | `Result<AuthorizationReceipt>` | Authorize an account (requires sudo) |
| `authorize_preimage(content_hash, max_size, signer)` | `Result<PreimageAuthorizationReceipt>` | Authorize a content hash |
| `refresh_account_authorization(who, signer)` | `Result<()>` | Refresh account authorization expiry |
| `refresh_preimage_authorization(content_hash, signer)` | `Result<()>` | Refresh preimage authorization expiry |
| `remove_expired_account_authorization(who, signer)` | `Result<()>` | Remove expired account authorization |
| `remove_expired_preimage_authorization(content_hash, signer)` | `Result<()>` | Remove expired preimage authorization |

**Queries:**

| Method | Returns | Description |
|--------|---------|-------------|
| `query_account_authorization(who)` | `Result<Option<(u32, u64)>>` | Query authorization (transactions, bytes) |
| `check_authorization_for_store(who, txs, bytes)` | `Result<()>` | Verify sufficient authorization |

**Renewal:**

| Method | Returns | Description |
|--------|---------|-------------|
| `renew(block, index, signer)` | `Result<RenewReceipt>` | Renew storage at block/index |

---

### BulletinClient

Offline client for local operations (CID calculation, data preparation, chunking) without a network connection. Works in both `std` and `no_std` environments.

```rust
impl BulletinClient {
    pub fn new() -> Self;
    pub fn with_config(config: ClientConfig) -> Self;
    pub fn with_auth_manager(self, auth_manager: AuthorizationManager) -> Self;
}
```

**Methods:**

| Method | Returns | Description |
|--------|---------|-------------|
| `prepare_store(data, options)` | `Result<StorageOperation>` | Prepare a single store operation |
| `prepare_store_chunked(data, config, options, callback)` | `Result<(BatchStorageOperation, Option<Vec<u8>>)>` | Prepare chunked store with optional manifest |
| `estimate_authorization(data_size)` | `(u32, u64)` | Estimate (transactions, bytes) needed |
| `prepare_renew(storage_ref)` | `Result<RenewalOperation>` | Prepare a renewal operation |
| `prepare_renew_raw(block, index)` | `Result<RenewalOperation>` | Prepare renewal from raw block/index |

---

### ClientConfig

```rust
pub struct ClientConfig {
    pub default_chunk_size: u32,    // default: 1 MiB
    pub max_parallel: u32,          // default: 8
    pub create_manifest: bool,      // default: true
}
```

---

## Storage Operations

### StorageOperation

Represents a single prepared store operation with data and CID configuration.

```rust
pub struct StorageOperation {
    pub data: Vec<u8>,
    pub cid_config: CidConfig,
    pub wait_finalization: bool,
}

impl StorageOperation {
    pub fn new(data: Vec<u8>, options: StoreOptions) -> Result<Self>;
    pub fn calculate_cid(&self) -> Result<CidData>;
    pub fn size(&self) -> usize;
    pub fn validate(&self) -> Result<()>;
}
```

### BatchStorageOperation

A collection of storage operations for chunked uploads.

```rust
pub struct BatchStorageOperation {
    pub operations: Vec<StorageOperation>,
    pub wait_finalization: bool,
}

impl BatchStorageOperation {
    pub fn new(chunks: &[Chunk], options: StoreOptions) -> Result<Self>;
    pub fn from_chunks(chunk_data: Vec<Vec<u8>>, options: StoreOptions) -> Result<Self>;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn total_size(&self) -> usize;
    pub fn calculate_cids(&self) -> Result<Vec<CidData>>;
}
```

### StoreOptions

```rust
pub struct StoreOptions {
    pub cid_codec: CidCodec,                // default: CidCodec::Raw
    pub hash_algorithm: HashingAlgorithm,    // default: HashingAlgorithm::Blake2b256
    pub wait_for_finalization: bool,         // default: false
}
```

### StoreResult

```rust
pub struct StoreResult {
    pub cid: Vec<u8>,
    pub size: u64,
    pub block_number: Option<u32>,
    pub chunks: Option<ChunkDetails>,
}
```

### ChunkDetails

```rust
pub struct ChunkDetails {
    pub chunk_cids: Vec<Vec<u8>>,
    pub num_chunks: u32,
}
```

---

## Renewal

### RenewalOperation

```rust
pub struct RenewalOperation {
    pub storage_ref: StorageRef,
}

impl RenewalOperation {
    pub fn new(storage_ref: StorageRef) -> Self;
    pub fn from_raw(block: u32, index: u32) -> Self;
    pub fn validate(&self) -> Result<()>;
    pub fn block(&self) -> u32;
    pub fn index(&self) -> u32;
}
```

### RenewalTracker

Tracks stored data entries and their expiry for renewal management.

```rust
impl RenewalTracker {
    pub fn new() -> Self;
    pub fn track(&mut self, storage_ref: StorageRef, content_hash: Vec<u8>, size: u64, retention_period: u32);
    pub fn update_after_renewal(&mut self, old_ref: StorageRef, new_ref: StorageRef, retention_period: u32) -> bool;
    pub fn expiring_before(&self, block: u32) -> Vec<&TrackedEntry>;
    pub fn entries(&self) -> &[TrackedEntry];
    pub fn remove_by_content_hash(&mut self, content_hash: &[u8]) -> bool;
    pub fn clear(&mut self);
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
```

### TrackedEntry

```rust
pub struct TrackedEntry {
    pub storage_ref: StorageRef,
    pub content_hash: Vec<u8>,
    pub size: u64,
    pub expires_at: u32,
}
```

### StorageRef

```rust
pub struct StorageRef {
    pub block: u32,
    pub index: u32,
}

impl StorageRef {
    pub fn new(block: u32, index: u32) -> Self;
}
```

---

## Authorization

### AuthorizationManager

```rust
impl AuthorizationManager {
    pub fn new() -> Self;
    pub fn with_account_auth() -> Self;
    pub fn with_preimage_auth() -> Self;
    pub fn with_auto_refresh(self, enabled: bool) -> Self;

    pub fn check_authorization(
        &self, available: &Authorization, required_size: u64, num_transactions: u32,
    ) -> Result<()>;

    pub fn calculate_requirements(
        &self, total_size: u64, num_chunks: usize, include_manifest: bool,
    ) -> (u32, u64);

    pub fn estimate_authorization(&self, data_size: u64, create_manifest: bool) -> (u32, u64);
}
```

### Authorization

```rust
pub struct Authorization {
    pub scope: AuthorizationScope,
    pub transactions: u32,
    pub max_size: u64,
    pub expires_at: Option<u32>,
}
```

---

## Chunking

### FixedSizeChunker

```rust
impl FixedSizeChunker {
    pub fn new(config: ChunkerConfig) -> Result<Self>;
    pub fn default_config() -> Self;
    pub fn chunk_size(&self) -> usize;
    pub fn num_chunks(&self, data_size: usize) -> usize;
}
```

**Implements:** `Chunker` trait

### Chunker Trait

```rust
pub trait Chunker {
    fn chunk(&self, data: &[u8]) -> Result<Vec<Chunk>>;
    fn validate_chunk_size(&self, size: usize) -> Result<()>;
}
```

### ChunkerConfig

```rust
pub struct ChunkerConfig {
    pub chunk_size: u32,          // default: 1 MiB
    pub max_parallel: u32,        // default: 8
    pub create_manifest: bool,    // default: true
}
```

### Chunk

```rust
pub struct Chunk {
    pub data: Vec<u8>,
    pub index: u32,
    pub total_chunks: u32,
}

impl Chunk {
    pub fn new(data: Vec<u8>, index: u32, total_chunks: u32) -> Self;
    pub fn size(&self) -> usize;
}
```

---

## DAG / Manifests

### UnixFsDagBuilder

Builds DAG-PB (UnixFS) manifests from chunks.

```rust
impl UnixFsDagBuilder {
    pub fn new() -> Self;
}
```

**Implements:** `DagBuilder` trait

### DagBuilder Trait

```rust
pub trait DagBuilder {
    fn build(&self, chunks: &[Chunk], hash_algo: HashingAlgorithm) -> Result<DagManifest>;
}
```

### DagManifest

```rust
pub struct DagManifest {
    pub root_cid: CidData,
    pub chunk_cids: Vec<CidData>,
    pub total_size: u64,
    pub dag_bytes: Vec<u8>,
}
```

---

## CID Functions

```rust
// Calculate CID with explicit codec and hash algorithm
pub fn calculate_cid_with_config(
    data: &[u8], codec: CidCodec, hash_algo: HashingAlgorithm,
) -> Result<CidData>;

// Calculate CID with defaults (Raw codec, Blake2b256)
pub fn calculate_cid_default(data: &[u8]) -> Result<CidData>;

// Encode CID data to bytes
pub fn cid_to_bytes(cid_data: &CidData) -> Result<Cid>;
```

**Re-exported from `transaction_storage_primitives`:**

```rust
pub use transaction_storage_primitives::cids::{
    calculate_cid, Cid, CidConfig, CidData, HashingAlgorithm,
};
pub use transaction_storage_primitives::ContentHash;
```

---

## Enums

### CidCodec

```rust
pub enum CidCodec {
    Raw,              // 0x55 - Raw binary
    DagPb,            // 0x70 - DAG-PB (UnixFS manifests)
    DagCbor,          // 0x71 - DAG-CBOR
    Custom(u64),      // Custom codec code
}

impl CidCodec {
    pub fn code(&self) -> u64;
    pub fn name(&self) -> Cow<'static, str>;
}
```

### HashingAlgorithm

Re-exported from `transaction_storage_primitives`. Variants:
- `Blake2b256` — BLAKE2b-256 (default, Substrate-native)
- `Sha2_256` — SHA2-256
- `Keccak256` — Keccak-256

### AuthorizationScope

```rust
pub enum AuthorizationScope {
    Account,
    Preimage,
}
```

### Error

All 18 error variants with structured metadata.

```rust
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
    pub fn code(&self) -> &'static str;          // e.g., "CHUNK_TOO_LARGE"
    pub fn is_retryable(&self) -> bool;           // true for transient errors
    pub fn recovery_hint(&self) -> &'static str;  // actionable advice
}
```

### ProgressEvent

```rust
pub enum ProgressEvent {
    Chunk(ChunkProgressEvent),
    Transaction(TransactionStatusEvent),
}
```

**Convenience constructors:** `chunk_started`, `chunk_completed`, `chunk_failed`, `manifest_started`, `manifest_created`, `completed`, `tx_validated`, `tx_broadcasted`, `tx_in_best_block`, `tx_finalized`.

### ChunkProgressEvent

```rust
pub enum ChunkProgressEvent {
    ChunkStarted { index: u32, total: u32 },
    ChunkCompleted { index: u32, total: u32, cid: Vec<u8> },
    ChunkFailed { index: u32, total: u32, error: String },
    ManifestStarted,
    ManifestCreated { cid: Vec<u8> },
    Completed { manifest_cid: Option<Vec<u8>> },
}
```

### TransactionStatusEvent

```rust
pub enum TransactionStatusEvent {
    Validated,
    Broadcasted,
    InBestBlock { block_hash: String, block_number: Option<u32>, extrinsic_index: Option<u32> },
    Finalized { block_hash: String, block_number: Option<u32>, extrinsic_index: Option<u32> },
    NoLongerInBestBlock,
    Invalid { error: String },
    Dropped { error: String },
}

impl TransactionStatusEvent {
    pub fn description(&self) -> String;
}
```

---

## Receipt Types <small>(std only)</small>

### StoreReceipt

```rust
pub struct StoreReceipt {
    pub block_hash: String,
    pub extrinsic_hash: String,
    pub data_size: u64,
}
```

### AuthorizationReceipt

```rust
pub struct AuthorizationReceipt {
    pub account: AccountId32,
    pub transactions: u32,
    pub bytes: u64,
    pub block_hash: String,
}
```

### PreimageAuthorizationReceipt

```rust
pub struct PreimageAuthorizationReceipt {
    pub content_hash: ContentHash,
    pub max_size: u64,
    pub block_hash: String,
}
```

### RenewReceipt

```rust
pub struct RenewReceipt {
    pub original_block: u32,
    pub transaction_index: u32,
    pub block_hash: String,
}
```

---

## Type Aliases

```rust
pub type Result<T> = core::result::Result<T, Error>;
pub type ProgressCallback = Arc<dyn Fn(ProgressEvent) + Send + Sync>;
```

---

## Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `MAX_CHUNK_SIZE` | `2 * 1024 * 1024` (2 MiB) | Maximum single chunk size |
| `MAX_FILE_SIZE` | `64 * 1024 * 1024` (64 MiB) | Maximum total file size |
| `DEFAULT_CHUNK_SIZE` | `1024 * 1024` (1 MiB) | Default chunk size |
| `VERSION` | `env!("CARGO_PKG_VERSION")` | Crate version string |
