# Bulletin Chain SDK Design

## Status

**Proposal** — This document describes the target architecture for the Bulletin Chain client SDK. It supersedes the current dual-SDK implementation (PR #202) which covers only write operations.

## Problem Statement

The Bulletin Chain pallet (`pallet-transaction-storage`) exposes 10 extrinsics, multiple storage queries, event subscriptions, and integrates with IPFS for data retrieval. The current SDK covers approximately 30-40% of this surface, limited to write-only operations, with no data retrieval, no state queries, and no event subscriptions.

Additionally, the current approach maintains two independent implementations (Rust and TypeScript) of the same deterministic logic (CID calculation, chunking, DAG-PB manifest encoding). This doubles the bug surface and has already produced behavioral divergence between the two SDKs.

## Current State Analysis

Source-code-verified review of the SDK as of PR #202 (naren-sdk branch). Every finding below was confirmed by reading the actual source files.

### Rust SDK

**Two non-working clients with different interfaces:**

- `TransactionClient` (`transaction.rs`) implements all 9 pallet extrinsics but requires a `metadata.scale` file at compile time (`#[subxt::subxt(runtime_metadata_path = "metadata.scale")]` at line 19) that does not exist in the repository. The crate cannot compile with this module enabled.
- `AsyncBulletinClient` (`async_client.rs`) has a builder pattern API but every submission method is a stub returning `Err("not yet implemented")`. Only `estimate_authorization()` does real work.
- `MockBulletinClient` (`mock_client.rs`) has a different interface (`operations()`, `clear_operations()`) from either real client. There is no shared trait between mock and real implementations.

**Hardcoded signer type:**

`TransactionClient` takes `signer: &Keypair` (hardcoded to `subxt_signer::sr25519::Keypair`) on every method. This locks out ed25519, ecdsa, and hardware wallet signers. The subxt library already provides `trait Signer<T: Config>` with implementations for sr25519, ed25519, and ecdsa — the SDK should accept `impl Signer<PolkadotConfig>` instead.

**Unused `BulletinConfig`:**

`subxt_config.rs` defines `BulletinConfig` with a custom `ProvideCidConfig` signed extension, but neither client uses it — both construct `OnlineClient<PolkadotConfig>`. This means the custom signed extension for CID configuration is dead code.

**Transport is already agnostic (no custom trait needed):**

`subxt::OnlineClient<T>` (v0.44.2) already supports multiple transports out of the box:
- `OnlineClient::from_url(url)` — WebSocket RPC
- `OnlineClient::from_insecure_url(url)` — Insecure WebSocket RPC
- `OnlineClient::from_rpc_client(rpc)` — Any `impl RpcClientT`
- `OnlineClient::from_backend(backend)` — Any `impl Backend<T>`

This means accepting `OnlineClient<PolkadotConfig>` already allows WebSocket, smoldot light client, or any custom backend. No custom `BulletinTransport` trait is needed on the Rust side.

### TypeScript SDK

**`api: any` defeats type safety:**

`AsyncBulletinClient` constructor accepts `api: any` (`async-client.ts` line 85). The `TypedApi` import exists (line 9) but is never used in any type annotation — it's a dead import. This means zero compile-time checking of pallet call names, parameter types, or return types. A typo in `this.api.tx.TransactionStorage.store(...)` would only fail at runtime.

**Inconsistent `Binary` handling:**

Most methods correctly wrap data as `Binary(data)` for PAPI submission, but `storeWithPreimageAuth` passes a raw `Uint8Array` instead of `Binary`. This inconsistency may cause runtime encoding errors.

**`waitForFinalization` option is a no-op:**

`StoreOptions` defines a `waitForFinalization` boolean, but the submission code always calls `signAndSubmitFinalized()` — the option is never checked. There is no code path for fire-and-forget submission.

**Mock client is incomplete:**

`MockBulletinClient` is missing `storeChunked`, `renew`, `storeWithPreimageAuth`, and `sendUnsigned` methods that exist on the real client. There is no shared interface or abstract base class enforcing method parity.

### Both SDKs

**No read operations:** Neither SDK can retrieve stored data, query on-chain state (authorizations, fees, retention period), or subscribe to events. Users must drop to raw subxt/PAPI calls for any read operation.

**Duplicate pure logic:** CID calculation, chunking, DAG-PB manifest building, content hashing, and fee estimation are independently reimplemented in both languages with divergent behavior (e.g., `optimalChunkSize` returns different values for the same input).

## Design Principles

1. **Transport-agnostic via existing abstractions.** subxt's `OnlineClient<T>` and PAPI's `TypedApi` already abstract transport. The SDK accepts these types — it does not define its own transport traits. Users plug in WebSocket, smoldot, or custom backends through subxt/PAPI's existing extension points.

2. **Accept connections, don't create them.** Dapp developers already have established node connections and wallet signers. The SDK should compose with what exists, not force its own transport.

3. **Pure logic has zero dependencies on transport.** CID calculation, chunking, and manifest building are deterministic computations. They should work in `no_std`, WASM, and any JS runtime without a node connection.

4. **One implementation, multiple targets.** Core logic is implemented once in Rust, compiled to WASM for JavaScript/TypeScript consumption. Platform-specific transaction submission stays native.

5. **Cover the full pallet surface.** The SDK should expose every extrinsic, every queryable storage item, and every subscribable event. Users should not need to drop down to raw RPC calls for standard operations.

## Architecture

The SDK is structured in three layers. Each layer builds on the one below and can be used independently.

```
+---------------------------------------------+
|  Layer 3: Connected Client                   |
|  Accepts user's connection (subxt / PAPI)    |
|  Submits transactions, queries state,        |
|  subscribes to events                        |
+---------------------------------------------+
                     |
+---------------------------------------------+
|  Layer 2: Pallet-Aware Operations            |
|  Builds extrinsic payloads, decodes storage  |
|  results, filters events                     |
|  Knows Bulletin Chain types, not transport   |
+---------------------------------------------+
                     |
+---------------------------------------------+
|  Layer 1: Pure Functions                     |
|  CID calculation, chunking, DAG-PB,         |
|  content hashing, validation                 |
|  Deterministic, no side effects              |
+---------------------------------------------+
```

### Layer 1: Pure Functions

Deterministic computation with no I/O. This layer compiles to `no_std` Rust and to WASM for JavaScript consumption.

**Dependencies:** `cid`, `multihash`, `unsigned-varint`, `blake2` (no Substrate crates).

**Capabilities:**

| Function | Description |
|---|---|
| `calculate_cid(data, codec, hash_algo)` | Compute CIDv1 for arbitrary data |
| `content_hash(data, hash_algo)` | Compute content hash (Blake2b-256, SHA2-256, Keccak-256) |
| `chunk(data, config)` | Split data into fixed-size chunks |
| `build_manifest(chunks, hash_algo)` | Build DAG-PB UnixFS v1 manifest from chunks |
| `parse_manifest(dag_bytes)` | Parse DAG-PB manifest, extract chunk CIDs and sizes |
| `reassemble(chunks)` | Reassemble ordered chunks into original data |
| `validate_chunk_size(size)` | Validate chunk size is within Bitswap-compatible bounds |
| `optimal_chunk_size(data_size)` | Suggest chunk size for a given total data size |
| `cid_to_bytes(cid)` / `cid_from_bytes(bytes)` | CID serialization |
| `cid_to_string(cid)` / `cid_from_string(s)` | CID string encoding (base32) |
| `estimate_authorization(data_size, create_manifest)` | Estimate transactions + bytes needed for authorization |

**Usage examples:**

```rust
// Rust — works in no_std, native, WASM
let cid = bulletin_sdk::calculate_cid(data, CidCodec::Raw, HashAlgorithm::Blake2b256)?;
let chunks = bulletin_sdk::chunk(data, ChunkerConfig::default())?;
let manifest = bulletin_sdk::build_manifest(&chunks, HashAlgorithm::Blake2b256)?;
```

```typescript
// TypeScript — backed by WASM, works in browser and Node.js
import { calculateCid, chunk, buildManifest } from "@bulletin/sdk";

const cid = calculateCid(data);
const chunks = chunk(data);
const manifest = buildManifest(chunks);
```

### Layer 2: Pallet-Aware Operations

Knows the Bulletin Chain's pallet types and encoding, but not how to talk to a node. Produces encodable call data and storage keys; parses raw responses into typed results.

**Dependencies:** Layer 1 + SCALE codec types for `pallet-transaction-storage` calls, storage keys, and events. These types should come from a lightweight shared crate (`bulletin-primitives`), not from the full pallet.

**Capabilities:**

```
calls.store(data, cid_config?)           -> EncodedCall
calls.renew(block, index)                -> EncodedCall
calls.authorize_account(who, txns, bytes)-> EncodedCall
calls.authorize_preimage(hash, max_size) -> EncodedCall
calls.refresh_account_auth(who)          -> EncodedCall
calls.refresh_preimage_auth(hash)        -> EncodedCall
calls.remove_expired_account_auth(who)   -> EncodedCall
calls.remove_expired_preimage_auth(hash) -> EncodedCall

queries.authorization(account)           -> StorageQuery<AuthorizationExtent>
queries.preimage_authorization(hash)     -> StorageQuery<AuthorizationExtent>
queries.retention_period()               -> StorageQuery<BlockNumber>
queries.byte_fee()                       -> StorageQuery<Balance>
queries.entry_fee()                      -> StorageQuery<Balance>
queries.transaction_roots(block)         -> StorageQuery<Vec<TransactionInfo>>

events.stored()                          -> EventDecoder<StoredEvent>
events.renewed()                         -> EventDecoder<RenewedEvent>
events.account_authorized()              -> EventDecoder<AccountAuthorizedEvent>
events.account_auth_refreshed()          -> EventDecoder<AccountAuthRefreshedEvent>
events.preimage_authorized()             -> EventDecoder<PreimageAuthorizedEvent>
events.preimage_auth_refreshed()         -> EventDecoder<PreimageAuthRefreshedEvent>
events.expired_account_auth_removed()    -> EventDecoder<ExpiredAccountAuthRemovedEvent>
events.expired_preimage_auth_removed()   -> EventDecoder<ExpiredPreimageAuthRemovedEvent>
```

This layer enables advanced use cases (custom batching, offline signing, indexer integration) without requiring a connected client.

### Layer 3: Connected Client

Thin wrapper that connects Layer 2 operations to a live chain connection. Uses existing abstractions from subxt and PAPI — no custom transport traits needed.

#### Why No Custom Transport Trait

subxt (Rust) and PAPI (TypeScript) already provide the transport abstraction:

- **subxt `OnlineClient<T>`** accepts any `impl RpcClientT` via `from_rpc_client()` or any `impl Backend<T>` via `from_backend()`. WebSocket, smoldot, and custom transports all produce the same `OnlineClient<T>` type.
- **PAPI `TypedApi`** is constructed from any provider (WebSocket, smoldot, custom). The `TypedApi` type is identical regardless of transport.

Wrapping these behind a custom `BulletinTransport` trait would add indirection without enabling any new transport that subxt/PAPI don't already support. The SDK should accept the library types directly.

#### Signer Abstraction

**Rust — use subxt's existing `Signer` trait:**

subxt defines `trait Signer<T: Config>` with implementations for sr25519, ed25519, and ecdsa key types. The SDK should accept `impl Signer<PolkadotConfig>` at client construction instead of hardcoding `subxt_signer::sr25519::Keypair` on every method.

```rust
// Current (broken): hardcoded sr25519 on every method
pub async fn store(&self, data: Vec<u8>, signer: &Keypair) -> Result<StoreReceipt>

// Target: signer stored in client, accepts any key type
let client = BulletinClient::new(api, signer); // signer: impl Signer<PolkadotConfig>
client.store(data).send().await?;
```

**TypeScript — already correct via PAPI:**

PAPI's `PolkadotSigner` interface already supports wallet extensions, programmatic keys (sr25519/ed25519/ecdsa), and hardware wallets. The TypeScript SDK already accepts this correctly. No changes needed for signer abstraction on the TypeScript side.

#### Constructor

```rust
// Rust — accepts any OnlineClient (WebSocket, smoldot, custom backend)
let api = OnlineClient::<PolkadotConfig>::from_url("ws://localhost:9944").await?;
let client = BulletinClient::new(api, signer);

// Or with smoldot light client (same OnlineClient type)
let api = OnlineClient::<PolkadotConfig>::from_rpc_client(smoldot_rpc).await?;
let client = BulletinClient::new(api, signer);

// Read-only client (no signer needed for queries)
let client = BulletinClient::read_only(api);
```

```typescript
// TypeScript — accepts any PAPI TypedApi (WebSocket, smoldot, custom provider)
const wsProvider = getWsProvider("ws://localhost:9944");
const papiClient = createClient(wsProvider);
const api = papiClient.getTypedApi(bulletin);
const client = new BulletinClient(api, signer);

// Or with smoldot (same TypedApi type)
const smoldotProvider = getSmProvider(chain);
const papiClient = createClient(smoldotProvider);
const api = papiClient.getTypedApi(bulletin);
const client = new BulletinClient(api, signer);

// Read-only client
const client = BulletinClient.readOnly(api);
```

#### Write Operations

```
client.store(data) -> StoreBuilder
  .with_codec(codec)
  .with_hash_algorithm(algo)
  .with_callback(progress_fn)
  .send() -> StoreResult { cid, block_number, index, size }

client.store_chunked(data, config?) -> ChunkedStoreResult

client.renew(block, index) -> RenewResult
client.authorize_account(who, txns, bytes) -> TxReceipt
client.authorize_preimage(hash, max_size) -> TxReceipt
client.refresh_account_auth(who) -> TxReceipt
client.refresh_preimage_auth(hash) -> TxReceipt
client.remove_expired_account_auth(who) -> TxReceipt
client.remove_expired_preimage_auth(hash) -> TxReceipt
```

#### Read Operations (currently missing)

```
client.get_data(block, index) -> Vec<u8>
client.get_data_by_cid(cid, gateway?) -> Vec<u8>
client.get_chunked_data(manifest_cid, gateway?) -> Vec<u8>
```

#### State Queries (currently missing)

```
client.query_authorization(account) -> AuthorizationExtent
client.query_preimage_authorization(hash) -> AuthorizationExtent
client.query_retention_period() -> BlockNumber
client.query_fees() -> { byte_fee, entry_fee }
client.query_transaction_info(block) -> Vec<TransactionInfo>
```

#### Event Subscriptions (currently missing)

```
client.on_stored(callback) -> Subscription
client.on_renewed(callback) -> Subscription
client.on_account_authorized(callback) -> Subscription
client.on_account_auth_refreshed(callback) -> Subscription
client.on_preimage_authorized(callback) -> Subscription
client.on_preimage_auth_refreshed(callback) -> Subscription
client.on_expired_account_auth_removed(callback) -> Subscription
client.on_expired_preimage_auth_removed(callback) -> Subscription
```

#### Read-Only vs Read-Write Clients

Not all SDK usage requires a signer. State queries, fee estimation, and data retrieval are read-only operations. The SDK should support read-only clients that don't require a signer at construction time:

```rust
// Read-only: queries, fee estimation, data retrieval
let reader = BulletinClient::read_only(api);
let fees = reader.query_fees().await?;
let auth = reader.query_authorization(account).await?;

// Read-write: adds transaction submission
let client = BulletinClient::new(api, signer);
client.store(data).send().await?;
```

```typescript
// Read-only
const reader = BulletinClient.readOnly(api);
const fees = await reader.queryFees();

// Read-write
const client = new BulletinClient(api, signer);
await client.store(data);
```

This is a departure from the current design where `AsyncBulletinClient` in TypeScript requires a signer at construction even for pure computations like `estimateAuthorization()` that need no chain access or signing.

#### Testing: Shared Interface for Mock and Real Clients

Both Rust and TypeScript should define a shared interface/trait that both the real client and mock client implement. This ensures mock completeness and enables generic code:

```rust
// Rust: trait implemented by both BulletinClient and MockBulletinClient
#[async_trait]
pub trait BulletinApi {
    /// Store data (single transaction, no chunking).
    async fn store(&self, data: Vec<u8>, options: StoreOptions) -> Result<StoreResult>;
    async fn store_chunked(&self, data: Vec<u8>, config: ChunkerConfig, options: StoreOptions) -> Result<ChunkedStoreResult>;
    async fn renew(&self, block: u32, index: u32) -> Result<TxReceipt>;
    async fn authorize_account(&self, who: AccountId32, txns: u32, bytes: u64) -> Result<TxReceipt>;
    // ... all write + read + query operations
}
```

Note: The real client additionally provides a builder pattern via `client.store(data).with_codec(...).send()`. The trait defines the underlying method the builder calls. Both the builder and mock client route through the same trait method.

```typescript
// TypeScript: interface implemented by both AsyncBulletinClient and MockBulletinClient
interface BulletinApi {
    store(data: Uint8Array, options?: StoreOptions): Promise<StoreResult>;
    storeChunked(data: Uint8Array, config?: ChunkerConfig, options?: StoreOptions): Promise<ChunkedStoreResult>;
    renew(block: number, index: number): Promise<TxReceipt>;
    authorizeAccount(who: string, txns: number, bytes: bigint): Promise<TxReceipt>;
    // ... all write + read + query operations
}
```

## Single Implementation via Rust-to-WASM

### Current Problem

Both SDKs independently implement the same deterministic logic:

| Logic | Rust implementation | TypeScript implementation |
|---|---|---|
| CID calculation | `sdk/rust/src/cid.rs` | `multiformats` npm package |
| Chunking | `sdk/rust/src/chunker.rs` | `sdk/typescript/src/chunker.ts` |
| DAG-PB encoding | `sdk/rust/src/dag.rs` (manual protobuf) | `ipfs-unixfs` + `@ipld/dag-pb` |
| Varint encoding | `sdk/rust/src/dag.rs` (manual) | handled by `@ipld/dag-pb` |
| Content hashing | `sp-io::hashing::blake2_256` | `@polkadot/util-crypto` |
| Hex utils | `sdk/rust/src/utils.rs` | `sdk/typescript/src/utils.ts` |
| Fee estimation | `sdk/rust/src/utils.rs` | `sdk/typescript/src/utils.ts` |

Two implementations means two places for bugs. The PR review already identified behavioral divergence in `optimalChunkSize`, missing input validation in the TypeScript `hexToBytes`, and differing error handling between the two.

### Target Architecture

```
bulletin-sdk-core (Rust, no_std)
├── Compiles natively for Rust consumers
├── Compiles to WASM via wasm-pack for JS/TS consumers
└── Dependencies: cid, multihash, unsigned-varint, blake2
    (NO sp-core, sp-runtime, sp-io, pallet-transaction-storage)

@bulletin/sdk-core (npm, WASM blob)
└── Auto-generated JS bindings from wasm-pack

@bulletin/sdk (npm, TypeScript)
├── Imports @bulletin/sdk-core for pure functions
├── Imports polkadot-api for transaction submission
└── Thin Layer 3 client with PAPI integration

bulletin-sdk (crates.io, Rust)
├── Imports bulletin-sdk-core for pure functions
├── Imports subxt for transaction submission (std feature)
└── Layer 3 client with subxt integration
```

### Removing Heavy Substrate Dependencies

The current Rust SDK pulls in the entire FRAME pallet system through four dependencies that are used minimally:

| Dependency | Current usage | Replacement |
|---|---|---|
| `pallet-transaction-storage` | `calculate_cid` fn + 6 types | Extract to `bulletin-primitives` crate (~50 lines) |
| `sp-runtime` | `AccountId32` type | `subxt::utils::AccountId32` (std) or `[u8; 32]` newtype (no_std) |
| `sp-core` | `Ss58Codec` trait | `subxt` provides this (std) or `ss58-registry` crate |
| `sp-io` | `blake2_256()` function | `blake2` crate: `Blake2b256::digest(data)` |

After this change, the `no_std` core has zero Substrate dependencies. The WASM binary shrinks from potentially megabytes to approximately 50-100KB gzipped.

### Shared Primitives Crate

CID types and calculation logic currently live inside the pallet (`pallet-transaction-storage::cids`). This forces the SDK to depend on the entire pallet to use them.

Extract these into a standalone crate:

```
bulletin-primitives/
├── Cargo.toml          # no_std, minimal deps: codec, scale-info, multihash
└── src/
    ├── lib.rs
    ├── cid.rs          # CidConfig, CidData, ContentHash, HashingAlgorithm, calculate_cid
    └── authorization.rs # AuthorizationExtent, authorization types
```

Both the pallet and the SDK depend on `bulletin-primitives`. Neither depends on the other.

## Pallet Feature Coverage

Complete mapping of pallet functionality to SDK methods:

### Extrinsics (10 total)

| Extrinsic | Rust coverage | TypeScript coverage | Target |
|---|---|---|---|
| `store(data)` | `TransactionClient`* | `AsyncBulletinClient` | `client.store(data).send()` |
| `store_with_cid_config(cid, data)` | `TransactionClient`* | `AsyncBulletinClient` | `client.store(data).with_codec(...).send()` |
| `renew(block, index)` | `TransactionClient`* | `AsyncBulletinClient` | `client.renew(block, index)` |
| `check_proof(proof)` | N/A (validator inherent) | N/A | Not in SDK scope |
| `authorize_account(who, txns, bytes)` | `TransactionClient`* | `AsyncBulletinClient` | `client.authorize_account(...)` |
| `authorize_preimage(hash, max_size)` | `TransactionClient`* | `AsyncBulletinClient` | `client.authorize_preimage(...)` |
| `refresh_account_authorization(who)` | `TransactionClient`* | `AsyncBulletinClient` | `client.refresh_account_auth(...)` |
| `refresh_preimage_authorization(hash)` | `TransactionClient`* | `AsyncBulletinClient` | `client.refresh_preimage_auth(...)` |
| `remove_expired_account_authorization(who)` | `TransactionClient`* | `AsyncBulletinClient` | `client.remove_expired_account_auth(...)` |
| `remove_expired_preimage_authorization(hash)` | `TransactionClient`* | `AsyncBulletinClient` | `client.remove_expired_preimage_auth(...)` |

\* Rust `TransactionClient` implements all 9 extrinsics but cannot compile — requires `metadata.scale` file that is not in the repository. `AsyncBulletinClient` has stubs only.

### Storage Queries (6 total)

| Storage item | Current coverage | Target |
|---|---|---|
| `Authorizations(account)` | Missing | `client.query_authorization(account)` |
| `PreimageAuthorizations(hash)` | Missing | `client.query_preimage_authorization(hash)` |
| `TransactionRoots(block)` | Missing | `client.query_transaction_info(block)` |
| `RetentionPeriod` | Missing | `client.query_retention_period()` |
| `ByteFee` | Missing | `client.query_fees()` |
| `EntryFee` | Missing | `client.query_fees()` |

### Events (9 total)

| Event | Current coverage | Target |
|---|---|---|
| `Stored` | Missing | `client.on_stored(callback)` |
| `Renewed` | Missing | `client.on_renewed(callback)` |
| `ProofChecked` | N/A | Not in SDK scope |
| `AccountAuthorized` | Missing | `client.on_account_authorized(callback)` |
| `AccountAuthorizationRefreshed` | Missing | `client.on_account_auth_refreshed(callback)` |
| `PreimageAuthorized` | Missing | `client.on_preimage_authorized(callback)` |
| `PreimageAuthorizationRefreshed` | Missing | `client.on_preimage_auth_refreshed(callback)` |
| `ExpiredAccountAuthorizationRemoved` | Missing | `client.on_expired_account_auth_removed(callback)` |
| `ExpiredPreimageAuthorizationRemoved` | Missing | `client.on_expired_preimage_auth_removed(callback)` |

### Data Retrieval

| Operation | Current coverage | Target |
|---|---|---|
| Fetch raw data by block + index | Missing | `client.get_data(block, index)` |
| Fetch data via IPFS gateway | Missing | `client.get_data_by_cid(cid)` |
| Fetch and reassemble chunked data | Missing | `client.get_chunked_data(manifest_cid)` |
| Parse DAG-PB manifest | Partial (code exists, unwired) | `parse_manifest(dag_bytes)` in Layer 1 |
| Reassemble chunks | Partial (code exists, unwired) | `reassemble(chunks)` in Layer 1 |

## Environment Compatibility

The three-layer design preserves or improves compatibility across all target environments:

| Environment | Layer 1 | Layer 2 | Layer 3 |
|---|---|---|---|
| Rust native (server, CLI) | Native | Native | subxt |
| Rust `no_std` (embedded, runtime) | Native | Native | N/A |
| Node.js >= 18 | WASM | WASM + JS | PAPI |
| Modern browsers | WASM | WASM + JS | PAPI |
| Deno / Bun | WASM | WASM + JS | PAPI |
| Edge workers (Cloudflare, Vercel) | WASM | WASM + JS | PAPI |
| React Native | WASM | WASM + JS | PAPI |

WASM is universally supported across all JavaScript target environments. The current TypeScript SDK already loads WASM internally via `@polkadot/util-crypto`, so this introduces no new runtime requirements.

## Migration Path

### Phase 1: Fix critical issues in current SDK

1. **Resolve Rust client situation.** Decide between `TransactionClient` (metadata codegen approach) and `AsyncBulletinClient` (stub approach). Keep one, remove the other. If keeping `TransactionClient`, document how to generate `metadata.scale` from a running node and add it to CI.
2. **Fix signer type.** Replace `signer: &Keypair` with `signer: &dyn Signer<PolkadotConfig>` (or generic `S: Signer<PolkadotConfig>`) on all Rust methods. This immediately enables ed25519, ecdsa, and custom signers.
3. **Fix TypeScript type safety.** Replace `api: any` with `TypedApi<typeof bulletin>` in `AsyncBulletinClient`. Remove the unused `TypedApi` import or use it.
4. **Fix `Binary` inconsistency.** Ensure all TypeScript submission methods wrap data as `Binary(data)`, including `storeWithPreimageAuth`.
5. **Fix `waitForFinalization`.** Either implement the option (add `signAndSubmit` path for fire-and-forget) or remove it from `StoreOptions`.
6. **Define shared interface.** Create `BulletinApi` trait (Rust) and interface (TypeScript) that both real and mock clients implement. Update `MockBulletinClient` to cover all methods.
7. **Resolve `BulletinConfig` vs `PolkadotConfig`.** Either use `BulletinConfig` with the `ProvideCidConfig` signed extension everywhere, or remove `subxt_config.rs` if the custom config is not needed.

### Phase 2: Extract primitives, remove heavy deps

1. Create `bulletin-primitives` crate with CID types extracted from the pallet.
2. Make both the pallet and Rust SDK depend on `bulletin-primitives`.
3. Replace `sp-io::hashing::blake2_256` with the `blake2` crate.
4. Replace `sp-runtime::AccountId32` with `subxt::utils::AccountId32` (behind `std` feature).
5. Drop `sp-core`, `sp-runtime`, `sp-io` from SDK dependencies.

### Phase 3: WASM compilation of Layer 1

1. Add `wasm-bindgen` exports to Layer 1 functions.
2. Set up `wasm-pack build --target web` in CI.
3. Publish `@bulletin/sdk-core` to npm with the WASM blob.
4. Replace TypeScript reimplementations (`chunker.ts`, `dag.ts`, utils) with calls to WASM.
5. Remove `@ipld/dag-pb`, `ipfs-unixfs`, `multiformats` from TypeScript dependencies.

### Phase 4: Complete pallet coverage

1. Add read-only client constructor (no signer required).
2. Add state query methods (authorizations, fees, retention period, transaction roots).
3. Add event subscription methods.
4. Add data retrieval methods (block+index fetch, IPFS gateway fetch, chunked reassembly).

### Phase 5: Publish

1. Publish `bulletin-primitives` to crates.io.
2. Publish `bulletin-sdk` to crates.io (requires replacing workspace git deps with published versions).
3. Publish `@bulletin/sdk-core` to npm.
4. Publish `@bulletin/sdk` to npm.
