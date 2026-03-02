# Rust SDK

The `bulletin-sdk-rust` crate provides a robust client for interacting with the Bulletin Chain. It is designed to be:

- **Type-Safe**: Leverages Rust's type system to prevent common errors.
- **Flexible**: Works with `std` (standard library) and `no_std` (WASM/embedded) environments.
- **Modular**: Use only what you need (chunking, CID calculation, or full client).

## Key Features

- **Direct Transaction Submission**: `TransactionClient` handles all chain interactions out of the box
- **Bring Your Own Client (BYOC)**: `AsyncBulletinClient` accepts an existing `subxt` client for connection reuse
- **Flexible Architecture**: Use `TransactionClient` for simplicity, `AsyncBulletinClient` for advanced use cases, or `BulletinClient` for manual preparation
- **Builder Pattern**: Fluent API for configuring store operations
- **Mock Testing**: `MockBulletinClient` allows testing without a blockchain node
- **Runtime Metadata**: Embedded metadata for Bulletin Chain - works out of the box

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

### Advanced: Bring Your Own Client (BYOC)

For advanced use cases (connection reuse, light clients), use `AsyncBulletinClient`:

```
┌─────────────────────────────────────────────────────────┐
│                    Your Application                      │
├─────────────────────────────────────────────────────────┤
│  ┌─────────────────┐    ┌─────────────────────────────┐ │
│  │  Bulletin SDK   │    │     Your Other Code         │ │
│  │                 │    │                             │ │
│  │ AsyncBulletinClient  │ queries, subscriptions, etc │ │
│  └────────┬────────┘    └──────────────┬──────────────┘ │
│           │                            │                │
│           └──────────┬─────────────────┘                │
│                      ▼                                  │
│           ┌──────────────────┐                          │
│           │  Shared subxt    │  ◄── You create this    │
│           │  OnlineClient    │                          │
│           └────────┬─────────┘                          │
│                    │                                    │
└────────────────────┼────────────────────────────────────┘
                     ▼
        ┌────────────────────────┐
        │   RPC / Light Client   │
        │   (your choice!)       │
        └────────────────────────┘
```

**Benefits of BYOC:**
- **Connection reuse** - Share one client across SDK and other code
- **Light client support** - Use smoldot instead of RPC
- **Custom transports** - HTTP, WebSocket, or custom providers
- **No hidden connections** - You control all network access

## Modules

- `transaction`: Direct transaction submission with progress tracking (`TransactionClient`) - **recommended for most use cases**
- `async_client`: High-level async client with BYOC pattern (`AsyncBulletinClient`)
- `mock_client`: Mock client for testing without blockchain (`MockBulletinClient`)
- `client`: Core client for operation preparation (`BulletinClient`)
- `chunker`: Splits data into chunks (`FixedSizeChunker`)
- `cid`: CID calculation utilities
- `storage`: Transaction preparation helpers
- `authorization`: Authorization management

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

### Using AsyncBulletinClient (Advanced)

For connection reuse or light client integration:

```rust
use bulletin_sdk_rust::prelude::*;
use subxt::{OnlineClient, PolkadotConfig};
use subxt_signer::sr25519::dev;

let ws_url = std::env::var("BULLETIN_WS_URL")
    .unwrap_or_else(|_| "ws://localhost:10000".to_string());

// Initialize signer from dev account (for testing)
// In production, use: Keypair::from_phrase() with your seed phrase
let signer = dev::alice();

// Connect to the blockchain using subxt
// Users must configure subxt with their own runtime metadata
let api = OnlineClient::<PolkadotConfig>::from_url(&ws_url).await?;

// Create SDK client with subxt client
let client = AsyncBulletinClient::new(api);

// Store data using builder pattern
let result = client
    .store(data)
    .send()
    .await?;
```

### Connection Reuse (AsyncBulletinClient)

When using `AsyncBulletinClient`, the SDK accepts an existing subxt client, so you can share one connection:

```rust
use bulletin_sdk_rust::prelude::*;
use subxt::{OnlineClient, PolkadotConfig};

// Create ONE shared subxt client for your whole app
let api = OnlineClient::<PolkadotConfig>::from_url("ws://localhost:9944").await?;

// SDK uses the shared client
let client = AsyncBulletinClient::new(api.clone());

// Your other code also uses the same client
let block_number = api.blocks().at_latest().await?.number();
let events = api.events().at_latest().await?;

// Both SDK and your queries share one WebSocket connection!
let result = client.store(data).send().await?;
```

### Light Client Support (smoldot) - AsyncBulletinClient

When using `AsyncBulletinClient`, the SDK accepts any subxt `OnlineClient`, including those backed by smoldot:

```rust
use subxt::{OnlineClient, PolkadotConfig};
use subxt::lightclient::{LightClient, ChainConfig};
use bulletin_sdk_rust::prelude::*;

// 1. Create light client with chain spec
let chain_spec = std::fs::read_to_string("bulletin-chain-spec.json")?;
let (lightclient, rpc) = LightClient::relay_chain(ChainConfig {
    chain_spec: &chain_spec,
    ..Default::default()
})?;

// 2. Create subxt client from light client RPC
let api = OnlineClient::<PolkadotConfig>::from_rpc_client(rpc).await?;

// 3. SDK works exactly the same - it doesn't know about the transport!
let client = AsyncBulletinClient::new(api);
let result = client.store(data).send().await?;
```

**Benefits of light clients:**
- No trusted RPC endpoint required
- Verifies chain state cryptographically
- Works in browser via smoldot WASM
- Better for user privacy

### Using Multiple Accounts

If you need to use different accounts, you need to handle signing at the transaction level.
The SDK client uses subxt directly, so you control the signer when creating transactions.

For testing without a blockchain, use the `MockBulletinClient`:

```rust
use bulletin_sdk_rust::prelude::*;

// Create mock client (no blockchain required)
let client = MockBulletinClient::new();

// Store data - calculates real CIDs but doesn't submit to chain
let result = client.store(data).send().await?;

// Verify operations performed
let ops = client.operations();
assert_eq!(ops.len(), 1);
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

let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
let client = AsyncBulletinClient::new(submitter);
```

Proceed to [Installation](./installation.md) to get started.
