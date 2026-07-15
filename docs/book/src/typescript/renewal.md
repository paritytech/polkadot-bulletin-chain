# Renewal

This guide shows how to renew stored data using the TypeScript SDK to extend the retention period.

> **Prerequisites**: Read [Data Renewal Concepts](../concepts/renewal.md) first to understand the renewal flow.

> **Note**: `client.renew(ref)` takes either a `{ block, index }` position or a 32-byte content hash (`Uint8Array`) — the SDK infers the on-chain `TransactionRef` variant from the shape. It schedules a one-shot renewal that fires once when the data reaches its retention boundary; for immediate renewal use `client.forceRenew(ref)`. On chains still running the pre-`TransactionRef` runtime, positions fall back to the legacy `renew` extrinsic (which renews immediately); content hashes and `forceRenew` error there. Recurring `enable_auto_renew` is not exposed by the SDK; call it via a raw PAPI transaction against the live runtime if you need it (see [Raw Runtime Renewal](#raw-runtime-renewal)).

## Using the SDK Client

`AsyncBulletinClient` wraps PAPI and returns builders you finish with `.send()`.

```typescript
import { AsyncBulletinClient } from "@parity/bulletin-sdk";
import { createClient } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws";
import { bulletin } from "@polkadot-api/descriptors";

const papiClient = createClient(getWsProvider("wss://paseo-bulletin-rpc.polkadot.io"));
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

For applications managing multiple stored items, track them and renew before expiry:

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

Bypassing the SDK client, a raw PAPI transaction targets the **current** runtime, where `renew`/`force_renew` take an `entry: TransactionRef` and `enable_auto_renew` takes a `content_hash`:

```typescript
// One-shot scheduled renewal
api.tx.TransactionStorage.renew({
  entry: { type: "Position", value: { block, index } },
});

// Immediate renewal (emits Renewed with a new index)
api.tx.TransactionStorage.force_renew({
  entry: { type: "Position", value: { block, index } },
});

// Recurring auto-renewal (takes a content hash directly, not an `entry`)
api.tx.TransactionStorage.enable_auto_renew({ content_hash: contentHash });
```

The raw `store` extrinsic takes only `{ data }`; use `store_with_cid_config` for a non-default CID:

```typescript
api.tx.TransactionStorage.store({ data: myData });

api.tx.TransactionStorage.store_with_cid_config({
  cid: { codec: 0x55n, hashing: { type: "Blake2b256" } },
  data: myData,
});
```

## Authorization for Renewal

Renewal consumes authorization just like storing — one transaction plus the data's byte size. Ensure the account has enough authorized capacity before renewing (see [Authorization](./authorization.md)).

## Error Handling

```typescript
try {
  await client.renew({ block: blockNumber, index }).send();
} catch (error) {
  if (error.message.includes("RenewedNotFound")) {
    console.log("Data not found - may have been pruned");
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
