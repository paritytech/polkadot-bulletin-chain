# API Reference

Complete reference for the `bulletin-sdk-rust` crate. All public items except `VERSION` (crate root) and the `compat` module are available via the `prelude` module:

```rust
use bulletin_sdk_rust::prelude::*;
```

## Clients

### TransactionClient <small>(std only)</small>

High-level async client that handles chain connection, transaction submission, and progress tracking. This is the recommended way to interact with Bulletin Chain.

```rust
impl TransactionClient {
    // Connect to a single Bulletin Chain node (WS)
    pub async fn new(endpoint: &str) -> Result<Self>;

    // Connect over multiple WS endpoints — the first is the monitor; every
    // endpoint is a broadcast target (one dead endpoint can't stall a run)
    pub async fn from_endpoints(endpoints: &[&str]) -> Result<Self>;

    // The provider abstraction: build the client from pre-made RpcClients.
    // Each RpcClient is a "provider" — WS (`RpcClient::from_url`) OR a smoldot
    // light client (`lc.parachain(spec)?.into()`). The SDK is agnostic to which.
    // First = monitor; all = broadcast targets. Mirrors PAPI's `providers`.
    pub async fn from_rpc_clients(clients: Vec<RpcClient>) -> Result<Self>;

    // Create from a single existing RpcClient
    pub async fn from_rpc_client(rpc_client: RpcClient) -> Result<Self>;

    // Block that the SDK's own chain reads use (authorization queries, the `store`
    // pre-check, `skip_existing`): WaitFor::Finalized (default) or WaitFor::InBlock (best block)
    pub fn with_wait_for(self, wait_for: WaitFor) -> Self;

    // Access the underlying subxt client
    pub fn api(&self) -> &OnlineClient<PolkadotConfig>;
}
```

**Light client (smoldot).** The SDK doesn't depend on smoldot or know whether a
provider is a node connection or a light client — it just takes `RpcClient`s.
To use a light client, enable `subxt`'s `unstable-light-client` feature in *your*
crate, build the provider, and pass it to `from_rpc_clients` (keep the
`LightClient` alive for the connection's lifetime):

```rust
use subxt::lightclient::LightClient;
let (lc, _relay) = LightClient::relay_chain(RELAY_CHAIN_SPEC)?;   // Bulletin is a parachain
let bulletin: subxt::backend::rpc::RpcClient = lc.parachain(BULLETIN_SPEC)?.into();
let client = TransactionClient::from_rpc_clients(vec![bulletin]).await?;
// hold `lc` for as long as `client` is used
```

**Upload (the primary path — `estimate_upload` → `submit`):**

| Method | Returns | Description |
|--------|---------|-------------|
| `estimate_upload(input, options)` | `Result<StreamEstimate>` | Plan an upload (`UploadInput::Items` or `::Source`) and size authorization; collapses repeated content in the input, and leaves out units already on chain when `skip_existing` is set |
| `submit(signer, estimate, source, config)` | `Result<UploadResult>` | Upload to finality; bytes fetched lazily from `source` (`blob_from_items(..)` for items, `blob_from_bytes` for a file; buffer a `blob_from_factory` stream with `collect_blob` first). Exactly-once |
| `submit_unsigned(estimate, source, config)` | `Result<UploadResult>` | As above via the unsigned (preimage-authorized) path — no signer |

**Single-item store (convenience):**

| Method | Returns | Description |
|--------|---------|-------------|
| `store(data, signer, wait_for)` | `Result<StoreReceipt>` | Store one payload |
| `store_with_progress(data, signer, wait_for, callback)` | `Result<StoreReceipt>` | Store one payload with progress tracking |

**Authorization:**

| Method | Returns | Description |
|--------|---------|-------------|
| `authorize_account(who, transactions, bytes, signer, wait_for)` | `Result<AuthorizationReceipt>` | Authorize an account (signer must be a registered authorizer) |
| `authorize_accounts(entries, sudo, signer, wait_for)` | `Result<Vec<AuthorizationReceipt>>` | Authorize many accounts atomically (`Utility.batch_all`); `sudo: true` wraps in `Sudo.sudo` |
| `authorize_preimage(content_hash, max_size, signer, wait_for)` | `Result<PreimageAuthorizationReceipt>` | Authorize a content hash |
| `refresh_account_authorization(who, signer, wait_for)` | `Result<()>` | Refresh account authorization expiry |
| `refresh_preimage_authorization(content_hash, signer, wait_for)` | `Result<()>` | Refresh preimage authorization expiry |
| `remove_expired_account_authorization(who, signer, wait_for)` | `Result<()>` | Remove expired account authorization |
| `remove_expired_preimage_authorization(content_hash, signer, wait_for)` | `Result<()>` | Remove expired preimage authorization |

**Queries:**

| Method | Returns | Description |
|--------|---------|-------------|
| `query_account_authorization(who)` | `Result<Option<(u32, u64)>>` | Remaining `(transactions, bytes)`; `None` if absent or expired; `Err(AuthorizationApiUnavailable)` on runtimes without the `BulletinTransactionStorageApi` runtime API |

**Renewal:**

| Method | Returns | Description |
|--------|---------|-------------|
| `renew(entry, signer, wait_for)` | `Result<RenewReceipt>` | Schedule a one-shot renewal at the retention boundary; `entry` is a `(block, index)` tuple or a `ContentHash` |
| `force_renew(entry, signer, wait_for)` | `Result<RenewReceipt>` | Renew immediately |
| `enable_auto_renew(content_hash, signer, wait_for)` | `Result<()>` | Register recurring auto-renewal; the first cycle is prepaid |
| `disable_auto_renew(content_hash, signer, wait_for)` | `Result<()>` | Cancel a renewal registration; refused while its next cycle is prepaid |

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
    pub default_chunk_size: u32,    // default: 2 MiB
    pub max_parallel: u32,          // default: 8
    pub create_manifest: bool,      // default: true
}
```

---

## Upload Pipeline <small>(std only)</small>

### UploadInput

```rust
pub enum UploadInput {
    Items(Vec<UploadItem>),           // each item is one unit, no manifest
    Source(Arc<dyn SeekableSource>),  // chunked; DAG-PB manifest when more than one chunk
}
```

### UploadItem

```rust
pub struct UploadItem {
    pub data: Vec<u8>,
    pub codec: Option<CidCodec>,             // default: CidCodec::Raw
    pub hash_algo: Option<HashingAlgorithm>, // default: HashingAlgorithm::Blake2b256
}

