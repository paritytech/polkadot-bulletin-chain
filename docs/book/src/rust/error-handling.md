# Error Handling

The SDK provides structured error handling through the `Error` enum, with built-in support for error codes, retry logic, and recovery hints.

## Error Enum

All SDK operations return `Result<T, Error>`. The `Error` enum covers all failure modes:

```rust
use bulletin_sdk_rust::prelude::*;
use tracing::{info, error};

match client.store(data, &signer, WaitFor::InBlock).await {
    Ok(receipt) => {
        info!(block_hash = %receipt.block_hash, "Stored successfully");
    }
    Err(Error::InsufficientAuthorization { need, available }) => {
        // Transactions or bytes, whichever is short
        error!(need, available, "Need more authorization");
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
match client.store(data.clone(), &signer, WaitFor::InBlock).await {
    Err(e) if e.is_retryable() => {
        tracing::warn!(?e, "Transient error, retrying...");
        client.store(data, &signer, WaitFor::InBlock).await?
    }
    Err(e) => return Err(e.into()),
    Ok(result) => result,
}
```

### `recovery_hint() -> &'static str`

Returns an actionable suggestion for resolving the error:

```rust
if let Err(e) = client.store(data, &signer, WaitFor::InBlock).await {
    tracing::error!(
        code = e.code(),
        hint = e.recovery_hint(),
        "Storage failed: {}",
        e
    );
    // Logs: Storage failed: Authorization not found for account: 5Grw...
    //       code="AUTHORIZATION_NOT_FOUND" hint="Authorize the account or the content hash first, then retry"
}
```

## Error Variant Reference

### Data Validation Errors (Non-Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `EmptyData` | `EMPTY_DATA` | Provide non-empty data |
| `DataTooLarge(msg)` | `DATA_TOO_LARGE` | Reduce the data size or use a chunked upload |
| `ChunkTooLarge(size)` | `CHUNK_TOO_LARGE` | Reduce chunk size to 2 MiB or less |
| `InvalidChunkSize(msg)` | `INVALID_CHUNK_SIZE` | Use a chunk size between 1 byte and 2 MiB |
| `InvalidConfig(msg)` | `INVALID_CONFIG` | Check configuration parameters |

`EmptyData` comes from the offline and planning APIs (`prepare_store`, `prepare_store_chunked`, `estimate_upload`). `store` does not check for empty data; the transaction pool rejects it, and the error surfaces as `TransactionInvalid` with `custom_code: Some(1)` (`BAD_DATA_SIZE`).

### CID & Encoding Errors (Non-Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `InvalidCid(msg)` | `INVALID_CID` | Verify CID format |
| `CidCalculationFailed(msg)` | `CID_CALCULATION_FAILED` | Verify data and hash algorithm |
| `DagEncodingFailed(msg)` | `DAG_ENCODING_FAILED` | Check chunk CIDs and data integrity |
| `ChunkingFailed(msg)` | `CHUNKING_FAILED` | Verify data integrity and chunker configuration |

### Authorization Errors (Non-Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `AuthorizationNotFound(msg)` | `AUTHORIZATION_NOT_FOUND` | Authorize the account or the content hash first, then retry |
| `InsufficientAuthorization { need, available }` | `INSUFFICIENT_AUTHORIZATION` | Request additional authorization, then retry |

The runtime API that the pre-store check queries leaves out expired authorizations, so an expired authorization surfaces as `AuthorizationNotFound`.

### Transaction Errors

| Variant | Code | Retryable | Recovery Hint |
|---|---|---|---|
| `TransactionInvalid { custom_code, message }` | `TRANSACTION_INVALID` | No | The transaction pool rejected the transaction; check the validity error before retrying |
| `TransactionFailed(msg)` | `TRANSACTION_FAILED` | Yes | Verify transaction parameters and account nonce, then retry |

Every submission method (`store`, `authorize_*`, `refresh_*`, `remove_expired_*`, `renew`, `force_renew`, `enable_auto_renew`, `disable_auto_renew`) reports failures through these two variants, and the message names the call. `TransactionInvalid` means the transaction pool rejected the transaction; `custom_code` holds `n` when the rejection is `InvalidTransaction::Custom(n)` (for example 11, `RENEWAL_ALREADY_ENABLED`; see [Renewal](./renewal.md)). `TransactionFailed` covers a dispatch error, a dropped transaction and a status stream that ends before inclusion.

### Network Errors (Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `NetworkError(msg)` | `NETWORK_ERROR` | Check network connectivity to the RPC endpoint, then retry |
| `StoreStalled(msg)` | `STORE_STALLED` | The RPC connection sent no chainHead progress; retry on a fresh client |

### Upload Pipeline Errors (Non-Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `UploadIncomplete(msg)` | `UPLOAD_INCOMPLETE` | Some items were not stored; re-run the estimate with skip-existing enabled, then submit to retry only the missing items |
| `HijackBudgetExceeded(msg)` | `HIJACK_BUDGET_EXCEEDED` | Another transaction from this signer keeps taking the nonce; avoid concurrent submissions on this account |

The current pipeline does not return `HijackBudgetExceeded`: an item that exhausts its retries emits `ItemFailed`, and the run ends with `UploadIncomplete`.

### Chain Capability Errors (Non-Retryable)

