# Rust SDK

The `bulletin-sdk-rust` crate provides a robust client for interacting with the Bulletin Chain. It is designed to be:

- **Type-Safe**: Leverages Rust's type system to prevent common errors.
- **Flexible**: Works with `std` (standard library) and `no_std` (WASM/embedded) environments.
- **Modular**: Use only what you need (chunking, CID calculation, or full client).

## Key Features

- **Direct Transaction Submission**: `TransactionClient` handles all chain interactions out of the box
- **Offline Preparation**: `BulletinClient` prepares operations (CID calculation, chunking, DAG building) without network access
- **Builder Pattern**: Fluent API for configuring store operations
- **Runtime Metadata**: Embedded metadata for Bulletin Chain - works out of the box
- **Structured Errors**: Error codes, retryable detection, and recovery hints consistent with TypeScript SDK
- **Progress Tracking**: Callback-based progress events for uploads
- **no_std Compatible**: Core functionality works in no_std environments

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

The SDK's public API is exposed through the crate root and the `prelude` module. Internal modules are not directly accessible — use the prelude for convenient imports:

```rust
use bulletin_sdk_rust::prelude::*;
```

Key types available through the prelude:

- **Clients**: `TransactionClient`, `BulletinClient`
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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to Bulletin Chain
    let client = TransactionClient::new("ws://localhost:10000").await?;

    // Create signer (dev account for testing)
    let uri = subxt_signer::SecretUri::from_str("//Alice")?;
    let signer = Keypair::from_uri(&uri)?;
    let account = subxt::utils::AccountId32::from(signer.public_key().0);

    // Authorize account (requires sudo)
    client.authorize_account(account.clone(), 10, 10 * 1024 * 1024, &signer).await?;

    // Store data with progress tracking
    let data = b"Hello, Bulletin!".to_vec();
    let receipt = client.store_with_progress(
        data,
        &signer,
        Some(std::sync::Arc::new(|event| {
            println!("Progress: {:?}", event);
        })),
    ).await?;

    println!("Stored in block: {}", receipt.block_hash);
    Ok(())
}
```

### Using BulletinClient (Prepare Only)

For offline preparation or when you have your own subxt setup:

```rust
use bulletin_sdk_rust::prelude::*;

// BulletinClient doesn't need a network connection
let client = BulletinClient::new();
let data = b"Hello, Bulletin!".to_vec();
let options = StoreOptions::default();

// Prepare the operation (calculates CID, no network calls)
let operation = client.prepare_store(data, options)?;
println!("CID: {:?}", operation.cid_bytes);

// Then submit via your own subxt client...
```

### Production Signer Setup

For production, use a seed phrase or private key:

```rust
use subxt_signer::sr25519::Keypair;

// From mnemonic seed phrase
let signer = Keypair::from_phrase(
    "bottom drive obey lake curtain smoke basket hold race lonely fit walk",
    None, // password
).expect("Invalid seed phrase");

// From secret URI (like //Alice for dev)
let signer = Keypair::from_uri("//Alice")
    .expect("Invalid URI");
```

Proceed to [Installation](./installation.md) to get started.