impl UploadItem {
    pub fn new(data: Vec<u8>) -> Self;
}
```

### UploadEstimateOptions

```rust
pub struct UploadEstimateOptions {
    pub skip_existing: bool,     // default: false; true leaves out units already on chain
    pub dedup_input: bool,       // default: true; collapses repeated content in the input
    pub chunker: ChunkerConfig,  // Source input only
    pub store: StoreOptions,     // Source input only (hash algorithm)
}
```

### StreamEstimate

```rust
pub struct StreamEstimate {
    pub base: UploadEstimate,
    pub plan: ChunkPlan, // pass to `submit` unchanged
}

pub struct UploadEstimate {
    pub total: usize,                    // units in the plan (chunks + manifest)
    pub items: Vec<UploadEstimateItem>,
    pub transactions: u32,               // store transactions to submit
    pub bytes: u64,                      // bytes those transactions store
    pub duplicate_indices: Vec<usize>,
    pub already_stored: Vec<usize>,      // only with skip_existing
    pub to_upload: Vec<usize>,
}

pub struct UploadEstimateItem {
    pub index: usize,
    pub cid: Vec<u8>,
    pub bytes: u64,
    pub skip_reason: Option<SkipReason>, // None if the unit is submitted
}

pub enum SkipReason {
    DuplicateInput,
    AlreadyOnChain,
}

pub struct ChunkPlan {
    pub chunk_cids: Vec<CidData>,
    pub chunk_sizes: Vec<u64>,
    pub offsets: Vec<u64>,
    pub codecs: Vec<CidCodec>,
    pub hash_algos: Vec<HashingAlgorithm>,
    pub total_size: u64,
    pub root_cid: Option<CidData>,      // manifest root; None without a manifest
    pub manifest_data: Option<Vec<u8>>,
}
```

### UploadConfig

```rust
pub struct UploadConfig {
    pub complete_on: WaitFor,                        // default: WaitFor::Finalized
    pub block_limits: BlockLimits,                   // default: DEFAULT_BLOCK_LIMITS
    pub on_event: Option<UploadCallback>,            // default: None
    pub submission_strategy: SubmissionStrategyKind, // default (and only variant): NonceTracking
}

