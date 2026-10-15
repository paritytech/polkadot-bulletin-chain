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
| `DATA_TOO_LARGE` | Exceeds 64 MiB (`FixedSizeChunker` / `prepareStoreChunked`) or `chunkingThreshold` (`BulletinPreparer.prepareStore`); never thrown by `estimateUpload` / `submit` | Reduce the data size or use a chunked upload |
| `CHUNK_TOO_LARGE` | Chunk exceeds 2 MiB | Reduce chunk size to 2 MiB or less |
| `INVALID_CHUNK_SIZE` | Chunk size is zero or negative | Use a chunk size between 1 byte and 2 MiB |
| `INVALID_CONFIG` | Invalid config (e.g. duplicate content hashes in one submit) | Check configuration parameters |
| `INVALID_CID` | Malformed CID | Verify CID format |
| `INVALID_HASH_ALGORITHM` | Unsupported hash algorithm | Use blake2b-256, sha2-256, or keccak-256 |
| `CID_CALCULATION_FAILED` | CID calculation failed | Verify data and hash algorithm |
| `DAG_ENCODING_FAILED` | DAG-PB encoding failed | Check chunk CIDs and data integrity |
| `CHUNKING_FAILED` | A chunk failed to prepare | Verify data integrity and chunker configuration |
| `MISSING_CHUNK` | A chunk was missing during reassembly | Ensure all chunks are present with contiguous indices starting from 0 |

### Authorization (non-retryable)

| Code | Description | Recovery Hint |
|---|---|---|
| `AUTHORIZATION_NOT_FOUND` | No unexpired authorization entry (checked only by `.ensureAuthorized()`) | Authorize the account or the content hash first, then retry |

### Capability (non-retryable)

| Code | Description | Recovery Hint |
|---|---|---|
| `UNSUPPORTED_OPERATION` | Called a signed path on a signer-less client, or a call the runtime doesn't have | This operation is not supported by this client or the connected runtime |

### Transaction & Upload

| Code | Retryable | Description |
|---|---|---|
| `TRANSACTION_INVALID` | No | The transaction pool rejected the transaction; `cause` is PAPI's `InvalidTxError`, whose `error` field holds the validity error |
| `TRANSACTION_FAILED` | Yes | A submitted transaction failed: dispatch error, dropped, or the subscription ended before inclusion |
| `NETWORK_ERROR` | Yes | A chain read of the SDK failed (`.ensureAuthorized()` queries, `skipExisting` lookups, renewal metadata) |
| `TIMEOUT` | Yes | Not included within the timeout window |
| `STORE_STALLED` | Yes | Submission stalled (pool saturation / reorg); the signed path retries internally |
| `HIJACK_BUDGET_EXCEEDED` | No | Too many nonce reassignments under concurrent same-account submission |
| `UPLOAD_INCOMPLETE` | No | Some items failed permanently and are not on chain; `cause.failedIndices` lists them |

Every call builder (`authorize*`, `refresh*`, `removeExpired*`, `renew`, `forceRenew`, `enableAutoRenew`, `disableAutoRenew`) reports a failed transaction as `TRANSACTION_INVALID` or `TRANSACTION_FAILED`, and the message names the call.

`error.retryable` is true for `TRANSACTION_FAILED`, `NETWORK_ERROR`, `TIMEOUT`, and `STORE_STALLED`. Note that a signed `submit()` already retries stalls internally (up to 3 times) before surfacing one; `.asUnsigned()` does not.

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
| `ItemInBlock` | `"item_in_block"` | `index`, `total`, `cid`, `blockHash`, `blockNumber`, `transactionIndex?` |
| `ItemFinalized` | `"item_finalized"` | `index`, `total`, `cid`, `blockHash`, `blockNumber`, `transactionIndex?` |
| `ItemFailed` | `"item_failed"` | `index`, `total`, `cid`, `error` |

`ItemFinalized` carries the `blockNumber` and `transactionIndex` you need to [renew](./renewal.md) the item later.

## Authorization Flow

```typescript
import { BulletinError, ErrorCode } from '@parity/bulletin-sdk'

try {
  await client.submit(await client.estimateUpload(src), src).ensureAuthorized().send()
} catch (error) {
  if (!(error instanceof BulletinError)) throw error
  if (error.code === ErrorCode.AUTHORIZATION_NOT_FOUND) {
    console.error('Need authorization:', error.recoveryHint)
  } else {
    console.error(error.code, error.message)
  }
}

try {
  await client.authorizeAccount(address, 10, 10_000_000n).send()
} catch (error) {
  if (error instanceof BulletinError && error.code === ErrorCode.TRANSACTION_INVALID) {
    console.error('Rejected by the transaction pool:', error.cause)
  } else {
    throw error
  }
}
```

`.ensureAuthorized()` on the builder fails fast with `AUTHORIZATION_NOT_FOUND` before any bytes are submitted; without it, the SDK does not check authorization before submitting. The authorization calls report failures like every other call: the transaction pool rejects a signer outside the authorizer set as an invalid transaction (`BadSigner`), so the error is `TRANSACTION_INVALID`.

## Wrapping External Errors

```typescript
try {
  // some external operation
} catch (cause) {
  throw new BulletinError('Failed to process data', ErrorCode.TRANSACTION_FAILED, cause)
}
```

## Cross-SDK Consistency

Both SDKs use the same error codes, the same retryable set and the same recovery hints: an `ErrorCode` value equals the string that the Rust `Error::code()` returns for the same failure. Three codes exist only in this SDK, because the failure cannot occur in the Rust SDK: `TIMEOUT` (`txTimeout`), `INVALID_HASH_ALGORITHM` (Rust uses an enum) and `MISSING_CHUNK` (only this SDK reassembles chunks).

A transaction pool rejection is `TRANSACTION_INVALID` in both SDKs. This SDK keeps PAPI's `InvalidTxError` as `cause`; the Rust SDK carries the custom validity code in `Error::TransactionInvalid { custom_code, .. }`.
