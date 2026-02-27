# Rust Authorize and Store Example

This example demonstrates using the `bulletin-sdk-rust` crate to interact with Bulletin Chain.

## Features

- Uses SDK's `TransactionClient` for all chain interactions
- Uses SDK's `BulletinClient` for CID calculation
- Progress tracking via callbacks
- No manual metadata generation required

## Prerequisites

1. **Running Bulletin Chain node**: You need a running Bulletin Chain node with WebSocket endpoint available

   Example for local development:
   ```bash
   # From project root
   cargo build --release
   ./target/release/polkadot-bulletin-chain --dev --tmp
   ```

   This typically runs on `ws://localhost:10000`.

## Usage

### Basic Usage

```bash
cargo run --release -- --ws <WS_URL> --seed "<SEED>"
```

Where:
- `<WS_URL>`: WebSocket URL of your Bulletin Chain node (default: `ws://localhost:10000`)
- `<SEED>`: Account seed phrase or dev seed like `//Alice` (default: `//Alice`)

### Example

```bash
# Using defaults (localhost, Alice)
cargo run --release

# Custom endpoint
cargo run --release -- --ws ws://your-node:9944 --seed "//Bob"
```

### Controlling Log Output

Control the log level using the `RUST_LOG` environment variable:

```bash
# Default (INFO level)
cargo run --release

# Debug output
RUST_LOG=debug cargo run --release

# Only warnings and errors
RUST_LOG=warn cargo run --release
```

## Example Output

```
INFO Using account: 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY
INFO Connecting to ws://localhost:10000 using Bulletin SDK...
INFO Connected successfully!
INFO
Step 1: Authorizing account using SDK...
INFO Account authorized successfully!
INFO   Block hash: 0x1234...
INFO   Transactions: 100
INFO   Bytes: 104857600
INFO
Step 2: Storing data using SDK...
INFO Data: Hello from Bulletin SDK at 1234567890s
INFO Pre-calculated CID: 0155...
INFO Content hash: a4e9...
INFO Progress: TransactionValidated
INFO Progress: TransactionBroadcasted { num_peers: 0 }
INFO Progress: TransactionInBestBlock { ... }
INFO Progress: TransactionFinalized { ... }
INFO
✅ Data stored successfully using Bulletin SDK!
INFO   Block hash: 0x5678...
INFO   Extrinsic hash: 0x9abc...
INFO   Data size: 42 bytes
```

## How it Works

1. **TransactionClient**: Connects to the chain and submits transactions
2. **BulletinClient**: Prepares data and calculates CIDs locally
3. **Progress Tracking**: Receives real-time updates as transactions progress

## SDK Integration

The example uses two SDK clients:

```rust
// TransactionClient for chain interaction
let client = TransactionClient::new("ws://localhost:10000").await?;

// BulletinClient for data preparation
let sdk_client = BulletinClient::new();
let operation = sdk_client.prepare_store(data, options)?;
let cid = operation.calculate_cid()?;

// Store with progress tracking
let receipt = client.store_with_progress(data, &signer, Some(callback)).await?;
```
