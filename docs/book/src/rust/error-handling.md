# Error Handling

The SDK provides structured error handling through the `Error` enum, with built-in support for error codes, retry logic, and recovery hints.

## Error Enum

All SDK operations return `Result<T, Error>`. The `Error` enum covers all failure modes:

```rust
use bulletin_sdk_rust::prelude::*;
use tracing::{info, error};

match client.store(data).send().await {
    Ok(result) => {
        info!(cid = %hex::encode(&result.cid), "Stored successfully");
    }
    Err(Error::EmptyData) => {
        error!("Data cannot be empty");
    }
    Err(Error::InsufficientAuthorization { need, available }) => {
        error!(
            need_bytes = need,
            available_bytes = available,
            "Need more authorization"
        );
    }
    Err(e) if e.is_retryable() => {
        error!(?e, hint = e.recovery_hint(), "Retryable error");
        // Implement retry logic
    }
    Err(e) => {
        error!(code = e.code(), hint = e.recovery_hint(), "Fatal error");
    }
}
```

## Error Metadata Methods

Every error variant has three metadata methods for programmatic error handling:

### `code() -> &'static str`

Returns a `SCREAMING_SNAKE_CASE` string code consistent with the TypeScript SDK:

```rust
let err = Error::EmptyData;
assert_eq!(err.code(), "EMPTY_DATA");

let err = Error::ChunkTooLarge(3_000_000);
assert_eq!(err.code(), "CHUNK_TOO_LARGE");
```

### `is_retryable() -> bool`

Returns `true` if the error is likely transient and retrying may succeed:

```rust
match client.store(data).send().await {
    Err(e) if e.is_retryable() => {
        tracing::warn!(?e, "Transient error, retrying...");
        client.store(data).send().await?
    }
    Err(e) => return Err(e.into()),
    Ok(result) => result,
}
```

### `recovery_hint() -> &'static str`

Returns an actionable suggestion for resolving the error:

```rust
if let Err(e) = client.store(data).send().await {
    tracing::error!(
        code = e.code(),
        hint = e.recovery_hint(),
        "Storage failed: {}",
        e
    );
    // Logs: Storage failed: Data cannot be empty
    //       code="EMPTY_DATA" hint="Provide non-empty data"
}
```

## Error Variant Reference

### Data Validation Errors (Non-Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `EmptyData` | `EMPTY_DATA` | Provide non-empty data |
| `FileTooLarge(size)` | `FILE_TOO_LARGE` | Reduce file size or use chunked upload |
| `ChunkTooLarge(size)` | `CHUNK_TOO_LARGE` | Reduce chunk size to 2 MiB or less |
| `InvalidChunkSize(msg)` | `INVALID_CHUNK_SIZE` | Use a chunk size between 1 byte and 2 MiB |
| `InvalidConfig(msg)` | `INVALID_CONFIG` | Check configuration parameters |

### CID & Encoding Errors (Non-Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `InvalidCid(msg)` | `INVALID_CID` | Verify CID format |
| `CidCalculationFailed(msg)` | `CID_CALCULATION_FAILED` | Verify data and hash algorithm |
| `DagEncodingFailed(msg)` | `DAG_ENCODING_FAILED` | Check chunk CIDs and data integrity |

### Authorization Errors

| Variant | Code | Retryable | Recovery Hint |
|---|---|---|---|
| `AuthorizationNotFound(msg)` | `AUTHORIZATION_NOT_FOUND` | No | Call authorizeAccount() or authorizePreimage() first |
| `InsufficientAuthorization { need, available }` | `INSUFFICIENT_AUTHORIZATION` | No | Request additional authorization via authorize_account() |
| `AuthorizationExpired { expired_at, current_block }` | `AUTHORIZATION_EXPIRED` | Yes | Call refreshAccountAuthorization() to extend expiry |

### Network & Transaction Errors (Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `NetworkError(msg)` | `NETWORK_ERROR` | Check network connectivity to the RPC endpoint |
| `StorageFailed(msg)` | `STORAGE_FAILED` | Check node connectivity and try again |
| `TransactionFailed(msg)` | `TRANSACTION_FAILED` | Verify transaction parameters and account nonce |
| `RetrievalFailed(msg)` | `RETRIEVAL_FAILED` | The data may not be available yet; try again |
| `RenewalFailed(msg)` | `RENEWAL_FAILED` | Check that storage hasn't expired, then retry |

### Other Errors (Non-Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `ChunkingFailed(msg)` | `CHUNKING_FAILED` | Verify data integrity and chunker configuration |
| `RenewalNotFound { block, index }` | `RENEWAL_NOT_FOUND` | Verify the block number and extrinsic index |

## Progress Events

When using progress callbacks, you receive `ProgressEvent` which wraps either a chunk progress event or a transaction status event:

```rust
use std::sync::Arc;

let receipt = client.store_with_progress(
    data,
    &signer,
    Some(Arc::new(|event| {
        match event {
            ProgressEvent::Transaction(status) => {
                tracing::info!("{}", status.description());
            }
            ProgressEvent::Chunk(chunk_event) => {
                tracing::debug!(?chunk_event, "Chunk progress");
            }
        }
    })),
).await?;
```

