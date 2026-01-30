# Authorization

Use the `estimate_authorization` helper to calculate costs.

```rust
let client = BulletinClient::new();
let file_size = 100 * 1024 * 1024; // 100 MiB

let (txs, bytes) = client.estimate_authorization(file_size);

println!("Need to authorize {} transactions and {} bytes", txs, bytes);
```

Then submit the authorization transaction:

```rust
let tx = bulletin::tx().transaction_storage().authorize_account(
    target_account,
    txs,
    bytes
);
```
