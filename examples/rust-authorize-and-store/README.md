# Rust Authorize and Store Example

This example demonstrates using the `bulletin-sdk-rust` with a subxt-based TransactionSubmitter.

**⚠️  Status**: This example is currently incomplete and requires metadata generation from a running node before it can be compiled and tested. It has been updated to use subxt 0.37 metadata codegen but the API compatibility still needs verification with actual runtime metadata.

**Note**: This example is excluded from the main workspace due to dependency conflicts between `subxt 0.37` and the polkadot-sdk dependencies. Build it separately from its directory.

## Prerequisites

1. **Running Bulletin Chain node**: You need a running Bulletin Chain node with WebSocket endpoint available

   Example for local development:
   ```bash
   # From project root
   cargo build --release
   ./target/release/polkadot-bulletin-chain --dev --tmp
   ```

   This typically runs on `ws://localhost:10000`, but your setup may differ.

2. **Generate metadata** (required before first build):
   ```bash
   cd examples/rust-authorize-and-store
   ./fetch_metadata.sh <WS_URL>
   ```

   Where `<WS_URL>` is your node's WebSocket endpoint (e.g., `ws://localhost:10000` or `ws://your-node:9944`).

   Or manually:
   ```bash
   # Install subxt CLI if not already installed
   cargo install subxt-cli

   # Fetch metadata from your running node
   subxt metadata --url <WS_URL> -f bytes > bulletin_metadata.scale
   ```

## Usage

```bash
cargo run --release -- --ws <WS_URL> --seed "<SEED>"
```

Where:
- `<WS_URL>`: WebSocket URL of your Bulletin Chain node (default: `ws://localhost:10000`)
- `<SEED>`: Account seed phrase or dev seed like `//Alice` (default: `//Alice`)

## How it Works

1. **Metadata Codegen**: The `#[subxt::subxt]` macro generates Rust types from `bulletin_metadata.scale` at compile time
2. **Custom Extension**: We handle Bulletin Chain's custom `ProvideCidConfig` signed extension for CID configuration
3. **SDK Integration**: The generated types are used with `bulletin-sdk-rust`'s `TransactionSubmitter` trait

## Updating Metadata

When the Bulletin Chain runtime changes, regenerate the metadata:

```bash
./fetch_metadata.sh ws://localhost:10000
```

Then rebuild:
```bash
cargo clean
cargo build --release
```
