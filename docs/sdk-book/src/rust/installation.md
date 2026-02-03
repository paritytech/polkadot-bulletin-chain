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

## Subxt Integration

The SDK is a *client* library; it prepares data and calculations. To actually submit transactions to the blockchain, you typically use `subxt`.

Add `subxt` to your dependencies:

```toml
[dependencies]
bulletin-sdk-rust = "0.1"
subxt = "0.37"
tokio = { version = "1", features = ["full"] }
```
