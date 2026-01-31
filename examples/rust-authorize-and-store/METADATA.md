# Metadata Generation

This example uses subxt's metadata codegen to generate Rust types from a running Bulletin Chain node.

## For Local Development

1. Start a Bulletin Chain node:
   ```bash
   cargo build --release
   ./target/release/polkadot-bulletin-chain --dev --tmp
   ```

2. Generate metadata:
   ```bash
   cd examples/rust-authorize-and-store
   ./fetch_metadata.sh ws://localhost:10000
   ```

3. Build and run:
   ```bash
   cargo run --release
   ```

## For CI/CD

The `bulletin_metadata.scale` file should be committed to the repository to avoid requiring a running node during CI builds. When the runtime changes, regenerate and commit the updated metadata.

## Why Metadata Codegen?

Using `#[subxt::subxt]` with metadata codegen provides:
- **Type safety**: Generated types match the actual runtime
- **Automatic updates**: Regenerate when runtime changes
- **Less boilerplate**: No need to manually define all types
- **IDE support**: Full autocomplete and type checking

The only custom code needed is for Bulletin Chain's `ProvideCidConfig` signed extension, which is specific to the TransactionStorage pallet.
