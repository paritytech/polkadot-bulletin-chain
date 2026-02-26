# Rust SDK

The `bulletin-sdk-rust` crate provides a robust client for interacting with the Bulletin Chain. It is designed to be:

- **Type-Safe**: Leverages Rust's type system to prevent common errors.
- **Flexible**: Works with `std` (standard library) and `no_std` (WASM/embedded) environments.
- **Modular**: Use only what you need (chunking, CID calculation, or full client).

## Key Features

- **Bring Your Own Client (BYOC)**: Accept an existing `subxt` client - enables light clients, connection reuse
- **Flexible Architecture**: Use `AsyncBulletinClient` for full automation or `BulletinClient` for manual preparation
- **Builder Pattern**: Fluent API for configuring store operations
- **Mock Testing**: `MockBulletinClient` allows testing without a blockchain node
- **Runtime Metadata**: Users configure subxt with their own metadata for maximum flexibility

## Architecture

The SDK follows a **Bring Your Own Client** pattern:

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

**Benefits:**
- **Connection reuse** - Share one client across SDK and other code
- **Light client support** - Use smoldot instead of RPC
- **Custom transports** - HTTP, WebSocket, or custom providers
- **No hidden connections** - You control all network access

## Modules

- `async_client`: High-level async client with transaction submission (`AsyncBulletinClient`)
- `mock_client`: Mock client for testing without blockchain (`MockBulletinClient`)
- `client`: Core client for operation preparation (`BulletinClient`)
- `chunker`: Splits data into chunks (`FixedSizeChunker`)
- `cid`: CID calculation utilities
- `storage`: Transaction preparation helpers
- `authorization`: Authorization management

## Quick Start

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

### Connection Reuse

The SDK accepts an existing subxt client, so you can share one connection:

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

### Light Client Support (smoldot)

The SDK accepts any subxt `OnlineClient`, including those backed by smoldot:

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
