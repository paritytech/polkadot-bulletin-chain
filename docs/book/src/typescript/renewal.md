# Renewal

This guide shows how to renew stored data using the TypeScript SDK to extend the retention period.

> **Prerequisites**: Read [Data Renewal Concepts](../concepts/renewal.md) first to understand the renewal flow.

## Overview

Data stored on Bulletin Chain has a **retention period**. After this period, data may be pruned. To keep data available, you must **renew** it before expiration.

The renewal flow:
1. **Store** data → capture `blockNumber` and `extrinsicIndex` from the `ItemFinalized` event
2. **Track** that `(block, index)` reference
3. **Renew** before expiration with `client.renew({ block, index })` → capture the new reference
4. **Repeat** as needed

## The Complete Flow

```typescript
import { blobFromBytes, UploadStatus } from "@parity/bulletin-sdk";

// 1. STORE - submit and capture the storage reference from the event.
let blockNumber: number | undefined;
let index: number | undefined;
let cid: string | undefined;

const src = blobFromBytes(myData);
await client
  .submit(await client.estimateUpload(src), src)
  .withWaitFor("finalized")
  .withCallback((ev) => {
    if (ev.type === UploadStatus.ItemFinalized) {
      blockNumber = ev.blockNumber;
      index = ev.extrinsicIndex; // the storage slot for renew()
      cid = ev.cid.toString();
    }
  })
  .send();

console.log(`Stored ${cid} at block ${blockNumber}, index ${index}`);

// 2. SAVE these for later renewal.
saveToDatabase({ blockNumber, index, cid });

// 3. RENEW (later) - when approaching expiration.
const receipt = await client.renew(blockNumber!, index!).withWaitFor("finalized").send();

// 4. UPDATE the reference for the NEXT renewal. The SDK receipt gives the
//    renewal's block; read the `Renewed` event for the new slot index.
const block = await client.api.query.System.Number.getValue(); // or from receipt
// See "Tracking the renewed index" below.
```

### Tracking the renewed index

`client.renew()` resolves with a `TransactionReceipt` (block hash, tx hash, block number) but not the new slot index. To chain renewals, read the `Renewed` event from that block via `client.api`, or query `TransactionByContentHash` for the content's current `(block, index)`.

## Querying Retention Period

Check how long data is retained:

```typescript
// Get retention period (in blocks)
const retentionPeriod = await api.constants.TransactionStorage.RetentionPeriod();
console.log("Retention period:", retentionPeriod, "blocks");

// Get current block
const currentBlock = await api.query.System.Number.getValue();

// Calculate when data expires
const storedAtBlock = 1000; // Your stored block number
const expiresAtBlock = storedAtBlock + retentionPeriod;
const blocksRemaining = expiresAtBlock - currentBlock;

console.log(`Data expires at block ${expiresAtBlock}`);
console.log(`${blocksRemaining} blocks remaining`);
```

## Checking if Data Exists

Before renewing, verify the data still exists on-chain:

```typescript
// Query transaction info
const txInfo = await api.query.TransactionStorage.Transactions.getValue(blockNumber);

if (!txInfo || txInfo.length <= index) {
  console.log("Data not found - may have been pruned");
  return;
}

const info = txInfo[index];
console.log("Data exists:");
console.log("  Size:", info.size, "bytes");
console.log("  Content hash:", info.content_hash.asHex());
```

## Building a Renewal Tracker

For applications managing multiple stored items, create a tracker:

```typescript
interface StoredItem {
  cid: string;
  blockNumber: number;
  index: number;
  storedAt: Date;
}

class RenewalTracker {
  private items: Map<string, StoredItem> = new Map();

  add(cid: string, blockNumber: number, index: number) {
    this.items.set(cid, {
      cid,
      blockNumber,
      index,
      storedAt: new Date()
    });
  }

  update(cid: string, newBlockNumber: number, newIndex: number) {
    const item = this.items.get(cid);
    if (item) {
      item.blockNumber = newBlockNumber;
      item.index = newIndex;
    }
  }

  async getItemsNeedingRenewal(api: TypedApi, bufferBlocks: number = 100) {
    const currentBlock = await api.query.System.Number.getValue();
    const retentionPeriod = await api.constants.TransactionStorage.RetentionPeriod();

    const needsRenewal: StoredItem[] = [];

    for (const item of this.items.values()) {
      const expiresAt = item.blockNumber + retentionPeriod;
      if (currentBlock + bufferBlocks >= expiresAt) {
        needsRenewal.push(item);
      }
    }

    return needsRenewal;
  }
}

// Usage
const tracker = new RenewalTracker();

// After storing
tracker.add(cid.toString(), blockNumber, index);

// Check what needs renewal
const toRenew = await tracker.getItemsNeedingRenewal(api);
for (const item of toRenew) {
  console.log(`Need to renew: ${item.cid}`);
}
```