pub type UploadCallback = Arc<dyn Fn(UploadEvent) + Send + Sync>;
```

### UploadEvent

```rust
pub enum UploadEvent {
    ItemStarted { index: usize, total: usize, cid: Vec<u8> },
    // Fires again if a reorg moves the item to another block
    ItemInBlock {
        index: usize, total: usize, cid: Vec<u8>,
        block_hash: String, block_number: Option<u32>, transaction_index: Option<u32>,
    },
    ItemFinalized {
        index: usize, total: usize, cid: Vec<u8>,
        block_hash: String, block_number: Option<u32>, transaction_index: Option<u32>,
    },
    ItemFailed { index: usize, total: usize, cid: Vec<u8>, error: String },
}
```

`UploadStatus` (`ItemStarted`, `ItemInBlock`, `ItemFinalized`, `ItemFailed`) names the same stages without data.

### UploadResult

```rust
pub struct UploadResult {
    pub cids: Vec<Vec<u8>>, // one per plan unit, manifest root last
}
```

### BlockLimits

```rust
pub struct BlockLimits {
    pub max_normal_weight: u64,
    pub normal_block_length: u32,
    pub max_block_transactions: u32,
    pub store_weight_base: u64,
    pub store_weight_per_byte: u64,
    pub extrinsic_overhead: u32,
}
```

### Byte Sources

```rust
pub trait BlobSource: Send + Sync {
    fn size_hint(&self) -> Option<u64>;
    fn open(&self) -> BoxStream<'_, Result<Vec<u8>>>; // re-openable forward read
}

pub trait SeekableSource: BlobSource {
    fn total_size(&self) -> u64;
    fn read(&self, offset: u64, length: u64) -> BoxFuture<'_, Result<Arc<[u8]>>>;
}

pub fn blob_from_bytes(data: Vec<u8>) -> impl SeekableSource;
pub fn blob_from_items(items: Vec<Vec<u8>>) -> impl SeekableSource;
pub fn blob_from_factory<F, S>(open: F, size: Option<u64>) -> impl BlobSource; // forward-only
pub async fn collect_blob(source: &dyn BlobSource) -> Result<Vec<u8>>;
pub async fn plan_stream<S: BlobSource + ?Sized>(
    source: &S, config: &ChunkerConfig, options: &StoreOptions,
) -> Result<ChunkPlan>;
```

`submit` takes a `SeekableSource`. A `blob_from_factory` source is forward-only: buffer it with `collect_blob` and pass the bytes to `blob_from_bytes`, or implement `SeekableSource` for your file type. `plan_stream` holds about one chunk in memory only when `open()` yields bounded parts; `blob_from_bytes` yields the whole buffer as one part.

### Submission Strategy Types

`SubmissionStrategy`, `NonceTrackingStrategy`, `BroadcastArgs`, `ItemBroadcastResult`, and `WaveResult` describe how signed extrinsics reach the chain. `UploadConfig` selects a strategy only through `SubmissionStrategyKind`.

### compat Module

Resolves which renewal call shape the connected chain uses:

```rust
pub fn renew_adapter(live: &subxt::Metadata) -> Result<RenewAdapter>;

pub enum RenewAdapter {
    DataRenewal,    // DataRenewal.renew(entry): current runtimes
    TransactionRef, // TransactionStorage.renew(entry): before the renewal split
    Positional,     // TransactionStorage.renew(block, index): legacy runtimes
}
```

`renew_adapter` returns `UnsupportedOperation` when the chain has no `renew` call or the call shape is unknown.

---

## Storage Operations

### StorageOperation

Represents a single prepared store operation with data and CID configuration.

```rust
pub struct StorageOperation {
    pub data: Vec<u8>,
    pub cid_config: CidConfig,
    pub wait_for: WaitFor,
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
    pub wait_for: WaitFor,
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
    pub wait_for: WaitFor,                   // default: WaitFor::Finalized
}
```

### StoreResult

No SDK method returns `StoreResult`; the type is exported for applications that record upload results.

```rust
pub struct StoreResult {
    pub cid: Vec<u8>,
    pub size: u64,
    pub block_number: Option<u32>,
    pub transaction_index: Option<u32>, // `Stored` event index, the `renew` slot
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

