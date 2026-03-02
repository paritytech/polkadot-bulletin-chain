# Authorization

## Quick Authorization (Recommended)

For most use cases, use `estimate_authorization` to automatically calculate the required authorization:

```rust
use bulletin_sdk_rust::prelude::*;
use tracing::info;

let client = BulletinClient::new();
let file_size = 100 * 1024 * 1024; // 100 MiB

// Automatically calculates transactions and bytes needed
let (txs, bytes) = client.estimate_authorization(file_size);
info!(transactions = txs, bytes = bytes, "Authorization needed");
```

> **Convenience Note**: The estimation is automatic - you just provide the file size, and it calculates chunking, manifest overhead, etc. A future enhancement could provide `authorize_for_size(account, size)` that combines estimation and authorization submission into one call.

Then submit the authorization transaction:

```rust
let tx = bulletin::tx().transaction_storage().authorize_account(
    target_account,
    txs,
    bytes
);
```
