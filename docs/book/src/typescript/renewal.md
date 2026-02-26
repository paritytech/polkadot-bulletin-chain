# Renewal

This guide shows how to renew stored data using the TypeScript SDK to extend the retention period.

> **Prerequisites**: Read [Data Renewal Concepts](../concepts/renewal.md) first to understand the renewal flow.

## Overview

Data stored on Bulletin Chain has a **retention period**. After this period, data may be pruned. To keep data available, you must **renew** it before expiration.

The renewal flow:
1. **Store** data → receive `blockNumber` and `index`
2. **Track** the block number and index
3. **Renew** before expiration → receive new `blockNumber` and `index`
4. **Repeat** as needed

## The Complete Flow

```typescript
import { createClient, Binary } from "polkadot-api";
import { bulletin } from "@polkadot-api/descriptors";

// 1. STORE - Submit data and track the result
const storeTx = api.tx.TransactionStorage.store({
  data: Binary.fromBytes(myData),
  cid_config: { codec: 0x55, hashing: "Blake2b256" }
});

const storeResult = await storeTx.signAndSubmit(signer);

// 2. EXTRACT - Get block number and index from events
const storedEvent = storeResult.events.find(
  e => e.type === "TransactionStorage" && e.value.type === "Stored"
);
const blockNumber = storeResult.block.number;
const index = storedEvent.value.value.index;

console.log(`Stored at block ${blockNumber}, index ${index}`);

// 3. SAVE - Store these for later renewal
saveToDatabase({ blockNumber, index, cid: myCid });

// 4. RENEW (later) - When approaching expiration
const renewTx = api.tx.TransactionStorage.renew({
  block: savedBlockNumber,
  index: savedIndex
});

const renewResult = await renewTx.signAndSubmit(signer);

// 5. UPDATE - Get new block/index for next renewal
const renewedEvent = renewResult.events.find(
  e => e.type === "TransactionStorage" && e.value.type === "Renewed"
);
const newBlockNumber = renewResult.block.number;
const newIndex = renewedEvent.value.value.index;

// Save the NEW values for next renewal!
updateDatabase({ blockNumber: newBlockNumber, index: newIndex });
```

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
import { createClient, Binary } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws-provider/node";
import { bulletin } from "@polkadot-api/descriptors";
import { calculateCid, CidCodec, HashAlgorithm } from "@bulletin/sdk";

async function storeAndTrackRenewal() {
  const client = createClient(getWsProvider("wss://paseo-bulletin-rpc.polkadot.io"));
  const api = client.getTypedApi(bulletin);

  // 1. Store data
  const data = new TextEncoder().encode("Important data to keep!");
  const cid = await calculateCid(data, CidCodec.Raw, HashAlgorithm.Blake2b256);

  const storeTx = api.tx.TransactionStorage.store({
    data: Binary.fromBytes(data),
    cid_config: { codec: 0x55, hashing: "Blake2b256" }
  });

  const storeResult = await storeTx.signAndSubmit(signer);

  // 2. Extract storage reference
  const storedEvent = storeResult.events.find(
    e => e.type === "TransactionStorage" && e.value.type === "Stored"
  );
  const blockNumber = storeResult.block.number;
  const index = storedEvent.value.value.index;

  console.log(`Stored CID ${cid} at block ${blockNumber}, index ${index}`);

  // 3. Calculate when to renew
  const retentionPeriod = await api.constants.TransactionStorage.RetentionPeriod();
  const expiresAt = blockNumber + retentionPeriod;

  // Renew with 10% buffer before expiration
  const renewAtBlock = expiresAt - Math.floor(retentionPeriod * 0.1);

  console.log(`Schedule renewal before block ${renewAtBlock}`);
  console.log(`Data expires at block ${expiresAt}`);

  // 4. Save for later (in your app's database)
  return {
    cid: cid.toString(),
    blockNumber,
    index,
    renewAtBlock,
    expiresAt
  };
}

async function performRenewal(blockNumber: number, index: number) {
  const renewTx = api.tx.TransactionStorage.renew({
    block: blockNumber,
    index: index
  });

  const result = await renewTx.signAndSubmit(signer);

  const renewedEvent = result.events.find(
    e => e.type === "TransactionStorage" && e.value.type === "Renewed"
  );

  const newBlockNumber = result.block.number;
  const newIndex = renewedEvent.value.value.index;

  console.log(`Renewed! New reference: block ${newBlockNumber}, index ${newIndex}`);

  return { newBlockNumber, newIndex };
}
```

## Error Handling

Common renewal errors:

```typescript
try {
  const result = await renewTx.signAndSubmit(signer);
} catch (error) {
  if (error.message.includes("RenewedNotFound")) {
    console.log("Data not found - may have been pruned or already renewed");
  } else if (error.message.includes("Unauthorized")) {
    console.log("Insufficient authorization - request more via Faucet");
  } else if (error.message.includes("ProofNotChecked")) {
    console.log("Chain hasn't verified storage proofs yet - try next block");
  } else {
    throw error;
  }
}
```

## Next Steps

- [Authorization](./authorization.md) - Manage authorization for renewals
- [Basic Storage](./basic-storage.md) - Store data
- [Data Renewal Concepts](../concepts/renewal.md) - Understand the renewal model