| Variant | Code | Recovery Hint |
|---|---|---|
| `UnsupportedOperation(msg)` | `UNSUPPORTED_OPERATION` | This operation is not supported by this client or the connected runtime |
| `AuthorizationApiUnavailable` | `UNSUPPORTED_OPERATION` | This operation is not supported by this client or the connected runtime |

`UnsupportedOperation` covers a chain without a `renew` call, renewal and auto-renew call shapes that this SDK release does not know, and content-hash renewal or `force_renew` on legacy runtimes. `AuthorizationApiUnavailable` is a separate variant because the `store` pre-check skips itself on it.

## Progress Events

When using progress callbacks, you receive `ProgressEvent` which wraps either a chunk progress event or a transaction status event:

```rust
use std::sync::Arc;

let receipt = client.store_with_progress(
    data,
    &signer,
    WaitFor::InBlock,
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
| `Signed { tx_hash, chunk_index }` | Transaction signed (not emitted by `store_with_progress`) |
| `Validated { chunk_index }` | Transaction validated and added to the pool |
| `Broadcasted { chunk_index }` | Transaction broadcast to network peers |
| `InBlock { block_hash, block_number, tx_index, chunk_index }` | Included in a best block |
| `Finalized { block_hash, block_number, tx_index, chunk_index }` | Finalized (irreversible) |
| `NoLongerInBlock { chunk_index }` | Removed from best block (chain reorg) |
| `Invalid { error, chunk_index }` | Transaction is no longer valid |
| `Dropped { error, chunk_index }` | Transaction dropped from the pool |

`tx_index` is the `Stored` event index that `renew` takes, not the extrinsic index; `store_with_progress` sets it to `None`.

Each event has a `description()` method that returns a human-readable string:

```rust
let event = TransactionStatusEvent::Finalized {
    block_hash: "0xabc".into(),
    block_number: Some(42),
    tx_index: None,
    chunk_index: None,
};
assert_eq!(event.description(), "Transaction finalized in block #42 (0xabc)");
```

### Chunk Progress Events (`ChunkProgressEvent`)

Only `BulletinClient::prepare_store_chunked` emits chunk events: `ChunkStarted { index: 0, total }` once, then `ManifestStarted` and `ManifestCreated` when it builds a manifest. No SDK method emits the other variants.

| Variant | Description |
|---|---|
| `ChunkStarted { index, total }` | Chunk preparation started |
| `ChunkCompleted { index, total, cid }` | A chunk uploaded successfully |
| `ChunkFailed { index, total, error }` | A chunk upload failed |
| `ManifestStarted` | Manifest creation started |
| `ManifestCreated { cid }` | Manifest built (not yet stored) |
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

### Upload Events (`UploadEvent`)

`submit` and `submit_unsigned` report progress through `UploadConfig.on_event`:

| Variant | Description |
|---|---|
| `ItemStarted { index, total, cid }` | Item accepted into the pipeline |
| `ItemInBlock { index, total, cid, block_hash, block_number, transaction_index }` | Item included in a best block; fires again if a reorg moves the item |
| `ItemFinalized { index, total, cid, block_hash, block_number, transaction_index }` | Item finalized |
| `ItemFailed { index, total, cid, error }` | Item failed permanently; the run ends with `UploadIncomplete` |

## Cross-SDK Consistency

Both SDKs use the same error codes, the same retryable set and the same recovery hints: `Error::code()` returns the `SCREAMING_SNAKE_CASE` string of the TypeScript `ErrorCode` for the same failure. Three TypeScript codes have no Rust counterpart because the failure cannot occur in the Rust SDK:

- `TIMEOUT`: the TypeScript client stops waiting after `txTimeout`; the Rust client has no submission timeout.
- `INVALID_HASH_ALGORITHM`: `HashingAlgorithm` is a Rust enum, so an unknown algorithm does not compile.
- `MISSING_CHUNK`: only the TypeScript chunker reassembles chunks.

`INSUFFICIENT_AUTHORIZATION` comes only from this SDK's capacity check; the TypeScript pre-flight checks existence and expiry only.

Retryable codes: `TRANSACTION_FAILED`, `NETWORK_ERROR` and `STORE_STALLED`, plus `TIMEOUT` in TypeScript.

A transaction pool rejection is `TRANSACTION_INVALID` in both SDKs. Rust carries the custom validity code in `Error::TransactionInvalid { custom_code, .. }`; TypeScript keeps PAPI's `InvalidTxError` as the error's `cause`.

## Common Error Patterns

### Authorization Flow

```rust
match client.store(data, &signer, WaitFor::InBlock).await {
    Err(Error::AuthorizationNotFound(_)) => {
        // Missing or expired authorization
        tracing::error!("No authorization found. Call authorize_account() or refresh it.");
    }
    Err(Error::InsufficientAuthorization { need, available }) => {
        tracing::error!(need, available, "Insufficient authorization");
    }
    Ok(result) => {
        tracing::info!("Success!");
    }
    Err(e) => return Err(e.into()),
}
```

### Retry with Backoff

Each retry of `store` submits a new transaction. For exactly-once uploads, use `estimate_upload` → `submit`, which retries internally.

```rust
let mut attempts = 0;
let max_retries = 3;

loop {
    match client.store(data.clone(), &signer, WaitFor::InBlock).await {
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
