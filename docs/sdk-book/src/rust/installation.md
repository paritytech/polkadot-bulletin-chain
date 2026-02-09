# Installation

Add the SDK to your `Cargo.toml`:

```toml
[dependencies]
bulletin-sdk-rust = "0.1"
```

## Feature Flags

The SDK exposes several feature flags to optimize your build:

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | Yes     | Enables standard library support. Disable for `no_std`. |
| `serde-support` | No | Adds `Serialize`/`Deserialize` support for types. |

## Transaction Submission

The SDK provides data preparation and CID calculation utilities. To submit transactions to the blockchain, you'll use `subxt` for blockchain interaction.

Add `subxt` to your dependencies:

```toml
[dependencies]
bulletin-sdk-rust = "0.1"
subxt = "0.37"
tokio = { version = "1", features = ["full"] }
```
