# Renewal

Extending the retention of stored data with the TypeScript SDK.

> **Prerequisites**: Read [Data Renewal Concepts](../concepts/renewal.md) first to understand the renewal flow.

> **Note**: `client.renew(ref)` takes a `{ block, index }` position or a 32-byte content hash (`Uint8Array`) — the SDK infers the `TransactionRef` variant from the shape. It schedules a one-shot renewal that fires at the retention boundary; `client.forceRenew(ref)` renews immediately, and `client.enableAutoRenew(contentHash)` renews every period.
>
> On chains still running the pre-`TransactionRef` runtime, positions fall back to the legacy `renew` extrinsic (which renews immediately); content hashes and `forceRenew` error there. Runtimes older than the auto-renew calls reject `enableAutoRenew` / `disableAutoRenew` with `UNSUPPORTED_OPERATION`.

## Using the SDK Client

The `ItemFinalized` upload event carries the `(blockNumber, transactionIndex)` reference that `renew` takes.

```typescript
import { BulletinClient, blobFromBytes, UploadStatus } from "@parity/bulletin-sdk";
import { getWsProvider } from "polkadot-api/ws";
import { bulletin } from "@polkadot-api/descriptors";

const client = new BulletinClient({
  providers: () => [getWsProvider("wss://paseo-bulletin-next-rpc.polkadot.io")],
  uploadSigner: signer,
  descriptor: bulletin,
});

// 1. STORE - capture the reference from the ItemFinalized event
let blockNumber: number | undefined;
let index: number | undefined;
const src = blobFromBytes(myData);
const { cids } = await client
  .submit(await client.estimateUpload(src), src)
  .withWaitFor("finalized")
  .withCallback((ev) => {
    if (ev.type === UploadStatus.ItemFinalized) {
      blockNumber = ev.blockNumber;
      index = ev.transactionIndex;
    }
  })
  .send();

// 2. RENEW (later) - before the retention period expires
await client.renew({ block: blockNumber!, index: index! }).send();
```

`renew(ref).send()` returns a `TransactionReceipt` (`blockHash`, `txHash`, `blockNumber`).

## Recurring Auto-Renewal

`enableAutoRenew` takes the 32-byte content hash, not a position. The first cycle is prepaid at registration; later cycles charge the owner's authorization and the registration is dropped with a `RenewalFailed` event once the quota runs out.

```typescript
const contentHash = cids[0].multihash.digest; // Uint8Array

await client.enableAutoRenew(contentHash).send();

// Only succeeds after the prepaid cycle has fired.
await client.disableAutoRenew(contentHash).send();
```

A content hash carries at most one registration, so `renew` and `enableAutoRenew` are mutually exclusive. The transaction pool rejects these calls as invalid transactions before they reach a block:

| Call | Rejected when | Pool error |
|------|---------------|------------|
| `renew`, `enableAutoRenew` | a renewal is already registered for the content hash | custom 11, `RENEWAL_ALREADY_ENABLED` |
| `disableAutoRenew` | no renewal is registered for the content hash | custom 9, `AUTO_RENEWAL_NOT_ENABLED` |
| `disableAutoRenew` | the caller does not own the registration | custom 10, `NOT_AUTO_RENEWAL_OWNER` |
| `disableAutoRenew` | the next cycle is still prepaid | custom 12, `CANNOT_DISABLE_PREPAID_AUTO_RENEWAL` |

`send()` then rejects with a `BulletinError` (`TRANSACTION_INVALID`) whose `cause` is PAPI's `InvalidTxError`; its `error` field holds `{ type: "Invalid", value: { type: "Custom", value: <code> } }`.

## Querying the Retention Period

`RetentionPeriod` is an on-chain storage value (default 201,600 blocks, ~14 days at 6s/block), not a constant — read it from storage. `client.api` is statically typed as the SDK's minimal `BulletinTypedApi`; when the client was built with your descriptor, cast it to the full typed API:

