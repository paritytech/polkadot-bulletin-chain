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

The SDK provides two approaches for transaction submission:

### Option 1: TransactionClient (Recommended)

`TransactionClient` handles all chain interactions out of the box. The SDK includes embedded chain metadata, so no additional setup is required:

```toml
[dependencies]
bulletin-sdk-rust = "0.1"
subxt-signer = { version = "0.44", features = ["sr25519"] }
tokio = { version = "1", features = ["full"] }
```

```rust
use bulletin_sdk_rust::prelude::*;

let client = TransactionClient::new("ws://localhost:10000").await?;
let receipt = client.store(data, &signer).await?;
```

### Option 2: AsyncBulletinClient (BYOC)

For advanced use cases like connection reuse or light client integration, use `AsyncBulletinClient` with your own subxt client:

```toml
[dependencies]
bulletin-sdk-rust = "0.1"
subxt = "0.44"
subxt-signer = { version = "0.44", features = ["sr25519"] }
tokio = { version = "1", features = ["full"] }
```
