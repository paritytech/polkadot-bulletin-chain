# Rust Authorize and Store Example

> [!WARNING]
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../../../LICENSE-APACHE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain).

This example demonstrates using the `bulletin-sdk-rust` crate to interact with Bulletin Chain.

## Features

- Uses SDK's `TransactionClient` for all chain interactions
- Uses SDK's `BulletinClient` for CID calculation
- Chunked upload through the `estimate_upload` → `submit` pipeline
- Progress tracking via callbacks
- No manual metadata generation required

## Prerequisites

1. **Running Bulletin Chain node**: You need a running Bulletin Chain node with WebSocket endpoint available

   Example for local development (see the root README quickstart):
   ```bash
   # From project root
   just binaries-polkadot
   just chain-spec westend
   $(just binaries-polkadot)/polkadot-omni-node --chain ./zombienet/bulletin-westend-spec.json --dev --ipfs-server
   ```

   This typically runs on `ws://localhost:9944`.

## Usage

### Basic Usage

```bash
cargo run --release -- --ws <WS_URL> --seed "<SEED>"
```

Where:
- `<WS_URL>`: WebSocket URL of your Bulletin Chain node (default: `ws://localhost:10000`)
- `<SEED>`: Account seed phrase or dev seed like `//Eve` (default: `//Eve`). The example authorizes its own account, so the seed must belong to a registered authorizer.

### Example

```bash
# Using defaults (ws://localhost:10000, //Eve)
cargo run --release

# Custom endpoint
cargo run --release -- --ws ws://your-node:9944 --seed "<authorizer seed>"
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
INFO Progress: Transaction(Validated { chunk_index: None })
INFO Progress: Transaction(Broadcasted { chunk_index: None })
INFO Progress: Transaction(InBlock { block_hash: "0x...", block_number: Some(...), tx_index: None, chunk_index: None })
INFO
✅ Data stored successfully using Bulletin SDK!
INFO   Block hash: 0x5678...
INFO   Extrinsic hash: 0x9abc...
INFO   Data size: 42 bytes
INFO
--- Step 3: Chunked Storage with DAG-PB Manifest ---
INFO Large data size: 3145728 bytes (3 MiB)
INFO Planned 3 chunks + manifest: 4 transactions, ... bytes of authorization
INFO   item 1/4 in block: 0x...
INFO   item 2/4 in block: 0x...
INFO   item 3/4 in block: 0x...
INFO   item 4/4 in block: 0x...
INFO ✅ Chunked upload complete: 4 stores (chunks + manifest)
INFO   Manifest root CID: 0170...
INFO   Use this manifest CID to retrieve the complete file via IPFS/Bitswap
INFO
✅ All examples completed successfully!
```

## How it Works

1. **TransactionClient**: Connects to the chain and submits transactions
2. **BulletinClient**: Prepares data and calculates CIDs locally
3. **Upload pipeline**: `estimate_upload` plans the chunks and manifest; `submit` uploads them with nonce, retry, and reorg handling
4. **Progress Tracking**: Receives real-time updates as transactions progress

## SDK Integration

### Simple Storage

```rust
// TransactionClient for chain interaction
let client = TransactionClient::new("ws://localhost:10000").await?;

// BulletinClient for data preparation
let sdk_client = BulletinClient::new();
let operation = sdk_client.prepare_store(data.clone(), options)?;
let cid = operation.calculate_cid()?;

// Store with progress tracking
let receipt = client.store_with_progress(data, &signer, WaitFor::InBlock, Some(callback)).await?;
```

### Chunked Storage with DAG-PB Manifest

For large files, plan the upload with `estimate_upload` and submit it through the pipeline:

```rust
// Plan: chunk CIDs, DAG-PB manifest, and the authorization the upload needs
let source: Arc<dyn SeekableSource> = Arc::new(blob_from_bytes(large_data));
let options = UploadEstimateOptions {
    chunker: ChunkerConfig { chunk_size: 1024 * 1024, ..Default::default() },
    ..Default::default()
};
let estimate = client.estimate_upload(UploadInput::Source(source.clone()), options).await?;

// Submit: the pipeline owns nonce assignment, retries, and reorg recovery
let config = UploadConfig { complete_on: WaitFor::InBlock, ..Default::default() };
let result = client.submit(&signer, estimate, source, config).await?;

// CIDs in plan order; the manifest root CID is last
let root_cid = result.cids.last();
```

The manifest CID can be used to retrieve the complete file via IPFS/Bitswap.

## Security

See the [root README](../../../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
