# Rust Examples

Rust examples for interacting with Polkadot Bulletin Chain using [subxt](https://github.com/paritytech/subxt).

## Examples

### authorize-and-store

Demonstrates using subxt to interact with Bulletin Chain:
- Auto-discovery of signed extensions from metadata
- Account authorization workflow
- Data storage with proper transaction handling
- Event parsing and CID retrieval

**Key Features:**
- Uses `PolkadotConfig` for automatic signed extension discovery
- Handles Bulletin's custom `ProvideCidConfig` extension automatically
- Clean error handling with `anyhow`
- Structured logging with `tracing`

**Files:**
- `src/main.rs` - Main example code
- `Cargo.toml` - Dependencies
- `fetch_metadata.sh` - Script to fetch chain metadata
- `README.md` - Example-specific documentation

## Setup

### 1. Generate Metadata

Subxt generates types from chain metadata. First, fetch the metadata from a running node:

```bash
cd authorize-and-store
./fetch_metadata.sh ws://localhost:10000
```

This creates `bulletin_metadata.scale` which subxt uses for type generation at compile time.

### 2. Build

```bash
cargo build --release
```

## Usage

### authorize-and-store

```bash
# Basic usage (defaults: ws://localhost:10000, //Alice)
./target/release/authorize-and-store

# Custom WebSocket URL
./target/release/authorize-and-store --ws ws://localhost:9944

# Custom seed (must have sudo for authorization)
./target/release/authorize-and-store --seed "//Bob"

# Full example
./target/release/authorize-and-store \
  --ws ws://localhost:10000 \
  --seed "//Alice"
```

**Command-line options:**
- `--ws <URL>` - WebSocket URL of the Bulletin Chain node
- `--seed <SEED>` - Seed phrase or dev seed (e.g., `//Alice`)

## Requirements

- Rust 1.75+
- Running Bulletin Chain node
- Seed account with sudo privileges (for authorization)

## Key Concepts

### Metadata Auto-Discovery

Instead of manually defining all signed extensions, this example uses subxt's automatic discovery:

```rust
// Subxt generates types from metadata at compile time
#[subxt::subxt(runtime_metadata_path = "bulletin_metadata.scale")]
pub mod bulletin {}

// Use PolkadotConfig which auto-discovers extensions
let api = OnlineClient::<PolkadotConfig>::from_url(&ws_url).await?;
```

This automatically handles:
- `CheckNonce`
- `CheckTxVersion`
- `CheckGenesis`
- `CheckMortality`
- `ChargeAssetTxPayment`
- `ChargeTransactionPayment`
- `CheckMetadataHash`
- `CheckSpecVersion`
- `ProvideCidConfig` (Bulletin's custom extension)

### Transaction Building

```rust
// Build transaction using generated types
let store_tx = bulletin::tx()
    .transaction_storage()
    .store(data.as_bytes().to_vec());

// Sign and submit
let result = api.tx()
    .sign_and_submit_then_watch(&store_tx, &keypair, bulletin_params())
    .await?
    .wait_for_finalized_success()
    .await?;
```

### Event Parsing

```rust
// Find specific events
let stored_event = result
    .find_first::<bulletin::transaction_storage::events::Stored>()?;

if let Some(event) = stored_event {
    println!("CID: {}", hex::encode(&event.content_hash));
    println!("Size: {} bytes", event.size);
}
```

## Updating Metadata

When the chain runtime changes, regenerate the metadata:

```bash
cd authorize-and-store
./fetch_metadata.sh ws://localhost:10000
cargo build --release
```

## Learn More

- **Subxt Documentation**: https://docs.rs/subxt
- **Polkadot Config**: https://docs.rs/subxt/latest/subxt/config/struct.PolkadotConfig.html
- **Bulletin SDK**: See `../../sdk/rust/` for higher-level abstractions
