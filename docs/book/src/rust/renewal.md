# Renewal

This guide shows how to renew stored data using the Rust SDK to extend the retention period.

> **Prerequisites**: Read [Data Renewal Concepts](../concepts/renewal.md) first to understand the renewal flow.

## The Complete Flow

```rust
use bulletin_sdk_rust::prelude::*;

// 1. STORE - Submit data and get the transaction result
let operation = client.prepare_store(data, StoreOptions::default())?;
// Submit via subxt...
// let result = api.tx().transaction_storage().store(...).sign_and_submit_then_watch(...).await?;

// 2. RECEIVE EVENT - Extract block number and index from Stored event
let block_number = result.block_number;
let index = /* extract from Stored event in result.events */;

// 3. TRACK - Save for later renewal
let storage_ref = StorageRef::new(block_number, index);
// Save storage_ref to your database/storage...

// 4. RENEW (later) - When approaching expiration
let renewal = client.prepare_renew(storage_ref)?;
// Submit: api.tx().transaction_storage().renew(renewal.block, renewal.index)
```

## Preparing a Renewal

The SDK provides `prepare_renew()` to create renewal operations:

```rust
use bulletin_sdk_rust::prelude::*;

let client = BulletinClient::new();

// From StorageRef
let storage_ref = StorageRef::new(1000, 5);  // block 1000, index 5
let renewal = client.prepare_renew(storage_ref)?;

// Or directly from raw values
let renewal = client.prepare_renew_raw(1000, 5)?;

// Submit via subxt
let tx = bulletin::tx()
    .transaction_storage()
    .renew(renewal.block, renewal.index);

let result = api
    .tx()
    .sign_and_submit_then_watch_default(&tx, &signer)
    .await?;
```

## Extracting Events After Storage

After storing data, extract the block number and index from the result:

```rust
use subxt::blocks::ExtrinsicEvents;

async fn store_and_track(
    api: &OnlineClient<BulletinConfig>,
    signer: &PairSigner,
    data: Vec<u8>,
) -> Result<StorageRef, Box<dyn std::error::Error>> {
    let client = BulletinClient::new();
    let operation = client.prepare_store(data, StoreOptions::default())?;

    // Submit transaction
    let tx = bulletin::tx()
        .transaction_storage()
        .store(operation.data().to_vec(), None);

    let result = api
        .tx()
        .sign_and_submit_then_watch_default(&tx, signer)
        .await?
        .wait_for_finalized_success()
        .await?;

    // Get block number
    let block_number = result.block_number();

    // Find Stored event and extract index
    let stored_event = result
        .find_first::<bulletin::transaction_storage::events::Stored>()?
        .ok_or("Stored event not found")?;

    let index = stored_event.index;

    Ok(StorageRef::new(block_number, index))
}
```

## Using RenewalTracker

For applications managing multiple stored items, use `RenewalTracker`:

```rust
use bulletin_sdk_rust::prelude::*;

// Create tracker
let mut tracker = RenewalTracker::new();

// Get retention period from chain (once)
let retention_period: u32 = api
    .constants()
    .at(&bulletin::constants().transaction_storage().retention_period())?;

// After each store, track the entry
tracker.track(
    StorageRef::new(block_number, index),
    content_hash.to_vec(),
    data_size,
    retention_period,
);

// Periodically check for expiring entries
let current_block = api.blocks().at_latest().await?.number();
let buffer = 1000; // Renew 1000 blocks before expiration

let expiring = tracker.expiring_before(current_block + buffer);

for entry in expiring {
    // Prepare and submit renewal
    let renewal = client.prepare_renew(entry.storage_ref)?;

    let tx = bulletin::tx()
        .transaction_storage()
        .renew(renewal.block, renewal.index);

    let result = api
        .tx()
        .sign_and_submit_then_watch_default(&tx, &signer)
        .await?
        .wait_for_finalized_success()
        .await?;

    // Extract new block/index from Renewed event
    let renewed_event = result
        .find_first::<bulletin::transaction_storage::events::Renewed>()?
        .ok_or("Renewed event not found")?;

    let new_block = result.block_number();
    let new_index = renewed_event.index;

    // Update tracker with new reference
    tracker.update_after_renewal(
        entry.storage_ref,
        StorageRef::new(new_block, new_index),
        retention_period,
    );

    tracing::info!(
        old_block = entry.storage_ref.block,
        new_block = new_block,
        new_index = new_index,
        "Renewed storage"
    );
}
```

## Persisting Tracker State

For production use, persist the tracker state:

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

