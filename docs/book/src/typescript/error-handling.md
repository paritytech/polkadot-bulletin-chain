# Error Handling

The SDK provides structured error handling through `BulletinError` and the `ErrorCode` enum, with built-in support for retry logic and recovery hints.

## BulletinError

All SDK errors are instances of `BulletinError`, which extends the standard `Error` class:

```typescript
import { BulletinError, ErrorCode } from '@bulletin/sdk'

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

## ErrorCode Enum

The `ErrorCode` enum provides all error codes as typed constants. Using `ErrorCode.*` instead of string literals gives you IDE autocomplete and compile-time checking:

```typescript
import { BulletinError, ErrorCode } from '@bulletin/sdk'

try {
  await client.store(data).send()
} catch (error) {
  if (error instanceof BulletinError) {
    switch (error.code) {
      case ErrorCode.EMPTY_DATA:
        console.error('Data cannot be empty')
        break
      case ErrorCode.INSUFFICIENT_AUTHORIZATION:
        console.error('Need more authorization')
        break
      case ErrorCode.AUTHORIZATION_EXPIRED:
        console.error('Authorization has expired')
        break
      default:
        console.error('Error:', error.message)
    }
  }
}
```

Since `ErrorCode` values are string enums, they remain backward-compatible with string comparisons:

```typescript
// Both work identically
error.code === ErrorCode.EMPTY_DATA  // preferred
error.code === 'EMPTY_DATA'          // still works
```

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

### Retryable Error Codes

| Error Code | Description |
|---|---|
| `AUTHORIZATION_EXPIRED` | Authorization has expired; refresh it |
| `NETWORK_ERROR` | Network connectivity issue |
| `STORAGE_FAILED` | Storage operation failed on-chain |
| `SUBMISSION_FAILED` | Transaction submission failed |
| `TRANSACTION_FAILED` | On-chain transaction failed |
| `RETRIEVAL_FAILED` | Data retrieval failed |
| `RENEWAL_FAILED` | Renewal operation failed |
| `TIMEOUT` | Operation timed out |

### Non-Retryable Error Codes

| Error Code | Description |
|---|---|
| `EMPTY_DATA` | Data is empty |
| `FILE_TOO_LARGE` | File exceeds 64 MiB limit |
| `CHUNK_TOO_LARGE` | Chunk exceeds 2 MiB limit |
| `INVALID_CHUNK_SIZE` | Chunk size is invalid |
| `INVALID_CONFIG` | Configuration is invalid |
| `INVALID_CID` | CID format is invalid |
| `UNSUPPORTED_HASH_ALGORITHM` | Hash algorithm not supported |
| `INVALID_HASH_ALGORITHM` | Hash algorithm code is invalid |
| `CID_CALCULATION_FAILED` | CID calculation failed |
| `DAG_ENCODING_FAILED` | DAG-PB encoding failed |
| `DAG_DECODING_FAILED` | DAG-PB decoding failed |
| `AUTHORIZATION_NOT_FOUND` | No authorization found |
| `INSUFFICIENT_AUTHORIZATION` | Authorized quota insufficient |
| `AUTHORIZATION_FAILED` | Authorization call failed |
| `CHUNKING_FAILED` | Chunking operation failed |
| `CHUNK_FAILED` | Individual chunk upload failed |
| `RENEWAL_NOT_FOUND` | Renewal target not found |
| `UNSUPPORTED_OPERATION` | Operation not supported |
| `RETRY_EXHAUSTED` | All retry attempts failed |

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
    // e.g. "Suggestion: Check node connectivity and try again"
  }
}
```

## Transaction Status Events

When using progress callbacks, you receive `TransactionStatusEvent`s that track the lifecycle of a transaction:

```typescript
const result = await client
  .store(data)
  .withCallback((event) => {
    switch (event.type) {
      case 'validated':
        console.log('Transaction validated and added to pool')
        break
      case 'broadcasted':
        console.log('Transaction broadcast to peers')
        break
      case 'in_best_block':
        console.log(`In best block #${event.blockNumber} (${event.blockHash})`)
        break
      case 'finalized':
        console.log(`Finalized in block #${event.blockNumber}`)
        break
      case 'no_longer_in_best_block':
        console.log('Block reorganization occurred')
        break
      case 'invalid':
        console.error(`Transaction invalid: ${event.error}`)
        break
      case 'dropped':
        console.error(`Transaction dropped: ${event.error}`)
        break
    }
  })
  .send()
```

### Event Types

| Event | Fields | Description |
|---|---|---|
| `validated` | -- | Transaction validated by the node |
| `broadcasted` | `numPeers?` | Broadcast to network peers |
| `in_best_block` | `blockHash`, `blockNumber`, `txIndex?` | Included in a best block |
| `finalized` | `blockHash`, `blockNumber`, `txIndex?` | Finalized (irreversible) |
| `no_longer_in_best_block` | -- | Removed from best block (reorg) |
| `invalid` | `error` | Transaction is invalid |
| `dropped` | `error` | Dropped from the transaction pool |

> **Note**: The `best_block` event type is deprecated. Use `in_best_block` instead.

## Common Error Patterns

### Authorization Flow

```typescript
import { BulletinError, ErrorCode } from '@bulletin/sdk'

try {
  const result = await client.store(data).send()
  console.log('Stored:', result.cid.toString())
} catch (error) {
  if (!(error instanceof BulletinError)) throw error

  switch (error.code) {
    case ErrorCode.AUTHORIZATION_NOT_FOUND:
      // Account has no authorization at all
      console.error('Please authorize your account first')
      break
    case ErrorCode.INSUFFICIENT_AUTHORIZATION:
      // Not enough quota remaining
      console.error('Need more authorization:', error.message)
      break
    case ErrorCode.AUTHORIZATION_EXPIRED:
      // Authorization expired, refresh it
      console.error('Authorization expired, refreshing...')
      break
    default:
      console.error(error.message)
  }
}
```

### Chunked Upload Error Handling

```typescript
const result = await client
  .store(largeData)
  .withCallback((event) => {
    if (event.type === 'chunk_failed') {
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
    ErrorCode.STORAGE_FAILED,
    cause  // Preserved as error.cause
  )
}
```
