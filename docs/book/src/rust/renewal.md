# Renewal

This guide shows how to renew stored data using the Rust SDK to extend the retention period.

> **Prerequisites**: Read [Data Renewal Concepts](../concepts/renewal.md) first to understand the renewal flow.

> **Note**: `TransactionClient::renew` schedules a one-shot renewal — it fires once when the data reaches its retention boundary. For immediate renewal use `TransactionClient::force_renew` (same arguments). Recurring `enable_auto_renew` is not exposed by the SDK; call it via subxt against the live runtime if you need it (see [concepts](../concepts/renewal.md)).

## Two Clients

- `BulletinClient` — offline. Prepares and validates operations (no network).
- `TransactionClient` — online. Submits extrinsics and returns receipts.

Both are in the prelude:

```rust
use bulletin_sdk_rust::prelude::*;
use subxt_signer::sr25519::Keypair;
```

## Preparing a Renewal

`BulletinClient::prepare_renew` validates a `StorageRef` and returns a `RenewalOperation`:

```rust
let client = BulletinClient::new();

// From a StorageRef (block 1000, index 5)
let storage_ref = StorageRef::new(1000, 5);
let renewal = client.prepare_renew(storage_ref)?;

// Or directly from raw values
let renewal = client.prepare_renew_raw(1000, 5)?;

// RenewalOperation exposes the reference via methods:
let block = renewal.block();
let index = renewal.index();
```

## Submitting a Renewal

`TransactionClient::renew` submits the extrinsic and returns a `RenewReceipt`:

```rust
let tx_client = TransactionClient::new("wss://paseo-bulletin-rpc.polkadot.io").await?;

let receipt = tx_client
    .renew(renewal.block(), renewal.index(), &signer, WaitFor::Finalized)
    .await?;

println!("Renewed {}:{} in block {}",
    receipt.original_block, receipt.transaction_index, receipt.block_hash);
```

`RenewReceipt` has `original_block`, `transaction_index`, and `block_hash`.

## Storing and Tracking

`TransactionClient::store` submits data; record the `(block, index)` from the `Stored` event (see [Basic Storage](./basic-storage.md)) to build the `StorageRef` used for renewal.

```rust
let receipt = tx_client.store(data, &signer, WaitFor::Finalized).await?;
// Read (block, index) from the Stored event to construct:
let storage_ref = StorageRef::new(block_number, index);
```

## Using RenewalTracker

For applications managing multiple stored items, use `RenewalTracker` to know when to renew:

```rust
let mut tracker = RenewalTracker::new();

// Retention period is an on-chain storage value (default 201_600 blocks).
let retention_period: u32 = 201_600;

// After each store, track the entry.
tracker.track(
    StorageRef::new(block_number, index),
    content_hash.to_vec(),
    data_size,
    retention_period,
);

// Periodically find entries close to expiry.
let buffer = 1000; // renew this many blocks before expiration
let expiring: Vec<StorageRef> = tracker
    .expiring_before(current_block + buffer)
    .iter()
    .map(|e| e.storage_ref)
    .collect();

for storage_ref in expiring {
    let renewal = client.prepare_renew(storage_ref)?;
    tx_client
        .renew(renewal.block(), renewal.index(), &signer, WaitFor::Finalized)
        .await?;

    // Update the tracker with the new reference (read from the Renewed event
    // when using the pallet's force_renew; see concepts).
    tracker.update_after_renewal(
        storage_ref,
        StorageRef::new(new_block, new_index),
        retention_period,
    );
}
```

`TrackedEntry` fields: `storage_ref` (with `.block` / `.index`), `content_hash`, `size`, `expires_at`.

## Persisting Tracker State

For production use, persist the tracked entries:

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct PersistedEntry {
    block: u32,
    index: u32,
    content_hash: Vec<u8>,
    size: u64,
    expires_at: u32,
}

fn save_entries(tracker: &RenewalTracker) -> Vec<PersistedEntry> {
    tracker
        .entries()
        .iter()
        .map(|e| PersistedEntry {
            block: e.storage_ref.block,
            index: e.storage_ref.index,
            content_hash: e.content_hash.clone(),
            size: e.size,
            expires_at: e.expires_at,
        })
        .collect()
}

fn restore_entries(entries: Vec<PersistedEntry>) -> RenewalTracker {
    let mut tracker = RenewalTracker::new();
    for e in entries {
        // expires_at was already computed as block + retention_period.
        tracker.track(
            StorageRef::new(e.block, e.index),
            e.content_hash,
            e.size,
            e.expires_at.saturating_sub(e.block),
        );
    }
    tracker
}
```

## Error Handling

`prepare_renew` rejects invalid input (e.g. block 0) with `Error::RenewalFailed`; submission errors surface the same variant:

```rust
match client.prepare_renew(storage_ref) {
    Ok(renewal) => {
        if let Err(e) = tx_client
            .renew(renewal.block(), renewal.index(), &signer, WaitFor::Finalized)
            .await
        {
            tracing::error!(?e, "Renewal submission failed");
        }
    }
    Err(Error::RenewalFailed(msg)) => {
        tracing::error!(reason = %msg, "Invalid renewal parameters");
    }
    Err(e) => {
        tracing::error!(?e, "Unexpected error");
    }
}
```

## Next Steps

- [Basic Storage](./basic-storage.md) - Storing data
- [Chunked Uploads](./chunked-uploads.md) - Large file handling
- [Data Retrieval](../concepts/retrieval.md) - Fetching from validator nodes
