# Error Handling

The SDK throws `BulletinError`, a subclass of `Error` carrying a typed `ErrorCode`, a `retryable` flag, and a `recoveryHint`.

```typescript
import { BulletinError, ErrorCode } from '@parity/bulletin-sdk'

try {
  await client.submit(await client.estimateUpload(src), src).send()
} catch (error) {
  if (error instanceof BulletinError) {
    console.error('Code:', error.code)            // e.g. "EMPTY_DATA"
    console.error('Message:', error.message)
    console.error('Retryable:', error.retryable)  // whether retrying may help
    console.error('Hint:', error.recoveryHint)    // actionable suggestion
    console.error('Cause:', error.cause)          // wrapped original error, if any
  }
}
```

`ErrorCode` is a string enum, so `error.code === ErrorCode.EMPTY_DATA` and `error.code === 'EMPTY_DATA'` are equivalent; prefer the enum for autocomplete.

## Error Code Reference

### Data & CID (non-retryable)

| Code | Description | Recovery Hint |
|---|---|---|
| `EMPTY_DATA` | Data is empty | Provide non-empty data |
| `DATA_TOO_LARGE` | Exceeds the 64 MiB limit | Reduce the data size |
| `CHUNK_TOO_LARGE` | Chunk exceeds 2 MiB | Use a chunk size ≤ 2 MiB |
| `INVALID_CHUNK_SIZE` | Chunk size is zero or negative | Use 1 byte – 2 MiB |
| `INVALID_CONFIG` | Invalid config (e.g. duplicate content hashes in one submit) | Check the inputs |
| `INVALID_CID` | Malformed CID | Verify the CID |
| `INVALID_HASH_ALGORITHM` | Unsupported hash algorithm | Use Blake2b-256, SHA2-256, or Keccak-256 |
| `CID_CALCULATION_FAILED` | CID calculation failed | Verify data and hash algorithm |
| `DAG_ENCODING_FAILED` | DAG-PB encoding failed | Check chunk CIDs and data |

### Authorization (non-retryable)

| Code | Description | Recovery Hint |
|---|---|---|
| `INSUFFICIENT_AUTHORIZATION` | Quota insufficient or no/expired entry | Authorize via `authorizeAccount()` |
| `AUTHORIZATION_FAILED` | Authorization call failed | Check the account has authorizer privileges |
| `UNSUPPORTED_OPERATION` | Called a signed path on a signer-less client (and similar) | Provide the required signer |

### Transaction & Upload

| Code | Retryable | Description |
|---|---|---|
| `TRANSACTION_FAILED` | Yes | On-chain transaction failed |
| `TIMEOUT` | Yes | Not included within the timeout window |
| `STORE_STALLED` | Yes | Submission stalled (pool saturation / reorg); the SDK retries internally |
| `CHUNK_FAILED` | No | A chunk failed to prepare |
| `MISSING_CHUNK` | No | A chunk was missing during reassembly |
| `HIJACK_BUDGET_EXCEEDED` | No | Too many nonce reassignments under concurrent same-account submission |

`error.retryable` is true for `TRANSACTION_FAILED`, `TIMEOUT`, and `STORE_STALLED`. Note that `submit()` already retries stalls internally before surfacing one.

```typescript
try {
  await client.submit(estimate, src).send()
} catch (error) {
  if (error instanceof BulletinError && error.retryable) {
    await client.submit(estimate, src).send() // estimate is reusable
  } else {
    throw error
  }
}
```

## Progress Events

`withCallback` receives an `UploadEvent` per stored unit. `index` is the unit's position in the source (chunks first, manifest last) and `total` is the unit count.

```typescript
import { UploadStatus } from '@parity/bulletin-sdk'

await client
  .submit(estimate, src)
  .withCallback((ev) => {
    switch (ev.type) {
      case UploadStatus.ItemStarted:
        console.log(`[${ev.index + 1}/${ev.total}] started`)
        break
      case UploadStatus.ItemInBlock:
        console.log(`[${ev.index + 1}/${ev.total}] in block #${ev.blockNumber}`)
        break
      case UploadStatus.ItemFinalized:
        console.log(`[${ev.index + 1}/${ev.total}] finalized #${ev.blockNumber}, cid ${ev.cid}`)
        break
      case UploadStatus.ItemFailed:
        console.error(`[${ev.index + 1}/${ev.total}] failed:`, ev.error.message)
        break
    }
  })
  .send()
```

| Event | Value | Fields |
|---|---|---|
| `ItemStarted` | `"item_started"` | `index`, `total`, `cid` |
| `ItemInBlock` | `"item_in_block"` | `index`, `total`, `cid`, `blockHash`, `blockNumber`, `extrinsicIndex?` |
| `ItemFinalized` | `"item_finalized"` | `index`, `total`, `cid`, `blockHash`, `blockNumber`, `extrinsicIndex?` |
| `ItemFailed` | `"item_failed"` | `index`, `total`, `cid`, `error` |

`ItemFinalized` carries the `blockNumber` and `extrinsicIndex` you need to [renew](./renewal.md) the item later.

## Authorization Flow

```typescript
import { BulletinError, ErrorCode } from '@parity/bulletin-sdk'

try {
  await client.submit(await client.estimateUpload(src), src).send()
} catch (error) {
  if (!(error instanceof BulletinError)) throw error
  switch (error.code) {
    case ErrorCode.INSUFFICIENT_AUTHORIZATION:
      console.error('Need authorization:', error.recoveryHint)
      break
    case ErrorCode.AUTHORIZATION_FAILED:
      console.error('Authorization call failed:', error.message)
      break
    default:
      console.error(error.code, error.message)
  }
}
```

Use `.ensureAuthorized()` on the builder to fail fast with `INSUFFICIENT_AUTHORIZATION` before any bytes are submitted.

## Wrapping External Errors

```typescript
try {
  // some external operation
} catch (cause) {
  throw new BulletinError('Failed to process data', ErrorCode.TRANSACTION_FAILED, cause)
}
```

## Cross-SDK Consistency

Codes are aligned with the Rust SDK where they overlap (`EMPTY_DATA`, `TRANSACTION_FAILED`, `INSUFFICIENT_AUTHORIZATION`, …). Each SDK also has codes specific to its implementation.
