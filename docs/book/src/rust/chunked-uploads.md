# Chunked Uploads

The Bulletin SDK automatically handles chunking for large files up to **64 MiB**. When data exceeds the chunk size threshold (default 1 MiB), it is split into chunks of up to **2 MiB** each (ensuring IPFS Bitswap compatibility).

> **Complete Working Example**: See [`examples/rust/authorize-and-store`](https://github.com/paritytech/polkadot-bulletin-chain/tree/main/examples/rust/authorize-and-store) for a complete runnable example demonstrating chunked storage with DAG-PB manifests.

## Using TransactionClient with DAG-PB Manifest

For most use cases, use `BulletinClient` to prepare chunks and `TransactionClient` to submit them:

```rust
use bulletin_sdk_rust::prelude::*;
use subxt_signer::sr25519::Keypair;
use std::str::FromStr;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect and setup signer
    let client = TransactionClient::new("ws://localhost:10000").await?;
    let uri = subxt_signer::SecretUri::from_str("//Alice")?;
    let signer = Keypair::from_uri(&uri)?;

    // Create large data (3 MiB)
    let large_data: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|i| (i % 256) as u8)
        .collect();

    // Configure chunking
    let chunker_config = ChunkerConfig {
        chunk_size: 1024 * 1024,  // 1 MiB chunks
        max_parallel: 4,
        create_manifest: true,    // Create DAG-PB manifest
    };

    let dag_options = StoreOptions {
        cid_codec: CidCodec::DagPb,
        hash_algorithm: HashingAlgorithm::Blake2b256,
        wait_for: WaitFor::InBlock,
    };

    // Prepare chunks using BulletinClient (no network needed)
    let sdk_client = BulletinClient::new();
    let progress_callback = Arc::new(|event: ProgressEvent| {
        info!("Chunk progress: {:?}", event);
    });

    let (batch_operation, manifest_data) = sdk_client
        .prepare_store_chunked(&large_data, Some(chunker_config), dag_options, Some(progress_callback))?;

    info!("Prepared {} chunks", batch_operation.operations.len());

    // Submit each chunk
    for (i, chunk_op) in batch_operation.operations.iter().enumerate() {
        info!("Submitting chunk {}/{}...", i + 1, batch_operation.operations.len());
        let receipt = client.store(chunk_op.data.clone(), &signer, WaitFor::InBlock).await?;
        info!("  Chunk {} stored in block: {}", i + 1, receipt.block_hash);
    }

    // Submit the manifest
    if let Some(manifest) = manifest_data {
        info!("Submitting DAG-PB manifest ({} bytes)...", manifest.len());
        let receipt = client.store(manifest, &signer, WaitFor::InBlock).await?;
        info!("Manifest stored in block: {}", receipt.block_hash);
        info!("Use this manifest CID to retrieve the complete file via IPFS/Bitswap");
    }

    Ok(())
}
```

## How It Works

The chunked upload flow:

1. **Splits data** into chunks (default 1 MiB, max 2 MiB)
2. **Calculates CIDs** for each chunk
3. **Creates DAG-PB manifest** linking all chunks (optional)
4. **Submits chunks** as individual `TransactionStorage.store` transactions
5. **Submits manifest** as the final transaction
6. **Returns** all chunk data and the manifest bytes

## Configuration Options

### Chunk Size

```rust
let config = ChunkerConfig {
    chunk_size: 2 * 1024 * 1024,  // 2 MiB chunks (MAX_CHUNK_SIZE)
    max_parallel: 4,
    create_manifest: true,
};
```

**Guidelines:**
- Minimum: 1 byte
- Maximum: 2 MiB (2,097,152 bytes) — ensures IPFS Bitswap compatibility
- Default: 1 MiB — good balance of transaction overhead and throughput
- Maximum file size: 64 MiB (MAX_FILE_SIZE)

### Manifest Creation

```rust
// With manifest (recommended for large files)
let config = ChunkerConfig {
    chunk_size: 1024 * 1024,
    max_parallel: 8,
    create_manifest: true,  // Creates DAG-PB manifest
};

// Without manifest (just upload chunks)
let config = ChunkerConfig {
    chunk_size: 1024 * 1024,
    max_parallel: 8,
    create_manifest: false,  // No manifest, just chunks
};
```

### Upload Timing

Bulletin Chain has a **6 second block time**. Each chunk requires one transaction, so:

| File Size | Chunk Size | Chunks | Transactions | Min Time (sequential) |
|-----------|------------|--------|--------------|----------------------|
| 2 MiB | 2 MiB | 1 | 2 (data + manifest) | ~12 seconds |
| 32 MiB | 2 MiB | 16 | 17 | ~102 seconds |
| 64 MiB | 2 MiB | 32 | 33 | ~198 seconds |

**Note**: Times shown are for sequential uploads waiting for each transaction to be included in a block. Actual times may vary based on network conditions and finalization requirements.

## Progress Tracking

Track chunk preparation progress with callbacks:

```rust
use std::sync::Arc;

let progress = Arc::new(|event: ProgressEvent| {
    match event {
        ProgressEvent::Chunk(chunk_event) => {
            match chunk_event {
                ChunkProgressEvent::ChunkStarted { index, total } => {
                    tracing::info!("[{}/{}] Starting chunk...", index + 1, total);
                }
                ChunkProgressEvent::ChunkCompleted { index, total, cid } => {
                    tracing::info!("[{}/{}] Chunk ready", index + 1, total);
                }
                ChunkProgressEvent::ManifestStarted => {
                    tracing::info!("Creating manifest...");
                }
                ChunkProgressEvent::ManifestCreated { cid } => {
                    tracing::info!("Manifest CID ready");
                }
                ChunkProgressEvent::Completed { manifest_cid } => {
                    tracing::info!("All chunks prepared!");
                }
                _ => {}
            }
        }
        ProgressEvent::Transaction(tx_event) => {
            tracing::info!("{}", tx_event.description());
        }
    }
});

let (batch, manifest) = sdk_client
    .prepare_store_chunked(&data, Some(config), options, Some(progress))?;
```

## Authorization for Chunked Uploads

For large chunked uploads, check authorization before starting to avoid wasting time uploading many chunks only to fail partway through.

### Estimate Requirements

```rust
let sdk_client = BulletinClient::new();
let file_size = std::fs::metadata("large-file.bin")?.len();

// Estimate what's needed (transactions + bytes)
let (txs_needed, bytes_needed) = sdk_client.estimate_authorization(file_size);

tracing::info!("This upload will need:");
tracing::info!("  {} transactions", txs_needed);
tracing::info!("  {} bytes authorized", bytes_needed);
```

### Check Before Upload

```rust
use subxt::utils::AccountId32;

let account = AccountId32::from(signer.public_key().0);

// Check current authorization on-chain
match client.check_authorization_for_store(&account, txs_needed, bytes_needed).await {
    Ok(()) => {
        tracing::info!("Authorization sufficient, proceeding with upload");
    }
    Err(Error::InsufficientAuthorization { need, available }) => {
        tracing::error!(
            need_bytes = need,
            available_bytes = available,
            "Insufficient authorization"
        );
        // Authorize first
        client.authorize_account(account, txs_needed, bytes_needed, &signer, WaitFor::InBlock).await?;
    }
    Err(Error::AuthorizationNotFound(_)) => {
        tracing::error!("No authorization found, authorizing...");
        client.authorize_account(account, txs_needed, bytes_needed, &signer, WaitFor::InBlock).await?;
    }
    Err(e) => return Err(e.into()),
}
```

## Error Handling

```rust
use bulletin_sdk_rust::prelude::*;

// Preparation errors (from BulletinClient)
match sdk_client.prepare_store_chunked(&data, Some(config), options, None) {
    Ok((batch, manifest)) => {
        tracing::info!("Prepared {} chunks", batch.len());
    }
    Err(Error::EmptyData) => {
        tracing::error!("No data to upload");
    }
    Err(Error::FileTooLarge(size)) => {
        tracing::error!("File size {} exceeds 64 MiB limit", size);
    }
    Err(Error::ChunkTooLarge(size)) => {
        tracing::error!("Chunk size {} exceeds 2 MiB limit", size);
    }
    Err(e) => {
        tracing::error!(?e, "Preparation failed");
    }
}

// Submission errors (from TransactionClient)
for (i, chunk_op) in batch.operations.iter().enumerate() {
    match client.store(chunk_op.data.clone(), &signer, WaitFor::InBlock).await {
        Ok(receipt) => {
            tracing::info!("Chunk {} stored in block: {}", i + 1, receipt.block_hash);
        }
        Err(Error::InsufficientAuthorization { need, available }) => {
            tracing::error!(
                need_bytes = need,
                available_bytes = available,
                chunk = i + 1,
                "Insufficient authorization mid-upload"
            );
            break;
        }
        Err(e) if e.is_retryable() => {
            tracing::warn!(code = e.code(), "Retryable error on chunk {}", i + 1);
            // Implement retry logic
        }
        Err(e) => {
            tracing::error!(code = e.code(), "Chunk {} failed: {}", i + 1, e);
            break;
        }
    }
}
```

## Two-Step Approach (Advanced)

For more control, prepare chunks with `BulletinClient` and submit via your own subxt client:

```rust
use bulletin_sdk_rust::prelude::*;

// Step 1: Prepare chunks locally (no network needed)
let client = BulletinClient::new();
let (batch, manifest_data) = client.prepare_store_chunked(
    &data,
    Some(config),
    StoreOptions::default(),
    None, // no progress callback
)?;

// Step 2: Submit manually via your own subxt client
for operation in &batch.operations {
    let tx = bulletin::tx().transaction_storage().store(
        operation.data.clone(),
    );
    let result = api.tx()
        .sign_and_submit_then_watch_default(&tx, &signer)
        .await?;
}

if let Some(manifest_bytes) = manifest_data {
    let tx = bulletin::tx().transaction_storage().store(manifest_bytes);
    let result = api.tx()
        .sign_and_submit_then_watch_default(&tx, &signer)
        .await?;
}
```

## Best Practices

1. **Estimate and check authorization** before starting large uploads to avoid wasting fees
2. **Choose appropriate chunk size** — 1 MiB is a good default (balances transaction overhead vs. throughput)
3. **Enable manifest creation** for files that need to be retrieved as a whole via IPFS
4. **Track progress** with callbacks to show users what's happening
5. **Handle partial failures** — chunked uploads are not atomic; if a chunk fails, previously submitted chunks remain on-chain
6. **Keep the manifest CID** — use it to retrieve the complete file later via IPFS/Bitswap

## Next Steps

- [Authorization](./authorization.md) - Manage storage authorization
- [Basic Storage](./basic-storage.md) - For small files < 2 MiB
- [Error Handling](./error-handling.md) - Error codes, retry logic, recovery hints
