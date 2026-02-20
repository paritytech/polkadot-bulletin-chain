# Data Renewal

Data stored on Bulletin Chain has a **retention period** after which it may be pruned. To keep data available, you must **renew** it before the retention period expires.

## How Retention Works

```
Block N: Data stored (Stored event with index)
         ↓
         ... retention period passes ...
         ↓
Block N + RetentionPeriod: Data eligible for pruning
```

- **Retention Period**: Configurable chain parameter (check `transactionStorage.retentionPeriod()`)
- **Pruning**: Validators may remove data after the retention period
- **IPFS Caching**: Even if pruned from chain, IPFS nodes that cached the data may still serve it

## When to Renew

Renew your data when:
- You need **guaranteed on-chain availability** beyond the retention period
- You want validators to continue providing **storage proofs** for your data
- You're building applications that depend on **chain-level data guarantees**

You don't need to renew if:
- The data only needs to exist temporarily
- You've pinned the data to external IPFS nodes
- The retention period is sufficient for your use case

## How to Renew

The `renew` extrinsic takes two parameters:
- `block`: The block number where the data was last stored or renewed
- `index`: The transaction index within that block (from `Stored` or `Renewed` event)

### Using the SDK (Recommended)

The Rust SDK provides helpers for renewal operations:

```rust
use bulletin_sdk_rust::prelude::*;

// After storing data, you received block=100, index=5 from the Stored event
let storage_ref = StorageRef::new(100, 5);

// Prepare renewal
let client = BulletinClient::new();
let renewal = client.prepare_renew(storage_ref)?;

// Submit via subxt:
// api.tx().transaction_storage().renew(renewal.block, renewal.index)
```

The SDK also provides a `RenewalTracker` for managing multiple entries:

```rust
use bulletin_sdk_rust::prelude::*;

let mut tracker = RenewalTracker::new();

// Track stored data (retention_period from chain constants)
let retention_period = 100_800; // blocks
tracker.track(
    StorageRef::new(100, 0),
    content_hash.to_vec(),
    data_size,
    retention_period,
);

// Check what's expiring soon (within 1000 blocks)
let current_block = 100_500;
let expiring = tracker.expiring_before(current_block + 1000);

for entry in expiring {
    // Renew each expiring entry
    let renewal = client.prepare_renew(entry.storage_ref)?;
    // Submit renewal...

    // After successful renewal, update tracker with new block/index
    tracker.update_after_renewal(
        entry.storage_ref,
        StorageRef::new(new_block, new_index),
        retention_period,
    );
}
```

### Finding Block and Index

When you store data, the chain emits a `Stored` event:

```typescript
// TypeScript - Listen for Stored event
const result = await api.tx.transactionStorage
  .store(data)
  .signAndSend(account);

// The event contains:
// - index: transaction index in the block
// - contentHash: hash of the stored data

// Save the block number and index for renewal
const blockNumber = result.blockNumber;
const index = result.events.find(e =>
  e.event.section === 'transactionStorage' &&
  e.event.method === 'Stored'
).event.data.index;
```

### Submitting Renewal

```typescript
// TypeScript - Renew storage
const renewTx = api.tx.transactionStorage.renew(blockNumber, index);
await renewTx.signAndSend(account);

// After renewal, a new Renewed event is emitted with a new index
// Use the NEW block number and index for the next renewal
```

```rust
// Rust - Renew storage (using subxt)
let renew_tx = bulletin::tx()
    .transaction_storage()
    .renew(block_number, index);

let result = api
    .tx()
    .sign_and_submit_then_watch_default(&renew_tx, &signer)
    .await?;

// Extract new block number and index from Renewed event
```

### Direct RPC (PAPI)

```typescript
import { createClient } from "polkadot-api";
import { bulletin } from "@polkadot-api/descriptors";

const client = createClient(/* provider */);
const api = client.getTypedApi(bulletin);

// Renew
const tx = api.tx.TransactionStorage.renew({
  block: originalBlockNumber,
  index: originalIndex,
});

const result = await tx.signAndSubmit(signer);
```

