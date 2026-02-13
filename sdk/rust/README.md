# Bulletin SDK for Rust

Off-chain client SDK for Polkadot Bulletin Chain. Supports `std` and `no_std` environments.

## Installation

```toml
[dependencies]
bulletin-sdk-rust = { workspace = true }

# For no_std:
# bulletin-sdk-rust = { workspace = true, default-features = false }
```

For transaction submission, also add `subxt`:

```toml
subxt = "0.44"
```

## Build & Test

```bash
cargo build --release --all-features
cargo test --lib --all-features
```

## Documentation

See the [SDK book](../../docs/sdk-book/) for usage guides, examples, and API reference.

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
