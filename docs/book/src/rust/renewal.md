# Renewal

Extending the retention of stored data with the Rust SDK.

> **Prerequisites**: Read [Data Renewal Concepts](../concepts/renewal.md) first to understand the renewal flow.

> **Note**: `TransactionClient::renew` schedules a one-shot renewal that fires at the retention boundary; `TransactionClient::force_renew` (same arguments) renews immediately, and `TransactionClient::enable_auto_renew(content_hash, ..)` renews every period.

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

`TransactionClient::renew` submits the extrinsic and returns a `RenewReceipt`. The entry
accepts anything convertible to `TransactionRef`: a `(block, index)` tuple or a `ContentHash`:

```rust
let tx_client = TransactionClient::new("wss://paseo-bulletin-next-rpc.polkadot.io").await?;

let receipt = tx_client
    .renew((renewal.block(), renewal.index()), &signer, WaitFor::Finalized)
    .await?;

// Or reference by content hash (stable across renewals):
let receipt = tx_client.renew(content_hash, &signer, WaitFor::Finalized).await?;

println!("Renewed {:?} in block {}", receipt.entry, receipt.block_hash);
```

`RenewReceipt` has `entry` (the `TransactionRef` that was renewed) and `block_hash`.

## Recurring Auto-Renewal

`enable_auto_renew` takes the 32-byte content hash, not a position. The first cycle is prepaid at registration; later cycles charge the owner's authorization, and the chain drops the registration with a `RenewalFailed` event once the quota runs out.

```rust
let content_hash =
    calculate_cid_with_config(&data, CidCodec::Raw, HashingAlgorithm::Blake2b256)?.content_hash;

tx_client.enable_auto_renew(content_hash, &signer, WaitFor::Finalized).await?;

// Only succeeds after the prepaid cycle has fired.
tx_client.disable_auto_renew(content_hash, &signer, WaitFor::Finalized).await?;
```

A content hash carries at most one registration, so `renew` and `enable_auto_renew` exclude each other. The transaction pool rejects these calls as invalid transactions before they reach a block:

| Call | Rejected when | Pool error |
|------|---------------|------------|
| `renew`, `enable_auto_renew` | a renewal is already registered for the content hash | custom 11, `RENEWAL_ALREADY_ENABLED` |
| `disable_auto_renew` | no renewal is registered for the content hash | custom 9, `AUTO_RENEWAL_NOT_ENABLED` |
| `disable_auto_renew` | the caller does not own the registration | custom 10, `NOT_AUTO_RENEWAL_OWNER` |
| `disable_auto_renew` | the next cycle is still prepaid | custom 12, `CANNOT_DISABLE_PREPAID_AUTO_RENEWAL` |

The call then returns `Error::RenewalFailed`, whose message carries the RPC error `1010 Invalid Transaction` with data `Custom error: <code>`.

## Storing and Tracking

The upload pipeline reports each item's `(block, index)` in its `UploadEvent::ItemFinalized` event; use it to build the `StorageRef` used for renewal. `StoreReceipt` (from `TransactionClient::store`) does not contain the index.

```rust
use std::sync::{Arc, Mutex};

let position = Arc::new(Mutex::new(None));
let position_sink = position.clone();
let config = UploadConfig {
    on_event: Some(Arc::new(move |event: UploadEvent| {
        if let UploadEvent::ItemFinalized {
            block_number: Some(block),
            transaction_index: Some(index),
            ..
        } = event
        {
            *position_sink.lock().unwrap() = Some((block, index));
        }
    })),
    ..Default::default()
};

let estimate = tx_client
    .estimate_upload(
        UploadInput::Items(vec![UploadItem::new(data.clone())]),
        UploadEstimateOptions::default(),
    )
    .await?;
let source: Arc<dyn SeekableSource> = Arc::new(blob_from_items(vec![data]));
tx_client.submit(&signer, estimate, source, config).await?;

let (block_number, index) = position.lock().unwrap().expect("item finalized");
let storage_ref = StorageRef::new(block_number, index);
```

Alternatively, renew by content hash, which does not change across renewals.

## Using RenewalTracker

For applications managing multiple stored items, use `RenewalTracker` to know when to renew. `renew` registers at most one scheduled renewal per content hash — the pool rejects a second submission before the boundary (custom error 11, `RENEWAL_ALREADY_ENABLED`):

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
let expiring: Vec<Vec<u8>> = tracker
    .expiring_before(current_block + buffer)
    .iter()
    .map(|e| e.content_hash.clone())
    .collect();

for content_hash in expiring {
    let content_hash: ContentHash = content_hash.as_slice().try_into()?;
    // Schedules a one-shot renewal that fires at the retention boundary
    tx_client.renew(content_hash, &signer, WaitFor::Finalized).await?;
}
```

`renew` does not move the data right away. When the scheduled renewal fires, the chain stores the data again and emits `DataRenewal.DataRenewed { index, .. }` in that block; `force_renew` stores it again in its own inclusion block and emits `DataRenewal.Renewed { index, .. }`. Pass that block and index to `tracker.update_after_renewal(old_ref, StorageRef::new(block, index), retention_period)`.

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

`prepare_renew` rejects invalid input (e.g. block 0) with `Error::RenewalFailed`. Submission errors surface the same variant, including the pool's invalid-transaction rejections such as custom error 11 (`RENEWAL_ALREADY_ENABLED`, a renewal is already scheduled for the content hash). Two exceptions: a chain without any `renew` call returns `Error::RenewalUnavailable` (code `UNSUPPORTED_OPERATION`), and legacy runtimes that take only `(block, index)` reject content-hash renewal, `force_renew` and the auto-renew calls with `RenewalFailed` before submitting:

```rust
match client.prepare_renew(storage_ref) {
    Ok(renewal) => {
        let entry = (renewal.block(), renewal.index());
        if let Err(e) = tx_client.renew(entry, &signer, WaitFor::Finalized).await {
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
