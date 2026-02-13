# Rust SDK

The `bulletin-sdk-rust` crate provides a client for interacting with the Bulletin Chain. It works in both `std` and `no_std` (WASM/embedded) environments.

## Quick Start

```rust
use bulletin_sdk_rust::prelude::*;

// With a real node (SubxtSubmitter)
let submitter = SubxtSubmitter::from_url("ws://localhost:10000", signer).await?;
let client = AsyncBulletinClient::new(submitter);
let result = client.store(b"Hello!".to_vec(), StoreOptions::default()).await?;

// For testing (MockSubmitter)
let client = AsyncBulletinClient::new(MockSubmitter::new());
let result = client.store(b"Hello!".to_vec(), StoreOptions::default()).await?;
```

## Guides

- [Installation](./installation.md)
- [Basic Storage](./basic-storage.md)
- [Chunked Uploads](./chunked-uploads.md)
- [Authorization](./authorization.md)
- [Mock Testing](./mock-testing.md)
- [no_std Support](./no_std.md)
