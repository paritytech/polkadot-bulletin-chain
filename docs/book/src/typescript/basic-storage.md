# Basic Storage

Storing data is always two steps: **estimate**, then **submit**.

- `estimateUpload(input)` hashes the data offline and returns a `StreamEstimate` — the CIDs, the number of `store` transactions, and the total bytes. Use it to preview cost or size an authorization.
- `submit(estimate, source)` stores it, fetching bytes from the `source` on demand and resolving with one CID per stored unit.

`input` is either a list of `UploadItem`s (`{ data, codec?, hashAlgo? }`) or a `BlobSource` (a file/blob, chunked automatically). Wrap raw bytes with `blobFromItems` (items) or `blobFromBytes` (a single blob).

## Quick Start

```typescript
import { BulletinClient, blobFromBytes } from '@parity/bulletin-sdk';
import { getWsProvider } from 'polkadot-api/ws-provider/node';

// The SDK owns its PAPI connection.
const client = new BulletinClient({
  providers: () => [getWsProvider('ws://localhost:9944')],
  uploadSigner: signer,
});

const data = new TextEncoder().encode('Hello, Bulletin Chain!');
const src = blobFromBytes(data);

const { cids } = await client.submit(await client.estimateUpload(src), src).send();
console.log('Stored with CID:', cids[cids.length - 1].toString());
```

## Storing Items

For one or more discrete values (each stored as its own `store` extrinsic), pass a list and source it with `blobFromItems`:

```typescript
import { blobFromItems, CidCodec, HashAlgorithm } from '@parity/bulletin-sdk';

const items = [
  { data: new TextEncoder().encode('first') },
  // Per-item CID config — defaults are Raw + Blake2b-256.
  { data: jsonBytes, codec: CidCodec.DagCbor, hashAlgo: HashAlgorithm.Sha2_256 },
];

const { cids } = await client
  .submit(await client.estimateUpload(items), blobFromItems(items))
  .send();

// cids[i] is the CID of items[i].
```

## Builder Options

`submit()` returns a builder. Chain options before `send()`:

```typescript
const { cids } = await client
  .submit(estimate, src)
  .withWaitFor('finalized')        // 'in_block' (default) | 'finalized'
  .ensureAuthorized()              // pre-flight: fail fast if not authorized
  .withCallback((ev) => console.log(ev.type, ev.index))
  .send();
```

- `.asUnsigned()` — submit preimage-authorized bare extrinsics (no signer). See [Authorization](./authorization.md).
- `.ensureAuthorized()` — verifies an `Authorizations` entry exists and isn't expired before submitting.

## Result

`send()` resolves with `{ cids: CID[] }`, one per stored unit in source order. For a chunked file the last entry is the manifest root; for a single value or a one-item list it is that value's CID.

## Estimating First

`estimateUpload` is offline and cheap — call it before paying:

```typescript
const estimate = await client.estimateUpload(src);
console.log(estimate.transactions, 'txs,', estimate.bytes, 'bytes');
console.log('CIDs:', estimate.plan.chunkCids.map((c) => c.toString()));
```

Pass `{ skipExisting: true }` to also query the chain and drop items already stored; the returned estimate carries that skip set into `submit()`.

## Error Handling

```typescript
import { BulletinError, ErrorCode } from '@parity/bulletin-sdk';

try {
  await client.submit(await client.estimateUpload(src), src).send();
} catch (error) {
  if (error instanceof BulletinError) {
    if (error.code === ErrorCode.INSUFFICIENT_AUTHORIZATION) {
      console.error('Authorize the account first:', error.recoveryHint);
    } else if (error.retryable) {
      console.error('Transient — retry may help:', error.message);
    } else {
      console.error(error.code, error.message);
    }
  } else {
    throw error;
  }
}
```

See [Error Handling](./error-handling.md) for the full reference.

## Testing Without a Node

`MockBulletinClient` implements the same interface and computes real CIDs without touching a chain:

```typescript
import { MockBulletinClient, blobFromItems } from '@parity/bulletin-sdk';

const client = new MockBulletinClient();
const items = [{ data: new TextEncoder().encode('test') }];
const { cids } = await client
  .submit(await client.estimateUpload(items), blobFromItems(items))
  .send();

expect(client.getOperations()).toHaveLength(1);
```
