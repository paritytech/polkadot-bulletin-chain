# Error Handling

The SDK provides structured error handling through `BulletinError` and the `ErrorCode` enum, with built-in support for retry logic and recovery hints.

## BulletinError

All SDK errors are instances of `BulletinError`, which extends the standard `Error` class:

```typescript
import { BulletinError, ErrorCode } from '@parity/bulletin-sdk'

try {
  const result = await client.store(data).send()
} catch (error) {
  if (error instanceof BulletinError) {
    console.error('Code:', error.code)        // e.g. "EMPTY_DATA"
    console.error('Message:', error.message)   // Human-readable description
    console.error('Retryable:', error.retryable) // Whether retrying may help
    console.error('Hint:', error.recoveryHint)   // Actionable suggestion
    console.error('Cause:', error.cause)         // Original error (if wrapped)
  }
}
```

## Error Code Reference

The `ErrorCode` enum provides all error codes as typed constants. Using `ErrorCode.*` instead of string literals gives you IDE autocomplete and compile-time checking.

Since `ErrorCode` values are string enums, they remain backward-compatible with string comparisons:

```typescript
// Both work identically
error.code === ErrorCode.EMPTY_DATA  // preferred
error.code === 'EMPTY_DATA'          // still works
```

### Data Validation Errors (Non-Retryable)

| Error Code | Description | Recovery Hint |
|---|---|---|
| `EMPTY_DATA` | Data is empty | Provide non-empty data |
| `DATA_TOO_LARGE` | Data exceeds 64 MiB limit | Reduce data size or use chunked upload |
| `CHUNK_TOO_LARGE` | Chunk exceeds 2 MiB limit | Reduce chunk size to 2 MiB or less |
| `INVALID_CHUNK_SIZE` | Chunk size is zero or negative | Use a chunk size between 1 byte and 2 MiB |
| `INVALID_CONFIG` | Configuration is invalid | Check configuration parameters |

### CID & Encoding Errors (Non-Retryable)

| Error Code | Description | Recovery Hint |
|---|---|---|
| `INVALID_CID` | CID format is invalid | Verify CID format |
| `INVALID_HASH_ALGORITHM` | Hash algorithm not supported | Use blake2b-256, sha2-256, or keccak-256 |
| `CID_CALCULATION_FAILED` | CID calculation failed | Verify data and hash algorithm |
| `DAG_ENCODING_FAILED` | DAG-PB encoding failed | Check chunk CIDs and data integrity |

### Authorization Errors (Non-Retryable)

| Error Code | Description | Recovery Hint |
|---|---|---|
| `INSUFFICIENT_AUTHORIZATION` | Authorized quota insufficient | Request additional authorization via authorizeAccount() |
| `AUTHORIZATION_FAILED` | Authorization call failed | Check that the account has authorizer privileges |

### Transaction & Upload Errors

| Error Code | Retryable | Description | Recovery Hint |
|---|---|---|---|
| `TRANSACTION_FAILED` | Yes | On-chain transaction failed | Verify transaction parameters and account nonce |
| `TIMEOUT` | Yes | Operation timed out | Increase `txTimeout` in `ClientConfig` or retry |
| `CHUNK_FAILED` | No | Individual chunk upload failed | Verify data integrity and chunker configuration |
| `MISSING_CHUNK` | No | Chunk missing during reassembly | Ensure all chunks are present with contiguous indices starting from 0 |
| `UNSUPPORTED_OPERATION` | No | Operation not supported | This operation is not supported in this context |

## Retryable Errors

The `retryable` getter indicates whether an error is likely transient and retrying may succeed:

```typescript
try {
  await client.store(data).send()
} catch (error) {
  if (error instanceof BulletinError && error.retryable) {
    console.log('Transient error, retrying...')
    // Implement your retry logic
    await client.store(data).send()
  } else {
    throw error // Non-retryable, propagate
  }
}
```

Only `TRANSACTION_FAILED` and `TIMEOUT` are retryable. All other errors indicate issues that must be fixed before retrying.

## Recovery Hints

The `recoveryHint` getter returns an actionable suggestion for resolving the error:

```typescript
try {
  await client.store(data).send()
} catch (error) {
  if (error instanceof BulletinError) {
    console.error(`Error: ${error.message}`)
    console.error(`Suggestion: ${error.recoveryHint}`)
    // e.g. "Suggestion: Provide non-empty data"
    // e.g. "Suggestion: Verify transaction parameters and account nonce"
  }
}
```

## Progress Events

When using progress callbacks, you receive two types of events: **transaction status events** and **chunk progress events**.

### Transaction Status Events (`TxStatus`)

Track the lifecycle of a submitted transaction:

```typescript
import { TxStatus } from '@parity/bulletin-sdk'

const result = await client
  .store(data)
  .withCallback((event) => {
    switch (event.type) {
      case TxStatus.Signed:
        console.log(`Transaction signed: ${event.txHash}`)
        break
      case TxStatus.Validated:
        console.log('Transaction validated and added to pool')
        break
      case TxStatus.Broadcasted:
        console.log('Transaction broadcast to peers')
        break
      case TxStatus.InBlock:
        console.log(`In best block #${event.blockNumber} (${event.blockHash})`)
        break
      case TxStatus.Finalized:
        console.log(`Finalized in block #${event.blockNumber}`)
        break
      case TxStatus.NoLongerInBlock:
        console.log('Block reorganization occurred')
        break
      case TxStatus.Invalid:
        console.error(`Transaction invalid: ${event.error}`)
        break
      case TxStatus.Dropped:
        console.error(`Transaction dropped: ${event.error}`)
        break
    }
  })
  .send()
