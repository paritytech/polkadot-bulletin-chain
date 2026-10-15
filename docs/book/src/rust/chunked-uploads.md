# Chunked Uploads

`estimate_upload` → `submit` uploads a large file in chunks (default 1 MiB). When the file spans more than one chunk, the SDK also stores a DAG-PB manifest that links the chunks; the manifest CID is the file's retrieval CID.

> **Complete Working Example**: See [`examples/rust/authorize-and-store`](https://github.com/paritytech/polkadot-bulletin-chain/tree/main/examples/rust/authorize-and-store) for a complete runnable example demonstrating chunked storage with DAG-PB manifests.

## Uploading a File

```rust
use bulletin_sdk_rust::prelude::*;
use std::{str::FromStr, sync::Arc};
use subxt_signer::{sr25519::Keypair, SecretUri};
use tracing::info;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let client = TransactionClient::new("ws://localhost:10000").await?;
    let signer = Keypair::from_uri(&SecretUri::from_str("//Alice")?)?;

    // 3 MiB of data
    let large_data: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 256) as u8).collect();
    let source: Arc<dyn SeekableSource> = Arc::new(blob_from_bytes(large_data));

    // Plan: chunk CIDs, DAG-PB manifest, and the authorization the upload needs
    let options = UploadEstimateOptions {
        chunker: ChunkerConfig { chunk_size: 1024 * 1024, ..Default::default() },
        ..Default::default()
    };
    let estimate = client.estimate_upload(UploadInput::Source(source.clone()), options).await?;
    info!("{} transactions, {} bytes", estimate.base.transactions, estimate.base.bytes);

    // Submit: the pipeline assigns nonces, packs chunks into blocks, and handles reorgs
    let config = UploadConfig {
        on_event: Some(Arc::new(|event: UploadEvent| info!("{event:?}"))),
        ..Default::default()
    };
    let result = client.submit(&signer, estimate, source, config).await?;

    // CIDs in plan order; the manifest root CID is last
    info!("Manifest CID: {:?}", result.cids.last());
    Ok(())
}
```

## How It Works

1. **`estimate_upload`** reads the source once, splits it into chunks, calculates each chunk's CID, and builds a DAG-PB manifest when there is more than one chunk. The returned `StreamEstimate` contains the plan and the `transactions`/`bytes` the upload consumes.
2. **`submit`** reads each chunk from the source when it is sent (`SeekableSource::read`), groups chunks into waves sized to the block limits, and confirms each chunk through the chain's `TransactionByContentHash` index. The manifest is submitted last.
3. **Returns** `UploadResult { cids }`: one CID per plan unit, with the manifest root CID last.

By default, `submit` stores every chunk again even if the same content is already on chain (this refreshes its retention). Set `skip_existing: true` in `UploadEstimateOptions` to leave out chunks that are already stored.

## Configuration Options

### Chunk Size

```rust
let options = UploadEstimateOptions {
    chunker: ChunkerConfig { chunk_size: 2 * 1024 * 1024, ..Default::default() },
    ..Default::default()
};
```

**Guidelines:**
- Default: 1 MiB
- Maximum: 2 MiB (2,097,152 bytes), the chain's maximum transaction size. `estimate_upload` only rejects a chunk size of 0, so keep the value within this limit yourself.
- `estimate_upload` and `submit` have no file size limit. `BulletinClient::prepare_store_chunked` accepts up to 64 MiB.

### Manifest Creation

```rust
// Without manifest (just upload chunks)
let options = UploadEstimateOptions {
    chunker: ChunkerConfig { create_manifest: false, ..Default::default() },
    ..Default::default()
};
```

A source that fits in one chunk never gets a manifest; its chunk CID is the retrieval CID.

### Block Usage

`submit` sizes each wave of chunks to the chain's block limits (`UploadConfig.block_limits`, default `DEFAULT_BLOCK_LIMITS`), so one block can include several chunks. Bulletin Chain has a 6 second block time. `submit` returns when every chunk reaches `UploadConfig.complete_on` (default `WaitFor::Finalized`).

## Progress Tracking

Upload progress arrives through `UploadConfig.on_event`:

```rust
use std::sync::Arc;

let on_event: UploadCallback = Arc::new(|event| match event {
    UploadEvent::ItemStarted { index, total, .. } => {
        tracing::info!("[{}/{}] Started", index + 1, total);
    }
    UploadEvent::ItemInBlock { index, total, block_number, .. } => {
        // Fires again if a reorg moves the chunk to another block
        tracing::info!("[{}/{}] In block {:?}", index + 1, total, block_number);
    }
    UploadEvent::ItemFinalized { index, total, block_number, transaction_index, .. } => {
        tracing::info!(
            "[{}/{}] Finalized in block {:?}, transaction index {:?}",
            index + 1, total, block_number, transaction_index
        );
    }
    UploadEvent::ItemFailed { index, total, error, .. } => {
        tracing::error!("[{}/{}] Failed: {}", index + 1, total, error);
    }
});
let config = UploadConfig { on_event: Some(on_event), ..Default::default() };
```

`BulletinClient::prepare_store_chunked` takes a separate `ProgressCallback`. It emits `ChunkStarted { index: 0, total }` once, then `ManifestStarted` and `ManifestCreated` when it builds a manifest.

## Authorization for Chunked Uploads

Check authorization before a large upload. The estimate contains the exact numbers:

```rust
use subxt::utils::AccountId32;

let account = AccountId32::from(signer.public_key().0);
let (transactions, bytes) = (estimate.base.transactions, estimate.base.bytes);

match client.check_authorization_for_store(&account, transactions, bytes).await {
    Ok(()) => tracing::info!("Authorization sufficient, proceeding with upload"),
    Err(Error::InsufficientAuthorization { .. } | Error::AuthorizationNotFound(_)) => {
        // `authorizer` must be a registered authorizer
        client.authorize_account(account, transactions, bytes, &authorizer, WaitFor::Finalized).await?;
    }
    Err(e) => return Err(e.into()),
}
```

## Error Handling

`estimate_upload` returns `EmptyData` for an empty source and `InvalidChunkSize` for a chunk size of 0. `submit` errors:

```rust
use bulletin_sdk_rust::prelude::*;

match client.submit(&signer, estimate, source, config).await {
    Ok(result) => tracing::info!("Stored {} units", result.cids.len()),
    Err(Error::UploadIncomplete(message)) => {
        // Some chunks failed permanently (see the ItemFailed events). Run
        // estimate_upload with skip_existing: true and submit again to upload
        // only the missing chunks.
        tracing::error!("Upload incomplete: {message}");
    }
    Err(e) if e.is_retryable() => {
        // StoreStalled or NetworkError after the pipeline's own retries
        tracing::warn!(code = e.code(), hint = e.recovery_hint(), "Upload interrupted: {e}");
    }
    Err(e) => tracing::error!(code = e.code(), "Upload failed: {e}"),
}
```

## Two-Step Approach (Advanced)

For more control, prepare chunks with `BulletinClient` and submit them via your own subxt client. `prepare_store_chunked` accepts up to 64 MiB with chunks of up to 2 MiB. This path has none of the pipeline's nonce, retry, or reorg handling. `bulletin` is your own subxt-generated module (see [Authorization](./authorization.md#two-step-approach-advanced)).

```rust
use bulletin_sdk_rust::prelude::*;
use bulletin::runtime_types::bulletin_transaction_storage_primitives::cids::{
    CidConfig, HashingAlgorithm,
};

// Step 1: Prepare chunks locally (no network needed)
let client = BulletinClient::new();
let (batch, manifest_data) = client.prepare_store_chunked(
    &data,
    Some(config),
    StoreOptions::default(),
    None, // no progress callback
)?;

// Step 2: Submit each chunk (raw codec, blake2b-256: the defaults of `store`)
for operation in &batch.operations {
    let tx = bulletin::tx().transaction_storage().store(operation.data.clone());
    api.tx().sign_and_submit_then_watch_default(&tx, &signer).await?;
}

// The manifest CID uses the DAG-PB codec (0x70)
if let Some(manifest_bytes) = manifest_data {
    let cid_config = CidConfig { codec: 0x70, hashing: HashingAlgorithm::Blake2b256 };
    let tx = bulletin::tx().transaction_storage().store_with_cid_config(cid_config, manifest_bytes);
    api.tx().sign_and_submit_then_watch_default(&tx, &signer).await?;
}
```

## Best Practices

1. **Estimate and check authorization** before starting large uploads
2. **Choose an appropriate chunk size**: 1 MiB is a good default; never exceed 2 MiB
3. **Keep manifest creation enabled** for files that need to be retrieved as a whole via IPFS
4. **Track progress** with `UploadConfig.on_event` to show users what's happening
5. **Handle partial failures**: uploads are not atomic. After `UploadIncomplete`, the chunks that were stored remain on chain; re-run with `skip_existing: true` to upload only the missing ones
6. **Keep the manifest CID**: use it to retrieve the complete file later via IPFS/Bitswap

## Next Steps

- [Authorization](./authorization.md) - Manage storage authorization
- [Basic Storage](./basic-storage.md) - For small data
- [Error Handling](./error-handling.md) - Error codes, retry logic, recovery hints
