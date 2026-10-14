# Rust SDK

> [!WARNING]
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../../../../LICENSE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain).

The `bulletin-sdk-rust` crate provides a robust client for interacting with the Bulletin Chain. It is designed to be:

- **Type-Safe**: Leverages Rust's type system to prevent common errors.
- **Flexible**: Works with `std` (standard library) and `no_std` (WASM/embedded) environments.
- **Modular**: Use only what you need (chunking, CID calculation, or full client).

## Key Features

- **Exactly-once upload pipeline**: `estimate_upload` → `submit` drives items to finality with wave batching, content-hash dedup, hijack recovery, watchdogs, and re-subscribe/retry-resume — a resumed run never double-pays; set `skip_existing: true` in the estimate options to skip content already on chain
- **Streaming**: upload a file from a `SeekableSource`, range-read lazily (resident memory tracks the in-flight window, not the file)
- **Provider-agnostic**: `from_rpc_clients` takes any subxt `RpcClient` — WS node or smoldot light client — and fans broadcast out across all of them
- **Unsigned (preimage) path**: `submit_unsigned` for preimage-authorized bare extrinsics
- **Offline preparation**: `BulletinClient` prepares operations (CID, chunking, DAG building) without network access
- **Runtime metadata**: embedded — works out of the box
- **Structured errors**: error codes, retryable detection, and recovery hints consistent with the TypeScript SDK
- **no_std compatible**: core (CID/chunking/DAG) works in `no_std`

## Architecture

The SDK provides two approaches:

### Simple: TransactionClient (Recommended)

For most use cases, `TransactionClient` handles everything:

```
┌─────────────────────────────────────────┐
│            Your Application              │
├─────────────────────────────────────────┤
│         TransactionClient               │
│    (connects, submits, tracks progress) │
└────────────────┬────────────────────────┘
                 │
                 ▼
        ┌────────────────────┐
        │  Bulletin Chain    │
        │   (WebSocket)      │
        └────────────────────┘
```

### Advanced: Prepare and Submit Separately

For advanced use cases (custom submission, light clients, batching), prepare operations with `BulletinClient` and submit via your own subxt client:

```
┌──────────────────────────────────────────────────┐
│                  Your Application                  │
├──────────────────────────────────────────────────┤
│  ┌──────────────────┐   ┌──────────────────────┐ │
│  │  BulletinClient   │   │  Your subxt code     │ │
│  │  (prepare only)   │   │  (submit + query)    │ │
│  └────────┬──────────┘   └──────────┬───────────┘ │
│           │ operations              │              │
│           └──────────┬──────────────┘              │
│                      ▼                             │
│           ┌──────────────────┐                     │
│           │  subxt client    │                     │
│           └────────┬─────────┘                     │
└────────────────────┼───────────────────────────────┘
                     ▼
        ┌────────────────────────┐
        │   RPC / Light Client   │
        └────────────────────────┘
```

## Public API

The SDK's public API is exposed through the crate root, the `prelude` module, and the `compat` module (renewal call shapes). Other modules are private — use the prelude for convenient imports:

```rust
use bulletin_sdk_rust::prelude::*;
```

Key types available through the prelude:

- **Clients**: `TransactionClient`, `BulletinClient`
- **Uploads** (std only): `UploadInput`, `UploadItem`, `UploadEstimateOptions`, `StreamEstimate`, `UploadConfig`, `UploadEvent`, `UploadResult`, `SeekableSource`, `blob_from_bytes`, `blob_from_items`
- **Chunking**: `FixedSizeChunker`, `Chunker` trait
- **CID**: `calculate_cid`, `CidCodec`, `HashingAlgorithm`, `CidConfig`
- **DAG**: `UnixFsDagBuilder`, `DagManifest`, `DagBuilder` trait
- **Storage**: `StorageOperation`, `BatchStorageOperation`
- **Authorization**: `Authorization`, `AuthorizationManager`
- **Renewal**: `RenewalTracker`, `RenewalOperation`
- **Types**: `StoreResult`, `StoreOptions`, `ChunkerConfig`, `Error`, `ProgressEvent`, etc.
- **Transaction Receipts** (std only): `StoreReceipt`, `AuthorizationReceipt`, `RenewReceipt`

## Quick Start

> **Complete Working Examples**: See [`examples/rust/authorize-and-store`](https://github.com/paritytech/polkadot-bulletin-chain/tree/main/examples/rust/authorize-and-store) for runnable examples demonstrating authorization, storage, and chunked uploads with DAG-PB manifests.

### Using TransactionClient (Recommended)

The simplest way to interact with Bulletin Chain:

```rust
use bulletin_sdk_rust::prelude::*;
use subxt_signer::sr25519::Keypair;
use std::str::FromStr;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Connect to Bulletin Chain
    let client = TransactionClient::new("ws://localhost:10000").await?;

    // Dev accounts: //Alice uploads, //Eve is a genesis authorizer in the dev preset.
    let signer = Keypair::from_uri(&subxt_signer::SecretUri::from_str("//Alice")?)?;
    let authorizer = Keypair::from_uri(&subxt_signer::SecretUri::from_str("//Eve")?)?;
    let account = subxt::utils::AccountId32::from(signer.public_key().0);

    // Authorize the uploader. The signer of this call must be a registered authorizer.
    client.authorize_account(account, 10, 10 * 1024 * 1024, &authorizer, WaitFor::Finalized).await?;

    // Upload via the `estimate_upload` -> `submit` pipeline (exactly-once).
    let items = vec![UploadItem::new(b"Hello, Bulletin!".to_vec())];
    let datas: Vec<Vec<u8>> = items.iter().map(|i| i.data.clone()).collect();
    let estimate = client
        .estimate_upload(UploadInput::Items(items), UploadEstimateOptions::default())
        .await?;
    let source: std::sync::Arc<dyn SeekableSource> = std::sync::Arc::new(blob_from_items(datas));
    let result = client.submit(&signer, estimate, source, UploadConfig::default()).await?;

    println!("Stored CIDs: {:?}", result.cids);
    Ok(())
}
```

For a large file, pass `UploadInput::Source(source)` (e.g. `blob_from_bytes(..)`)
to `estimate_upload` and the same `source` to `submit` — it is chunked (with a
DAG-PB manifest when it spans more than one chunk) and range-read lazily during upload. Use `submit_unsigned` for the
preimage-authorized path, and `from_endpoints(&[..])` for multi-provider
broadcast.

### Using BulletinClient (Prepare Only)

For offline preparation or when you have your own subxt setup:

```rust
use bulletin_sdk_rust::prelude::*;

// BulletinClient doesn't need a network connection
let client = BulletinClient::new();
let data = b"Hello, Bulletin!".to_vec();
let options = StoreOptions::default();

// Validate the operation (no network calls), then compute its CID
let operation = client.prepare_store(data, options)?;
let cid = cid_to_bytes(&operation.calculate_cid()?)?;
println!("CID: {:?}", cid);

// Then submit via your own subxt client...
```

### Production Signer Setup

For production, use a seed phrase or private key:

```rust
use std::str::FromStr;
use subxt_signer::{bip39::Mnemonic, sr25519::Keypair, SecretUri};

// From mnemonic seed phrase
let phrase = Mnemonic::from_str("bottom drive obey lake curtain smoke basket hold race lonely fit walk")?;
let signer = Keypair::from_phrase(&phrase, None)?; // None = no password

// From secret URI (like //Alice for dev)
let signer = Keypair::from_uri(&SecretUri::from_str("//Alice")?)?;
```

Proceed to [Installation](./installation.md) to get started.

## Security

See the [root README](../../../../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