    pub fn calculate_requirements(
        &self, total_size: u64, num_chunks: usize, include_manifest: bool,
    ) -> (u32, u64);

    pub fn estimate_authorization(&self, data_size: u64, create_manifest: bool) -> (u32, u64);
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
    pub chunk_size: u32,          // default: 2 MiB
    pub max_parallel: u32,        // default: 8
    pub create_manifest: bool,    // default: true
}
```

### Chunk

```rust
pub struct Chunk {
    pub data: Vec<u8>,
    pub cid: Option<Vec<u8>>,
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

    // Build a manifest from chunk CIDs and sizes, without the chunk data
    pub fn build_from_parts(
        &self, chunk_cids: &[CidData], chunk_sizes: &[u64], hash_algo: HashingAlgorithm,
    ) -> Result<DagManifest>;
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

**Re-exported from `bulletin_transaction_storage_primitives`:**

```rust
pub use bulletin_transaction_storage_primitives::cids::{
    calculate_cid, Cid, CidConfig, CidData, HashingAlgorithm,
};
pub use bulletin_transaction_storage_primitives::{ContentHash, TransactionRef};
```

---

## Enums

### WaitFor

Transaction confirmation level. Controls when submission methods resolve.

```rust
pub enum WaitFor {
    InBlock,      // Return when tx is in a best block (faster, may reorg)
    Finalized,    // Return when tx is finalized (safer, slower)
}
```

Default: `WaitFor::Finalized`. Every submission method takes its `wait_for` explicitly; `TransactionClient::with_wait_for` sets the block that the SDK's own chain reads use.

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

Re-exported from `bulletin_transaction_storage_primitives`. Variants:
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

All 18 error variants with structured metadata. The codes are shared with the TypeScript SDK; see [Error Handling](./error-handling.md).

```rust
pub enum Error {
    ChunkTooLarge(u64),
    DataTooLarge(String),
    EmptyData,
    InvalidCid(String),
    AuthorizationNotFound(String),
    DagEncodingFailed(String),
    NetworkError(String),
    InvalidConfig(String),
    ChunkingFailed(String),
    CidCalculationFailed(String),
    TransactionFailed(String),
    TransactionInvalid { custom_code: Option<u8>, message: String },
    InvalidChunkSize(String),
    UnsupportedOperation(String),
    AuthorizationApiUnavailable,
    StoreStalled(String),
    HijackBudgetExceeded(String),
    UploadIncomplete(String),
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

**Convenience constructors:** `chunk_started`, `chunk_completed`, `chunk_failed`, `manifest_started`, `manifest_created`, `completed`, `tx_validated`, `tx_broadcasted`, `tx_in_block`, `tx_finalized`.

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
    Signed { tx_hash: String, chunk_index: Option<u32> },
    Validated { chunk_index: Option<u32> },
    Broadcasted { chunk_index: Option<u32> },
    InBlock { block_hash: String, block_number: Option<u32>, tx_index: Option<u32>, chunk_index: Option<u32> },
    Finalized { block_hash: String, block_number: Option<u32>, tx_index: Option<u32>, chunk_index: Option<u32> },
    NoLongerInBlock { chunk_index: Option<u32> },
    Invalid { error: String, chunk_index: Option<u32> },
    Dropped { error: String, chunk_index: Option<u32> },
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
    pub entry: TransactionRef<u32>, // the renewed position or content hash
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
| `VERSION` (crate root) | `env!("CARGO_PKG_VERSION")` | Crate version string |
| `DEFAULT_BLOCK_LIMITS` | `BlockLimits { .. }` | Block capacity of bulletin-westend / bulletin-paseo runtimes, used to size upload waves |

`FixedSizeChunker` (and `prepare_store_chunked`) enforce a chunk size between 1 byte and 2 MiB and a total size of at most 64 MiB; these limits are not exported as constants. The default chunk size is 2 MiB.
