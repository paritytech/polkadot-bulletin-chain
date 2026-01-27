# Authorization

Before storing data, you must authorize the storage. This mechanism prevents spam and ensures users pay for the state bloat they introduce.

## Types of Authorization

### 1. Account Authorization (`authorize_account`)
This authorizes a specific **account** (public key) to store a certain amount of data.
- **Flexible**: The account can store *any* data up to the limit.
- **Usage**: Good for active users or applications uploading dynamic content.
- **Parameters**:
    - `who`: The account to authorize.
    - `transactions`: Number of transactions allowed.
    - `bytes`: Total bytes allowed.

### 2. Preimage Authorization (`authorize_preimage`)
This authorizes a specific **piece of data** (identified by its hash) to be stored by *anyone*.
- **Restricted**: Only data matching the authorized hash can be stored.
- **Usage**: Good for "sponsored" uploads where you want to pay for a specific file to be hosted, regardless of who submits the transaction.
- **Parameters**:
    - `content_hash`: Hash of the data.
    - `max_size`: Maximum size of the data.

## SDK Helpers

The SDK provides `estimate_authorization` / `estimateAuthorization` to help you calculate the required values.

**Example Calculation:**
If you want to store a 100 MiB file with 1 MiB chunks:
- **Chunks**: 100
- **Manifest**: 1
- **Total Transactions**: 101
- **Total Bytes**: 100 MiB + size of manifest (~2KB)

```rust
// Rust
let (txs, bytes) = client.estimate_authorization(100_000_000);
```

```typescript
// TypeScript
const { transactions, bytes } = client.estimateAuthorization(100000000);
```
