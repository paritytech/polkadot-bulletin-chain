# Bulletin SDK for Rust

Off-chain client SDK for Polkadot Bulletin Chain. Connects, uploads, and tracks
storage to finality through a wave-batched, reconcile-driven pipeline with
exactly-once guarantees — plus CID calculation, chunking, DAG-PB manifests, and
authorization management.

## Quick Start

```rust
use bulletin_sdk_rust::prelude::*;
use subxt_signer::sr25519::dev;
use std::sync::Arc;

# async fn run() -> Result<()> {
// Connect (use `from_endpoints` for multi-provider broadcast redundancy).
let client = TransactionClient::new("ws://localhost:10000").await?;
let signer = dev::alice();

// Upload in-memory items: plan, then submit. Items already on chain (by
// content hash) are skipped, so a re-run never pays twice for the same CID.
let items = vec![UploadItem::new(b"hello".to_vec()), UploadItem::new(b"world".to_vec())];
let datas: Vec<Vec<u8>> = items.iter().map(|i| i.data.clone()).collect();

let estimate = client
    .estimate_upload(UploadInput::Items(items), UploadEstimateOptions::default())
    .await?;

let source: Arc<dyn SeekableSource> = Arc::new(blob_from_items(datas));
let result = client.submit(&signer, estimate, source, UploadConfig::default()).await?;
println!("stored CIDs: {:?}", result.cids);
# Ok(()) }
```

## Uploading

Uploads go through one primitive: **`estimate_upload` → `submit`**.

`estimate_upload(input, ..)` plans the upload (per-unit CIDs, sizes, offsets, and
— for a file — a DAG-PB manifest) and sizes the authorization, skipping units
already on chain. `input` is either:

- `UploadInput::Items(items)` — each item stored as its own unit (no manifest).
- `UploadInput::Source(source)` — a re-openable byte source chunked into a file
  with a DAG-PB manifest. The source is streamed once in `O(chunk_size)` memory.

`submit(signer, estimate, source, config)` then drives the items to the requested
confirmation level. Bytes are fetched **lazily** from the source on each
(re-)broadcast and freed on finalization, so resident memory tracks the in-flight
window, not the whole upload. Pass the matching source — `blob_from_items(..)`
for items, `blob_from_bytes(..)` / `blob_from_factory(..)` for a file.

### Streaming a large file

```rust
# use bulletin_sdk_rust::prelude::*;
# use std::sync::Arc;
# async fn run(client: bulletin_sdk_rust::TransactionClient, signer: subxt_signer::sr25519::Keypair, file: Vec<u8>) -> Result<()> {
let source: Arc<dyn SeekableSource> = Arc::new(blob_from_bytes(file));
let estimate = client
    .estimate_upload(UploadInput::Source(source.clone()), UploadEstimateOptions::default())
    .await?;
// `estimate.base.transactions` / `.bytes` size the authorization to request.
let result = client.submit(&signer, estimate, source, UploadConfig::default()).await?;
// `result.cids` ends with the manifest root CID (the file's retrieval id).
# Ok(()) }
```

### Unsigned (preimage-authorized)

`submit_unsigned(estimate, source, config)` broadcasts bare extrinsics — no
signer, no nonce. Each content hash must be authorized with
`authorize_preimage` first.

## Guarantees

The pipeline mirrors the TypeScript SDK and provides the same guarantees:

- **Wave batching** to the chain's per-block capacity (`BlockLimits`).
- **Pool-aware nonce floor** + **finalized mortality anchor** (survives tip reorgs).
- **Reconcile-driven inclusion** via the on-chain `TransactionByContentHash` map.
- **Exactly-once** — content-hash dedup, one nonce slot per item; re-runs never double-pay.
- **Hijack recovery** — a slot taken by a concurrent same-signer tx is re-queued.
- **Watchdogs + retry-resume** — a stall/disconnect triggers a re-subscribe and
  re-broadcast of not-yet-confirmed items at their carried nonce (exactly-once);
  only after the retry budget does it surface `Error::StoreStalled`.
- **Multi-provider broadcast** — fan-out to every provider (accepted-if-any).

## Authorization

```rust
# use bulletin_sdk_rust::prelude::*;
# use subxt::utils::AccountId32;
# async fn run(client: bulletin_sdk_rust::TransactionClient, who: AccountId32, authorizer: subxt_signer::sr25519::Keypair) -> Result<()> {
// Single account.
client.authorize_account(who.clone(), 100, 64 * 1024 * 1024, &authorizer, WaitFor::Finalized).await?;

// Many accounts atomically (Utility.batch_all); `sudo: true` wraps in Sudo.sudo.
let entries = vec![AuthorizeAccountEntry { who, transactions: 10, bytes: 1 << 20 }];
client.authorize_accounts(entries, false, &authorizer, WaitFor::Finalized).await?;
# Ok(()) }
```

`authorize_preimage`, `renew`, `refresh_*`, and `remove_expired_*` round out the
authorization API.

## Connecting

The SDK is **provider-agnostic** — it takes subxt `RpcClient`s and doesn't care
whether each is a node WS connection or a light client.

- `TransactionClient::new(endpoint)` — single WS endpoint.
- `TransactionClient::from_endpoints(&[a, b, ..])` — multiple WS endpoints.
- `TransactionClient::from_rpc_clients(vec![..])` — the **provider** abstraction
  (the PAPI-`providers` analog): pass pre-built `RpcClient`s. The first is the
  monitor (reconcile + nonce/storage reads); every one is a broadcast target, so
  one dead provider can't stall a run.

### Light client (smoldot)

A light client is just a provider. Enable `subxt`'s `unstable-light-client`
feature in your crate, build the provider, and pass it in (Bulletin is a
parachain, so smoldot needs both the relay and Bulletin chain specs):

```rust,ignore
use subxt::lightclient::LightClient;
let (lc, _relay) = LightClient::relay_chain(RELAY_CHAIN_SPEC)?;
let bulletin: subxt::backend::rpc::RpcClient = lc.parachain(BULLETIN_SPEC)?.into();
let client = TransactionClient::from_rpc_clients(vec![bulletin]).await?;
// keep `lc` alive for the lifetime of `client`
```

## Offline preparation (`BulletinClient`)

For custom submission, light clients, or batching, `BulletinClient` prepares
operations (CID calculation, chunking, DAG building) without any network access;
submit them via your own subxt client.

```rust
# use bulletin_sdk_rust::prelude::*;
# fn run(data: Vec<u8>) -> Result<()> {
let client = BulletinClient::new();
let operation = client.prepare_store(data, StoreOptions::default())?;
println!("CID: {:?}", operation.cid_bytes);
# Ok(()) }
```

## Installation

```toml
[dependencies]
bulletin-sdk-rust = { path = "sdk/rust" }
```

For `no_std` (core CID/chunking/DAG only, no client):

```toml
[dependencies]
bulletin-sdk-rust = { path = "sdk/rust", default-features = false }
```

## Build & Test

```bash
cargo build --release
cargo test                 # unit tests
# Live integration tests need a node at ws://localhost:10000:
cargo test --test pipeline_live -- --ignored --nocapture
```

## Data Retrieval

This SDK does **not** provide retrieval. Public IPFS gateways are discouraged
(centralized). Decentralized retrieval via the smoldot light client's
`bitswap_block` RPC is planned — see
<https://github.com/paritytech/polkadot-bulletin-chain/pull/264>.

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
