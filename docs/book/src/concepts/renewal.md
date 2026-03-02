# Data Renewal

Data stored on Bulletin Chain has a **retention period** after which it may be pruned from validators. To keep data available on-chain, you must **renew** it before the retention period expires.

## The Renewal Flow

```
1. STORE           2. RECEIVE EVENT      3. TRACK           4. RENEW (later)
   ↓                  ↓                     ↓                  ↓
Submit data    Get block number      Save block/index    Call renew()
to chain       and index from        for later use       before expiration
               Stored event
```

### Step 1: Store Data

When you submit a `store` transaction, data is written to the chain.

### Step 2: Receive the Stored Event

After the transaction is included in a block, the chain emits a **Stored** event containing:
- `index`: Transaction index within the block
- `content_hash`: Hash of the stored content

You also need to record the **block number** where the transaction was included.

### Step 3: Track for Later Renewal

Save the `(block_number, index)` pair. You'll need these to renew later:

```
Stored at block 1000, index 5
Retention period: 100,800 blocks
Expires at: block 101,800
```

### Step 4: Renew Before Expiration

Before the retention period ends, submit a `renew(block, index)` transaction. This:
- Extends the retention period from the **current block**
- Emits a new `Renewed` event with a **new index**
- You must track the **new** block/index for the next renewal

## Retention Period

The retention period is a chain constant. Query it via:
- RPC: `transactionStorage.retentionPeriod()`
- Typical value: ~100,800 blocks (~7 days at 6s/block)

After the retention period, validators may prune the data. The chain no longer guarantees availability.

## When to Renew

**Renew when you need:**
- Guaranteed on-chain availability beyond the retention period
- Validators to continue providing storage proofs
- Chain-level data guarantees for your application

**You don't need to renew if:**
- The data only needs to exist temporarily
- You've replicated the data to external storage
- The retention period is sufficient for your use case

## Renewal Chain

Each renewal creates a new record. Always use the **most recent** block and index:

```
Block 1000: store()  → Stored { index: 5 }
                       ↓
                       Save (1000, 5)
                       ↓
Block 50000: renew(1000, 5) → Renewed { index: 2 }
                              ↓
                              Save (50000, 2)  ← Use this for next renewal!
                              ↓
Block 100000: renew(50000, 2) → Renewed { index: 0 }
                                ↓
                                Save (100000, 0)
```

## Authorization

Like `store`, renewal requires **authorization**:
- The account must have sufficient authorized bytes/transactions
- Authorization is consumed based on the data size
- Pre-authorize enough capacity if you plan multiple renewals

## Data Availability After Expiration

Even after data expires on-chain:
- Validator nodes may still have the data cached temporarily
- The CID remains valid; only on-chain storage guarantees expire
- Consider replicating critical data to external storage as backup

## SDK Support

Both SDKs provide renewal helpers:
- **Rust SDK**: `prepare_renew()`, `RenewalTracker` - See [Rust SDK: Renewal](../rust/renewal.md)
- **TypeScript SDK**: Coming soon - See [TypeScript SDK](../typescript/README.md)

## Next Steps

- [Rust SDK: Renewal](../rust/renewal.md) - SDK-specific implementation
- [Storage Model](./storage.md) - How data is stored
- [Data Retrieval](./retrieval.md) - Fetching from validator nodes