// Save to database/file
fn save_entries(tracker: &RenewalTracker) -> Vec<PersistedEntry> {
    tracker.entries()
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

// Restore from database/file
fn restore_entries(entries: Vec<PersistedEntry>) -> RenewalTracker {
    let mut tracker = RenewalTracker::new();
    for e in entries {
        // Note: We store expires_at directly, so retention_period=0 works
        // because expires_at = block + retention_period was already computed
        tracker.track(
            StorageRef::new(e.block, e.index),
            e.content_hash,
            e.size,
            e.expires_at.saturating_sub(e.block), // Recover retention period
        );
    }
    tracker
}
```

## Error Handling

```rust
use bulletin_sdk_rust::prelude::*;

match client.prepare_renew(storage_ref) {
    Ok(renewal) => {
        // Submit renewal...
    }
    Err(Error::RenewalFailed(msg)) => {
        tracing::error!(reason = %msg, "Invalid renewal parameters");
    }
    Err(e) => {
        tracing::error!(?e, "Unexpected error");
    }
}

// Chain-level errors (from subxt)
match submit_renewal(&api, &signer, renewal).await {
    Ok(result) => { /* success */ }
    Err(e) if e.to_string().contains("RenewedNotFound") => {
        // Original transaction not found - data may have been pruned
        tracing::error!("Cannot renew: original data not found on chain");
    }
    Err(e) if e.to_string().contains("InsufficientAuthorization") => {
        // Need more authorization
        tracing::error!("Insufficient authorization for renewal");
    }
    Err(e) => {
        tracing::error!(?e, "Renewal failed");
    }
}
```

## Complete Example

```rust
use bulletin_sdk_rust::prelude::*;
use std::collections::HashMap;
use tracing::{info, error};

struct StorageManager {
    client: BulletinClient,
    tracker: RenewalTracker,
    retention_period: u32,
}

impl StorageManager {
    pub fn new(retention_period: u32) -> Self {
        Self {
            client: BulletinClient::new(),
            tracker: RenewalTracker::new(),
            retention_period,
        }
    }

    /// Store data and track for renewal
    pub async fn store(
        &mut self,
        api: &OnlineClient<BulletinConfig>,
        signer: &PairSigner,
        data: Vec<u8>,
    ) -> Result<(Vec<u8>, StorageRef)> {
        let operation = self.client.prepare_store(data.clone(), StoreOptions::default())?;
        let cid = operation.calculate_cid()?.to_bytes()
            .ok_or(Error::StorageFailed("CID conversion failed".into()))?;

        // Submit
        let tx = bulletin::tx()
            .transaction_storage()
            .store(operation.data().to_vec(), None);

        let result = api.tx()
            .sign_and_submit_then_watch_default(&tx, signer)
            .await?
            .wait_for_finalized_success()
            .await?;

        // Extract event
        let stored = result
            .find_first::<bulletin::transaction_storage::events::Stored>()?
            .ok_or(Error::StorageFailed("No Stored event".into()))?;

        let storage_ref = StorageRef::new(result.block_number(), stored.index);

        // Track
        self.tracker.track(
            storage_ref,
            stored.content_hash.to_vec(),
            data.len() as u64,
            self.retention_period,
        );

        info!(
            block = storage_ref.block,
            index = storage_ref.index,
            cid = %hex::encode(&cid),
            "Stored and tracking for renewal"
        );

        Ok((cid, storage_ref))
    }

    /// Renew all entries expiring within `buffer` blocks
    pub async fn renew_expiring(
        &mut self,
        api: &OnlineClient<BulletinConfig>,
        signer: &PairSigner,
        current_block: u32,
        buffer: u32,
    ) -> Result<u32> {
        let expiring: Vec<_> = self.tracker
            .expiring_before(current_block + buffer)
            .iter()
            .map(|e| e.storage_ref)
            .collect();

        let mut renewed = 0;

        for storage_ref in expiring {
            match self.renew_one(api, signer, storage_ref).await {
                Ok(new_ref) => {
                    self.tracker.update_after_renewal(
                        storage_ref,
                        new_ref,
                        self.retention_period,
                    );
                    renewed += 1;
                }
                Err(e) => {
                    error!(?e, block = storage_ref.block, "Renewal failed");
                }
            }
        }

        Ok(renewed)
    }

    async fn renew_one(
        &self,
        api: &OnlineClient<BulletinConfig>,
        signer: &PairSigner,
        storage_ref: StorageRef,
    ) -> Result<StorageRef> {
        let renewal = self.client.prepare_renew(storage_ref)?;

        let tx = bulletin::tx()
            .transaction_storage()
            .renew(renewal.block, renewal.index);

        let result = api.tx()
            .sign_and_submit_then_watch_default(&tx, signer)
            .await?
            .wait_for_finalized_success()
            .await?;

        let renewed = result
            .find_first::<bulletin::transaction_storage::events::Renewed>()?
            .ok_or(Error::RenewalFailed("No Renewed event".into()))?;

        Ok(StorageRef::new(result.block_number(), renewed.index))
    }
}
```

## Next Steps

- [Basic Storage](./basic-storage.md) - Storing data
- [Chunked Uploads](./chunked-uploads.md) - Large file handling
- [Data Retrieval](../concepts/retrieval.md) - Fetching from validator nodes
