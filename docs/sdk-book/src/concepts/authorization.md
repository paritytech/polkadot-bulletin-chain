# Authorization

Before storing data, you must authorize the storage.

## Account Authorization (`authorize_account`)

Authorizes a specific account to store up to N transactions / M bytes. The account can store any data within the limit.

Parameters: `who`, `transactions`, `bytes`

## Preimage Authorization (`authorize_preimage`)

Authorizes a specific piece of data (by hash) to be stored by anyone. Only data matching the hash can be stored.

Parameters: `content_hash`, `max_size`

## Estimating Authorization

The SDKs provide `estimate_authorization` / `estimateAuthorization` to calculate the number of transactions and bytes needed for a given file size (accounting for chunks + manifest overhead).

```rust
// Rust
let (txs, bytes) = client.estimate_authorization(file_size);
```

```typescript
// TypeScript
const { transactions, bytes } = client.estimateAuthorization(fileSize);
```
