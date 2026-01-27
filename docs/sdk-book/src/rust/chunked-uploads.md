# Chunked Uploads

For large files, use `prepare_store_chunked`.

```rust
use bulletin_sdk_rust::prelude::*;

let client = BulletinClient::new();
let large_data = vec![0u8; 10 * 1024 * 1024]; // 10 MiB

// Configure chunking
let config = ChunkerConfig {
    chunk_size: 1024 * 1024, // 1 MiB
    max_parallel: 8,
    create_manifest: true,
};

// Optional progress callback
let progress = |event: ProgressEvent| {
    println!("{:?}", event);
};

// Prepare operations
let (batch, manifest) = client.prepare_store_chunked(
    &large_data,
    Some(config),
    StoreOptions::default(),
    Some(progress),
)?;

// 'batch.operations' contains a list of StorageOperations (chunks)
// 'manifest' contains the DAG-PB manifest bytes (if requested)
```

## Submitting Chunks

You must submit each chunk individually. The order doesn't strictly matter for the chain, but sequential is usually best.

```rust
for op in batch.operations {
    // Submit op.data via subxt
}

if let Some(manifest_bytes) = manifest {
    // Submit manifest_bytes via subxt
}
```
