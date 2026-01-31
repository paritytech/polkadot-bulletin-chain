# Rust Authorize and Store Example

This example demonstrates using the `bulletin-sdk-rust` with a subxt-based TransactionSubmitter.

## Prerequisites

1. **Running Bulletin Chain node**:
   ```bash
   # From project root
   cargo build --release
   ./target/release/polkadot-bulletin-chain --dev --tmp
   ```

2. **Generate metadata** (required before first build):
   ```bash
   cd examples/rust-authorize-and-store
   ./fetch_metadata.sh
   ```

   Or manually:
   ```bash
   # Install subxt CLI if not already installed
   cargo install subxt-cli

   # Fetch metadata from running node
   subxt metadata --url ws://localhost:10000 -f bytes > bulletin_metadata.scale
   ```

## Usage

```bash
cargo run --release -- --ws ws://localhost:10000 --seed "//Alice"
```

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
