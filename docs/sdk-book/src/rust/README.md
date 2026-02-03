# Rust SDK

The `bulletin-sdk-rust` crate provides a robust client for interacting with the Bulletin Chain. It is designed to be:

- **Type-Safe**: Leverages Rust's type system to prevent common errors.
- **Flexible**: Works with `std` (standard library) and `no_std` (WASM/embedded) environments.
- **Modular**: Use only what you need (chunking, CID calculation, or full client).

## Key Features

- **Complete Transaction Support**: Built-in submitters for `subxt` and mock testing
- **Flexible Architecture**: Use `AsyncBulletinClient` for full automation or prepare operations manually
- **Multiple Submitter Options**: SubxtSubmitter, MockSubmitter, or create your own
- **Connection Management**: Simple `from_url()` constructor handles WebSocket connections
- **Testing Support**: MockSubmitter allows testing without a blockchain node

## Modules

- `async_client`: High-level async client with transaction submission (`AsyncBulletinClient`)
- `client`: Core client for operation preparation (`BulletinClient`)
- `submitters`: Transaction submitter implementations (SubxtSubmitter, MockSubmitter)
- `chunker`: Splits data into chunks (`FixedSizeChunker`)
- `cid`: CID calculation utilities
- `storage`: Transaction preparation helpers
- `authorization`: Authorization management
- `submit`: TransactionSubmitter trait definition

## Quick Start

```rust
use bulletin_sdk_rust::prelude::*;
use subxt_signer::sr25519::dev;

let ws_url = std::env::var("BULLETIN_WS_URL")
    .unwrap_or_else(|_| "ws://localhost:10000".to_string());

// Initialize signer from dev account (for testing)
// In production, use: Keypair::from_phrase() with your seed phrase
let signer = dev::alice();

// Connect and create client
// Note: The submitter contains the signer for transaction signing
let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
let client = AsyncBulletinClient::new(submitter);

// Store data - complete workflow with builder pattern
let result = client
    .store(data)
    .send()
    .await?;
```

### Using Multiple Accounts

If you need to use different accounts, create separate clients or submitters:

```rust
use subxt_signer::sr25519::dev;

// Client for Alice (e.g., for sudo operations)
let alice = dev::alice();
let alice_submitter = SubxtSubmitter::from_url(&ws_url, alice).await?;
let alice_client = AsyncBulletinClient::new(alice_submitter);

// Client for Bob (e.g., for regular storage)
let bob = dev::bob();
let bob_submitter = SubxtSubmitter::from_url(&ws_url, bob).await?;
let bob_client = AsyncBulletinClient::new(bob_submitter);

// Use alice_client for authorization
alice_client.authorize_account(bob.public_key().into(), 100, 10_000_000).await?;

// Use bob_client for storing data
let result = bob_client.store(data).send().await?;
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