## Batch Renewal

Renew multiple items efficiently:

```typescript
async function renewBatch(
  api: TypedApi,
  signer: PolkadotSigner,
  items: Array<{ blockNumber: number; index: number }>
) {
  // Create renewal calls
  const calls = items.map(item =>
    api.tx.TransactionStorage.renew({
      block: item.blockNumber,
      index: item.index
    }).decodedCall
  );

  // Batch them together
  const batchTx = api.tx.Utility.batch_all({ calls });

  const result = await batchTx.signAndSubmit(signer);

  // Extract all Renewed events
  const renewedEvents = result.events.filter(
    e => e.type === "TransactionStorage" && e.value.type === "Renewed"
  );

  return renewedEvents.map((event, i) => ({
    originalBlock: items[i].blockNumber,
    originalIndex: items[i].index,
    newBlock: result.block.number,
    newIndex: event.value.value.index
  }));
}
```

## Authorization for Renewal

Renewal consumes authorization just like storing:

```typescript
// Check you have enough authorization for renewal
const auth = await api.query.TransactionStorage.Authorizations.getValue({
  type: "Account",
  value: myAddress
});

// Each renewal needs:
// - 1 transaction
// - Same number of bytes as the original data

const txInfo = await api.query.TransactionStorage.Transactions.getValue(blockNumber);
const dataSize = txInfo[index].size;

if (!auth || auth.extent.transactions < 1 || auth.extent.bytes < dataSize) {
  console.log("Insufficient authorization for renewal");
  return;
}
```

## Complete Example: Store and Schedule Renewal

```typescript
import { getWsProvider } from "polkadot-api/ws-provider/node";
import { bulletin } from "@polkadot-api/descriptors";
import { BulletinClient, blobFromBytes, UploadStatus } from "@parity/bulletin-sdk";

const client = new BulletinClient({
  providers: () => [getWsProvider("wss://paseo-bulletin-next-rpc.polkadot.io")],
  uploadSigner: signer,
  descriptor: bulletin,
});

async function storeAndTrackRenewal() {
  // 1. Store and capture the (block, index) reference.
  const data = new TextEncoder().encode("Important data to keep!");
  const src = blobFromBytes(data);
  let ref: { blockNumber: number; index?: number; cid: string } | undefined;
  await client
    .submit(await client.estimateUpload(src), src)
    .withWaitFor("finalized")
    .withCallback((ev) => {
      if (ev.type === UploadStatus.ItemFinalized) {
        ref = { blockNumber: ev.blockNumber, index: ev.extrinsicIndex, cid: ev.cid.toString() };
      }
    })
    .send();

  // 2. Compute when to renew (10% buffer before expiry).
  const retentionPeriod = await client.api.constants.TransactionStorage.RetentionPeriod();
  const expiresAt = ref!.blockNumber + retentionPeriod;
  const renewAtBlock = expiresAt - Math.floor(retentionPeriod * 0.1);

  // 3. Save for later (in your app's database).
  return { ...ref!, renewAtBlock, expiresAt };
}

async function performRenewal(blockNumber: number, index: number) {
  const receipt = await client.renew({ block: blockNumber, index }).withWaitFor("finalized").send();
  console.log(`Renewed in block ${receipt.blockNumber}`);
  // Read the Renewed event from that block for the new slot index (see above).
}
```

## Error Handling

`client.renew(...).send()` throws a `BulletinError`; inspect `error.code` / `error.message` for the on-chain reason (data pruned/already renewed, insufficient authorization, proofs not yet checked).

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
