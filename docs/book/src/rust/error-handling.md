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

## Error Variants

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
| `DagDecodingFailed(msg)` | `DAG_DECODING_FAILED` | Verify DAG-PB data format |

### Authorization Errors

| Variant | Code | Retryable | Recovery Hint |
|---|---|---|---|
| `AuthorizationNotFound(msg)` | `AUTHORIZATION_NOT_FOUND` | No | Call `authorize_account()` first |
| `InsufficientAuthorization { need, available }` | `INSUFFICIENT_AUTHORIZATION` | No | Request additional authorization |
| `AuthorizationExpired { expired_at, current_block }` | `AUTHORIZATION_EXPIRED` | Yes | Refresh authorization |
| `AuthorizationFailed(msg)` | `AUTHORIZATION_FAILED` | No | Check authorizer privileges |

### Network & Transaction Errors (Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `NetworkError(msg)` | `NETWORK_ERROR` | Check network connectivity |
| `StorageFailed(msg)` | `STORAGE_FAILED` | Check node connectivity and try again |
| `TransactionFailed(msg)` | `TRANSACTION_FAILED` | Verify transaction parameters and nonce |
| `RetrievalFailed(msg)` | `RETRIEVAL_FAILED` | Data may not be available yet; try again |
| `RenewalFailed(msg)` | `RENEWAL_FAILED` | Check that storage hasn't expired |
| `Timeout(msg)` | `TIMEOUT` | Increase timeout or retry |

### Other Errors (Non-Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `ChunkingFailed(msg)` | `CHUNKING_FAILED` | Verify data integrity and chunker config |
| `RenewalNotFound { block, index }` | `RENEWAL_NOT_FOUND` | Verify block number and extrinsic index |
| `UnsupportedOperation(msg)` | `UNSUPPORTED_OPERATION` | Operation not supported in this context |
| `RetryExhausted(msg)` | `RETRY_EXHAUSTED` | Check underlying cause |

## Transaction Status Events

When using progress callbacks, you receive `TransactionStatusEvent`s that track the lifecycle of a transaction:

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

### Event Variants

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

## Cross-SDK Consistency

Error codes are consistent between the Rust and TypeScript SDKs. The Rust `Error::code()` method returns the same string as the TypeScript `ErrorCode` enum value:

| Rust | TypeScript | Code String |
|---|---|---|
| `Error::EmptyData` | `ErrorCode.EMPTY_DATA` | `"EMPTY_DATA"` |
| `Error::StorageFailed(_)` | `ErrorCode.STORAGE_FAILED` | `"STORAGE_FAILED"` |
| `Error::TransactionFailed(_)` | `ErrorCode.TRANSACTION_FAILED` | `"TRANSACTION_FAILED"` |

This makes it easy to handle errors consistently across polyglot systems or when translating between SDKs.

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
        tracing::error!(expired_at, current_block, "Authorization expired");
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