```typescript
import type { TypedApi } from "polkadot-api";
import { bulletin } from "@polkadot-api/descriptors";

const api = client.api as unknown as TypedApi<typeof bulletin>;
const retentionPeriod = await api.query.TransactionStorage.RetentionPeriod.getValue();
const currentBlock = await api.query.System.Number.getValue();

const storedAtBlock = 1000; // your stored block number
const expiresAtBlock = storedAtBlock + retentionPeriod;
const blocksRemaining = expiresAtBlock - currentBlock;

console.log(`Data expires at block ${expiresAtBlock} (${blocksRemaining} blocks remaining)`);
```

## Building a Renewal Tracker

For applications managing multiple stored items, track them and renew before expiry. `renew` registers at most one scheduled renewal per content hash — the pool rejects a second call before it fires (custom error 11, `RENEWAL_ALREADY_ENABLED`), so drop entries once scheduled:

```typescript
interface StoredItem {
  cid: string;
  blockNumber: number;
  index: number;
}

class RenewalTracker {
  private items = new Map<string, StoredItem>();

  add(cid: string, blockNumber: number, index: number) {
    this.items.set(cid, { cid, blockNumber, index });
  }

  async getItemsNeedingRenewal(api: TypedApi<typeof bulletin>, bufferBlocks = 100) {
    const currentBlock = await api.query.System.Number.getValue();
    const retentionPeriod = await api.query.TransactionStorage.RetentionPeriod.getValue();

    return [...this.items.values()].filter(
      (item) => currentBlock + bufferBlocks >= item.blockNumber + retentionPeriod,
    );
  }
}

// Usage
const tracker = new RenewalTracker();
tracker.add(cids[0].toString(), blockNumber!, index!);

for (const item of await tracker.getItemsNeedingRenewal(api)) {
  await client.renew({ block: item.blockNumber, index: item.index }).send();
}
```

## Raw Runtime Renewal

Against the **current** runtime, bypassing the SDK client: `renew` / `force_renew` take an `entry: TransactionRef`, `enable_auto_renew` a `content_hash`:

```typescript
// One-shot scheduled renewal
api.tx.DataRenewal.renew({
  entry: { type: "Position", value: { block, index } },
});

// Immediate renewal (emits Renewed with a new index)
api.tx.DataRenewal.force_renew({
  entry: { type: "Position", value: { block, index } },
});

// Recurring auto-renewal (takes the content hash directly, not an `entry`;
// fixed-size hashes are passed as 0x-prefixed hex)
api.tx.DataRenewal.enable_auto_renew({ content_hash: contentHashHex });
```

Pre-split runtimes host these calls on `api.tx.TransactionStorage`; `client.renew` / `forceRenew` / `enableAutoRenew` resolve the pallet for you.

While the registration is still prepaid, the pool rejects `disable_auto_renew` as an invalid transaction (custom error 12, `CANNOT_DISABLE_PREPAID_AUTO_RENEWAL`); it only succeeds after the first cycle has consumed the prepayment.

The raw `store` extrinsic takes only `{ data }`; use `store_with_cid_config` for a non-default CID:

```typescript
api.tx.TransactionStorage.store({ data: myData });

api.tx.TransactionStorage.store_with_cid_config({
  cid: { codec: 0x55n, hashing: { type: "Blake2b256", value: undefined } },
  data: myData,
});
```

## Authorization for Renewal

Renewal is accounted differently from storing. Going over budget on a store only lowers its priority; a renewal charges one transaction unit and adds the data size to the account's `bytes_permanent` counter, which may not exceed its `bytes_allowance`, and to the chain-wide `PermanentStorageUsed`, which may not exceed `MaxPermanentStorageSize`. Exceeding either limit **rejects** the renewal (see [Authorization](./authorization.md)).

## Error Handling

`client.renew(...).send()` throws a `BulletinError`; inspect `error.code` / `error.message` for the on-chain reason (data pruned, renewal already scheduled, insufficient authorization).

```typescript
import { BulletinError } from "@parity/bulletin-sdk";

try {
  await client.renew({ block: blockNumber, index }).send();
} catch (error) {
  if (error instanceof BulletinError) {
    console.error(error.code, error.message);
  } else {
    throw error;
  }
}
```

## Next Steps

- [Authorization](./authorization.md) - Manage authorization for renewals
- [Basic Storage](./basic-storage.md) - Store data
- [Data Renewal Concepts](../concepts/renewal.md) - Understand the renewal model
