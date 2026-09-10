# Renewal

Extending the retention of stored data with the TypeScript SDK.

> **Prerequisites**: Read [Data Renewal Concepts](../concepts/renewal.md) first to understand the renewal flow.

> **Note**: `client.renew(ref)` takes a `{ block, index }` position or a 32-byte content hash (`Uint8Array`) — the SDK infers the `TransactionRef` variant from the shape. It schedules a one-shot renewal that fires at the retention boundary; `client.forceRenew(ref)` renews immediately. Recurring `enable_auto_renew` is not exposed by the SDK — use a [raw PAPI transaction](#raw-runtime-renewal).
>
> On chains still running the pre-`TransactionRef` runtime, positions fall back to the legacy `renew` extrinsic (which renews immediately); content hashes and `forceRenew` error there.

## Using the SDK Client

`AsyncBulletinClient` wraps PAPI and returns builders you finish with `.send()`.

```typescript
import { AsyncBulletinClient } from "@parity/bulletin-sdk";
import { createClient } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws";
import { bulletin } from "@polkadot-api/descriptors";

const papiClient = createClient(getWsProvider("wss://paseo-bulletin-next-rpc.polkadot.io"));
const api = papiClient.getTypedApi(bulletin);
const client = new AsyncBulletinClient(api, signer, papiClient.submit);

// 1. STORE - returns a StoreResult with the reference you need to renew
const result = await client.store(myData).send();
const blockNumber = result.blockNumber;   // block the store landed in
const index = result.extrinsicIndex;      // from the Stored event

// 2. RENEW (later) - before the retention period expires
await client.renew({ block: blockNumber, index }).send();
```

`store().send()` returns a `StoreResult` (`cid`, `size`, `blockNumber`, `extrinsicIndex`).
`renew(ref).send()` returns a `TransactionReceipt` (`blockHash`, `txHash`, `blockNumber`).

## Querying the Retention Period

`RetentionPeriod` is an on-chain storage value (default 201,600 blocks, ~14 days at 6s/block), not a constant — read it from storage:

```typescript
const retentionPeriod = await api.query.TransactionStorage.RetentionPeriod.getValue();
const currentBlock = await api.query.System.Number.getValue();

const storedAtBlock = 1000; // your stored block number
const expiresAtBlock = storedAtBlock + retentionPeriod;
const blocksRemaining = expiresAtBlock - currentBlock;

console.log(`Data expires at block ${expiresAtBlock} (${blocksRemaining} blocks remaining)`);
```

## Building a Renewal Tracker

For applications managing multiple stored items, track them and renew before expiry. `renew` registers at most one scheduled renewal per content hash — a second call before it fires rejects with `RenewalAlreadyEnabled`, so drop entries once scheduled:

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

  async getItemsNeedingRenewal(api: TypedApi, bufferBlocks = 100) {
    const currentBlock = await api.query.System.Number.getValue();
    const retentionPeriod = await api.query.TransactionStorage.RetentionPeriod.getValue();

    return [...this.items.values()].filter(
      (item) => currentBlock + bufferBlocks >= item.blockNumber + retentionPeriod,
    );
  }
}

// Usage
const tracker = new RenewalTracker();
tracker.add(result.cid.toString(), result.blockNumber, result.extrinsicIndex);

for (const item of await tracker.getItemsNeedingRenewal(api)) {
  await client.renew({ block: item.blockNumber, index: item.index }).send();
}
```

## Raw Runtime Renewal

Against the **current** runtime, bypassing the SDK client: `renew` / `force_renew` take an `entry: TransactionRef`, `enable_auto_renew` a `content_hash`:

```typescript
// One-shot scheduled renewal
api.tx.TransactionStorage.renew({
  entry: { type: "Position", value: { block, index } },
});

// Immediate renewal (emits Renewed with a new index)
api.tx.TransactionStorage.force_renew({
  entry: { type: "Position", value: { block, index } },
});

// Recurring auto-renewal (takes the content hash directly, not an `entry`;
// fixed-size hashes are passed as 0x-prefixed hex)
api.tx.TransactionStorage.enable_auto_renew({ content_hash: contentHashHex });
```

`disable_auto_renew` is refused while the registration is still prepaid (`CannotDisablePrepaidAutoRenewal`) — it only succeeds after the first cycle has consumed the prepayment.

The raw `store` extrinsic takes only `{ data }`; use `store_with_cid_config` for a non-default CID:

```typescript
api.tx.TransactionStorage.store({ data: myData });

api.tx.TransactionStorage.store_with_cid_config({
  cid: { codec: 0x55n, hashing: { type: "Blake2b256" } },
  data: myData,
});
```

## Authorization for Renewal

Renewal is accounted differently from storing. Going over budget on a store only lowers its priority; a renewal charges the data size against `bytes_permanent` (the account's renew quota) plus one transaction unit, and against the chain-wide permanent-storage **hard cap**. Exceeding either limit **rejects** the renewal (see [Authorization](./authorization.md)).

## Error Handling

```typescript
try {
  await client.renew({ block: blockNumber, index }).send();
} catch (error) {
  if (error.message.includes("RenewedNotFound")) {
    console.log("Data not found - may have been pruned");
  } else if (error.message.includes("RenewalAlreadyEnabled")) {
    console.log("A renewal is already scheduled for this data");
  } else if (error.message.includes("AuthorizationNotFound")) {
    console.log("Insufficient authorization - request more via Faucet");
  } else {
    throw error;
  }
}
```

## Next Steps

- [Authorization](./authorization.md) - Manage authorization for renewals
- [Basic Storage](./basic-storage.md) - Store data
- [Data Renewal Concepts](../concepts/renewal.md) - Understand the renewal model
