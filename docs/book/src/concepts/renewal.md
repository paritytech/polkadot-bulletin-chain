# Data Renewal

Data stored on Bulletin Chain has a **retention period** after which it may be pruned from validators. To keep data available on-chain, you must **renew** it before the retention period expires.

## Referencing Stored Data

Renewal extrinsics identify the data with a `TransactionRef` (`entry`). It has two variants:

- `Position { block, index }` — the block number and transaction index from the original `Stored` event.
- `ContentHash(hash)` — the content hash of the stored data.

## Renewal Extrinsics

The pallet exposes three distinct renewal operations. They behave differently — pick the one that matches your needs.

### `renew(entry)` — one-shot scheduled renewal

Schedules a **single** auto-renewal that fires once when the data reaches its retention boundary. After that one renewal the registration is removed and the data is no longer renewed.

- Does **not** renew synchronously at dispatch time.
- Emits `RenewalEnabled { content_hash, who, recurring: false }`.
- Does **not** emit `Renewed`.

### `force_renew(entry)` — immediate synchronous renewal

Renews the data **immediately** at dispatch time, extending its retention from the current block.

- Emits `Renewed { index, content_hash }` with the **new index**.
- You must track the new `(block, index)` for the next renewal.

### `enable_auto_renew(content_hash)` — continuous renewal

Registers the data (identified by content hash, not a `TransactionRef`) for **recurring** auto-renewal. The chain renews it automatically at each retention cycle until disabled.

- Emits `RenewalEnabled { content_hash, who, recurring: true }`.
- Emits `DataAutoRenewed { index, content_hash, account }` at each cycle.

Use `disable_auto_renew(content_hash)` to stop recurring renewal. It emits `AutoRenewalDisabled { content_hash, who }`.

## Retention Period

The retention period is a runtime **storage value** (`RetentionPeriod`), not a fixed constant — it can be changed by governance. Query it from storage:

- Storage: `TransactionStorage.RetentionPeriod`
- Default: 201,600 blocks (~14 days at 6s/block)

After the retention period, validators may prune the data. The chain no longer guarantees availability.

## Renewal Flow

```
1. STORE            2. RECEIVE EVENT      3. TRACK            4. RENEW (later)
   ↓                   ↓                     ↓                   ↓
Submit data     Get block number      Save block/index    renew / force_renew /
to chain        and index from        for later use       enable_auto_renew
                Stored event                              before expiration
```

When you submit `store`, the chain emits a `Stored` event with the transaction `index` and `content_hash`. Record the block number too, so you can build the `Position { block, index }` reference later.

For a `force_renew` chain, always renew against the **most recent** reference, since each `force_renew` emits a `Renewed` event with a new index:

```
Block 1000:  store()             → Stored  { index: 5 }   → save (1000, 5)
Block 50000: force_renew((1000,5)) → Renewed { index: 2 } → save (50000, 2)
Block 100000: force_renew((50000,2)) → Renewed { index: 0 } → save (100000, 0)
```

With `enable_auto_renew` the chain tracks this for you and re-registers the data each cycle; you only need the original reference and can stop it with `disable_auto_renew`.

## Raw Runtime Call

`renew` and `force_renew` take an `entry: TransactionRef` (a tagged enum). `enable_auto_renew` / `disable_auto_renew` instead take the `content_hash` directly. A raw runtime call (e.g. via PAPI):

```typescript
api.tx.TransactionStorage.renew({
  entry: { type: "Position", value: { block, index } }
});

// force_renew takes the same `entry`:
api.tx.TransactionStorage.force_renew({
  entry: { type: "ContentHash", value: contentHash }
});

// enable_auto_renew / disable_auto_renew take a content hash, not an `entry`:
api.tx.TransactionStorage.enable_auto_renew({ content_hash: contentHash });
```

## When to Renew

**Renew when you need:**
- Guaranteed on-chain availability beyond the retention period
- Validators to continue providing storage proofs
- Chain-level data guarantees for your application

**You don't need to renew if:**
- The data only needs to exist temporarily
- You've replicated the data to external storage
- The retention period is sufficient for your use case

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

Both SDKs provide a `renew` helper:
- **Rust SDK**: `prepare_renew()`, `RenewalTracker` — See [Rust SDK: Renewal](../rust/renewal.md)
- **TypeScript SDK**: `client.renew()` — See [TypeScript SDK: Renewal](../typescript/renewal.md)

## Next Steps

- [Rust SDK: Renewal](../rust/renewal.md) - SDK-specific implementation
- [TypeScript SDK: Renewal](../typescript/renewal.md) - SDK-specific implementation
- [Storage Model](./storage.md) - How data is stored
- [Data Retrieval](./retrieval.md) - Fetching from validator nodes
