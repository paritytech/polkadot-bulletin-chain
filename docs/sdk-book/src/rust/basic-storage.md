# Basic Storage

This guide shows how to store a small piece of data (< 8 MiB).

## 1. Initialize Client

```rust
use bulletin_sdk_rust::prelude::*;

let client = BulletinClient::new();
```

## 2. Prepare Data

```rust
let data = b"Hello, Bulletin!".to_vec();
```

## 3. Prepare Operation

The `prepare_store` method validates the data and calculates the CID.

```rust
let options = StoreOptions::default();
let operation = client.prepare_store(data, options)?;

println!("Data Size: {}", operation.size());
// operation.data contains the bytes to submit
```

## 4. Submit Transaction (with subxt)

Assuming you have a `subxt` client connected:

```rust
// This part depends on your subxt generation
let tx = bulletin::tx().transaction_storage().store(operation.data);

api.tx()
   .sign_and_submit_then_watch_default(&tx, &signer)
   .await?
   .wait_for_finalized_success()
   .await?;
```