```

| Event | Value | Fields | Description |
|---|---|---|---|
| `Signed` | `"signed"` | `txHash`, `chunkIndex?` | Transaction signed and ready |
| `Validated` | `"validated"` | `chunkIndex?` | Validated by the node |
| `Broadcasted` | `"broadcasted"` | `chunkIndex?` | Broadcast to network peers |
| `InBlock` | `"in_block"` | `blockHash`, `blockNumber`, `txIndex?`, `chunkIndex?` | Included in a best block |
| `Finalized` | `"finalized"` | `blockHash`, `blockNumber`, `txIndex?`, `chunkIndex?` | Finalized (irreversible) |
| `NoLongerInBlock` | `"no_longer_in_block"` | `chunkIndex?` | Removed from best block (reorg) |
| `Invalid` | `"invalid"` | `error`, `chunkIndex?` | Transaction is invalid |
| `Dropped` | `"dropped"` | `error`, `chunkIndex?` | Dropped from the transaction pool |

> **Note**: All events include an optional `chunkIndex` field to identify which chunk the event relates to during chunked uploads.

### Chunk Progress Events (`ChunkStatus`)

Track progress of chunked uploads:

```typescript
import { ChunkStatus } from '@parity/bulletin-sdk'

const result = await client
  .store(largeData)
  .withCallback((event) => {
    switch (event.type) {
      case ChunkStatus.ChunkStarted:
        console.log(`Starting chunk ${event.index + 1}/${event.total}`)
        break
      case ChunkStatus.ChunkCompleted:
        console.log(`Chunk ${event.index + 1}/${event.total} done: ${event.cid}`)
        break
      case ChunkStatus.ChunkFailed:
        console.error(`Chunk ${event.index + 1}/${event.total} failed: ${event.error}`)
        break
      case ChunkStatus.ManifestStarted:
        console.log('Creating manifest...')
        break
      case ChunkStatus.ManifestCreated:
        console.log(`Manifest created: ${event.cid}`)
        break
      case ChunkStatus.Completed:
        console.log('All uploads complete!')
        break
    }
  })
  .send()
```

| Event | Value | Fields | Description |
|---|---|---|---|
| `ChunkStarted` | `"chunk_started"` | `index`, `total` | A chunk upload has started |
| `ChunkCompleted` | `"chunk_completed"` | `index`, `total`, `cid` | A chunk uploaded successfully |
| `ChunkFailed` | `"chunk_failed"` | `index`, `total`, `error` | A chunk upload failed |
| `ManifestStarted` | `"manifest_started"` | -- | Manifest creation started |
| `ManifestCreated` | `"manifest_created"` | `cid` | Manifest created and stored |
| `Completed` | `"completed"` | `manifestCid?` | All uploads completed |

## Common Error Patterns

### Authorization Flow

```typescript
import { BulletinError, ErrorCode } from '@parity/bulletin-sdk'

try {
  const result = await client.store(data).send()
  console.log('Stored:', result.cid.toString())
} catch (error) {
  if (!(error instanceof BulletinError)) throw error

  switch (error.code) {
    case ErrorCode.INSUFFICIENT_AUTHORIZATION:
      // Not enough quota remaining — the SDK checks this before submission
      console.error('Need more authorization:', error.message)
      console.error('Hint:', error.recoveryHint)
      break
    case ErrorCode.AUTHORIZATION_FAILED:
      // Authorization call itself failed
      console.error('Authorization failed:', error.message)
      break
    default:
      console.error(error.message)
  }
}
```

### Chunked Upload Error Handling

Chunked uploads are **not atomic**. If a chunk fails mid-upload, previously submitted chunks remain on-chain. The error contains details about which chunk failed.

```typescript
const result = await client
  .store(largeData)
  .withCallback((event) => {
    if (event.type === ChunkStatus.ChunkFailed) {
      console.error(`Chunk ${event.index + 1}/${event.total} failed:`, event.error)
    }
  })
  .send()
```

### Wrapping External Errors

When integrating with other libraries, wrap errors to preserve context:

```typescript
try {
  // Some external operation
} catch (cause) {
  throw new BulletinError(
    'Failed to process data',
    ErrorCode.TRANSACTION_FAILED,
    cause  // Preserved as error.cause
  )
}
```

## Cross-SDK Consistency

Error codes are consistent between the Rust and TypeScript SDKs where applicable:

| TypeScript | Rust | Code String |
|---|---|---|
| `ErrorCode.EMPTY_DATA` | `Error::EmptyData` | `"EMPTY_DATA"` |
| `ErrorCode.TRANSACTION_FAILED` | `Error::TransactionFailed(_)` | `"TRANSACTION_FAILED"` |
| `ErrorCode.INSUFFICIENT_AUTHORIZATION` | `Error::InsufficientAuthorization { .. }` | `"INSUFFICIENT_AUTHORIZATION"` |

Some codes exist only in one SDK:
- **Rust-only**: `NETWORK_ERROR`, `STORAGE_FAILED`, `RETRIEVAL_FAILED`, `RENEWAL_FAILED`, `RENEWAL_NOT_FOUND`, `AUTHORIZATION_NOT_FOUND`, `AUTHORIZATION_EXPIRED`, `CHUNKING_FAILED`, `FILE_TOO_LARGE`, `DAG_DECODING_FAILED`, `RETRY_EXHAUSTED`
- **TypeScript-only**: `MISSING_CHUNK`, `DATA_TOO_LARGE`

The `retryable` sets also differ: Rust includes more network-related codes (`NETWORK_ERROR`, `STORAGE_FAILED`, `RETRIEVAL_FAILED`, `RENEWAL_FAILED`, `AUTHORIZATION_EXPIRED`), while TypeScript covers `TRANSACTION_FAILED` and `TIMEOUT`.