### Transaction Status Events (`TransactionStatusEvent`)

Track the lifecycle of a submitted transaction:

| Variant | Description |
|---|---|
| `Validated` | Transaction validated and added to the pool |
| `Broadcasted` | Transaction broadcast to network peers |
| `InBestBlock { block_hash, block_number, extrinsic_index }` | Included in a best block |
| `Finalized { block_hash, block_number, extrinsic_index }` | Finalized (irreversible) |
| `NoLongerInBestBlock` | Removed from best block (chain reorg) |
| `Invalid { error }` | Transaction is no longer valid |
| `Dropped { error }` | Transaction dropped from the pool |

Each event has a `description()` method that returns a human-readable string:

```rust
let event = TransactionStatusEvent::Finalized {
    block_hash: "0xabc".into(),
    block_number: Some(42),
    extrinsic_index: None,
};
assert_eq!(event.description(), "Transaction finalized in block #42 (0xabc)");
```

### Chunk Progress Events (`ChunkProgressEvent`)

Track progress of chunked uploads:

| Variant | Description |
|---|---|
| `ChunkStarted { index, total }` | A chunk upload has started |
| `ChunkCompleted { index, total, cid }` | A chunk uploaded successfully |
| `ChunkFailed { index, total, error }` | A chunk upload failed |
| `ManifestStarted` | Manifest creation started |
| `ManifestCreated { cid }` | Manifest created and stored |
| `Completed { manifest_cid }` | All uploads completed |

### Combined `ProgressEvent` Enum

The `ProgressEvent` enum wraps both event types:

```rust
pub enum ProgressEvent {
    Chunk(ChunkProgressEvent),
    Transaction(TransactionStatusEvent),
}
```

Convenience constructors are available:

```rust
let event = ProgressEvent::chunk_started(0, 5);
let event = ProgressEvent::tx_validated();
let event = ProgressEvent::tx_finalized("0xabc".into(), Some(42), None);
```

## Cross-SDK Consistency

Error codes are consistent between the Rust and TypeScript SDKs where applicable. The Rust `Error::code()` method returns the same `SCREAMING_SNAKE_CASE` string as the TypeScript `ErrorCode` enum:

| Rust | TypeScript | Code String |
|---|---|---|
| `Error::EmptyData` | `ErrorCode.EMPTY_DATA` | `"EMPTY_DATA"` |
| `Error::TransactionFailed(_)` | `ErrorCode.TRANSACTION_FAILED` | `"TRANSACTION_FAILED"` |
| `Error::InsufficientAuthorization { .. }` | `ErrorCode.INSUFFICIENT_AUTHORIZATION` | `"INSUFFICIENT_AUTHORIZATION"` |

Some codes exist only in one SDK:
- **Rust-only**: `NETWORK_ERROR`, `STORAGE_FAILED`, `RETRIEVAL_FAILED`, `RENEWAL_FAILED`, `RENEWAL_NOT_FOUND`, `AUTHORIZATION_NOT_FOUND`, `AUTHORIZATION_EXPIRED`, `CHUNKING_FAILED`, `FILE_TOO_LARGE`
- **TypeScript-only**: `MISSING_CHUNK`, `DATA_TOO_LARGE`, `TIMEOUT`, `UNSUPPORTED_OPERATION`

The `is_retryable()` / `retryable` sets also differ: Rust includes more network-related codes (`NETWORK_ERROR`, `STORAGE_FAILED`, `RETRIEVAL_FAILED`, `RENEWAL_FAILED`, `AUTHORIZATION_EXPIRED`), while TypeScript covers `TRANSACTION_FAILED` and `TIMEOUT`.

## Common Error Patterns

### Authorization Flow

```rust
match client.store(data).send().await {
    Err(Error::AuthorizationNotFound(_)) => {
        tracing::error!("No authorization found. Call authorize_account() first.");
    }
    Err(Error::InsufficientAuthorization { need, available }) => {
        tracing::error!(need, available, "Insufficient authorization");
    }
    Err(Error::AuthorizationExpired { expired_at, current_block }) => {
        tracing::error!(expired_at, current_block, "Authorization expired, refreshing...");
        // Refresh authorization
    }
    Ok(result) => {
        tracing::info!("Success!");
    }
    Err(e) => return Err(e.into()),
}
```

### Retry with Backoff

```rust
let mut attempts = 0;
let max_retries = 3;

loop {
    match client.store(data.clone()).send().await {
        Ok(result) => break result,
        Err(e) if e.is_retryable() && attempts < max_retries => {
            attempts += 1;
            let delay = std::time::Duration::from_millis(100 * 2u64.pow(attempts));
            tracing::warn!(
                attempt = attempts,
                code = e.code(),
                hint = e.recovery_hint(),
                "Retrying after error"
            );
            tokio::time::sleep(delay).await;
        }
        Err(e) => return Err(e.into()),
    }
}
```