## Authorization for Renewal

Like `store`, the `renew` extrinsic requires **authorization**:

- The account must have sufficient authorized bytes/transactions
- Authorization is consumed by the renewal (based on data size)
- Pre-authorize enough capacity if you plan multiple renewals

```typescript
// Estimate authorization needed for renewal
// (same as original storage - based on data size)
const dataSize = /* size of the stored data */;
const authNeeded = estimateAuthorization(dataSize);

// Authorize if needed
await api.tx.transactionStorage
  .authorizeAccount(account, authNeeded.transactions, authNeeded.bytes)
  .signAndSend(sudoOrAuthorizer);
```

## Tracking Renewal Schedule

For applications that need to renew data, implement a tracking system:

```typescript
interface StoredData {
  cid: string;
  blockNumber: number;
  index: number;
  storedAt: Date;
  expiresAt: Date; // blockNumber + retentionPeriod
  dataSize: number;
}

class RenewalTracker {
  private stored: Map<string, StoredData> = new Map();

  async trackStore(cid: string, blockNumber: number, index: number, size: number) {
    const retentionPeriod = await api.constants.transactionStorage.retentionPeriod();
    const blockTime = 6; // seconds per block (approximate)

    this.stored.set(cid, {
      cid,
      blockNumber,
      index,
      storedAt: new Date(),
      expiresAt: new Date(Date.now() + retentionPeriod * blockTime * 1000),
      dataSize: size,
    });
  }

  getExpiringWithin(hours: number): StoredData[] {
    const threshold = new Date(Date.now() + hours * 60 * 60 * 1000);
    return Array.from(this.stored.values())
      .filter(d => d.expiresAt <= threshold);
  }

  async renewData(cid: string): Promise<void> {
    const data = this.stored.get(cid);
    if (!data) throw new Error("Unknown CID");

    const result = await api.tx.transactionStorage
      .renew(data.blockNumber, data.index)
      .signAndSend(account);

    // Update tracking with new block/index
    const newBlock = result.blockNumber;
    const newIndex = /* extract from Renewed event */;

    await this.trackStore(cid, newBlock, newIndex, data.dataSize);
  }
}
```

## Renewal Chain

Each renewal creates a new record. You must use the **most recent** block and index:

```
Block 100: store() -> Stored { index: 0 }
           ↓
Block 500: renew(100, 0) -> Renewed { index: 2 }
           ↓
Block 900: renew(500, 2) -> Renewed { index: 1 }
           ↓
Block 1300: renew(900, 1) -> Renewed { index: 0 }
```

Always track the latest block/index pair for future renewals.

## Cost Considerations

Renewal costs are similar to storage costs:
- **Transaction fee**: Standard Bulletin Chain transaction fee
- **Authorization**: Must have pre-authorized capacity
- **No additional storage fee**: You're not storing new data, just extending retention

For long-term storage, consider:
- Batching renewals for multiple CIDs
- Setting up automated renewal before expiration
- Monitoring authorization balance

## Error Handling

```typescript
try {
  await api.tx.transactionStorage.renew(block, index).signAndSend(account);
} catch (error) {
  if (error.message.includes("RenewedNotFound")) {
    // The original transaction doesn't exist at that block/index
    // Data may have already been pruned or block/index is wrong
    console.error("Cannot find original storage transaction");
  } else if (error.message.includes("InsufficientAuthorization")) {
    // Need to authorize more capacity
    console.error("Insufficient authorization for renewal");
  }
}
```

## Best Practices

1. **Renew early**: Don't wait until the last block before expiration
2. **Track metadata**: Store block numbers and indices in a database
3. **Monitor authorization**: Keep sufficient authorization for planned renewals
4. **Handle failures**: Implement retry logic for failed renewals
5. **Consider IPFS pinning**: For critical data, also pin to external IPFS services as backup

## Next Steps

- [Storage Model](./storage.md) - Understanding how data is stored
- [Authorization](./authorization.md) - Managing storage authorization
- [Data Retrieval](./retrieval.md) - Fetching stored data via IPFS
