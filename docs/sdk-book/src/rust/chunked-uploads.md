# Chunked Uploads

Files larger than 2 MiB are automatically chunked when using `store()`. You can also use `store_chunked()` for explicit control.

## Automatic Chunking

```rust
let data = std::fs::read("large-file.bin")?;

let result = client
    .store(data)
    .with_callback(|event| {
        match event {
            ProgressEvent::ChunkCompleted { index, total, .. } => {
                println!("Chunk {}/{}", index + 1, total);
            }
            ProgressEvent::Completed { .. } => println!("Done!"),
            _ => {}
        }
    })
    .send()
    .await?;
```

### Configuration

```rust
let config = AsyncClientConfig {
    chunking_threshold: 5 * 1024 * 1024,  // chunk files > 5 MiB
    default_chunk_size: 2 * 1024 * 1024,   // 2 MiB chunks
    create_manifest: true,
    check_authorization_before_upload: true,
    ..Default::default()
};

let client = AsyncBulletinClient::with_config(submitter, config);
```

## Manual Chunking

```rust
let config = ChunkerConfig {
    chunk_size: 1024 * 1024,  // 1 MiB
    max_parallel: 8,
    create_manifest: true,
};

let result = client
    .store_chunked(&data, Some(config), StoreOptions::default(), None)
    .await?;

println!("Chunks: {}, Manifest: {:?}", result.num_chunks, result.manifest_cid.map(hex::encode));
```

Chunk size guidelines: 1 MiB (min) to 2 MiB (max, Bitswap limit). Default 1 MiB.

## Two-Step Approach

Prepare chunks locally without submitting:

```rust
let client = BulletinClient::new();
let (batch, manifest_data) = client.prepare_store_chunked(
    &data, Some(config), StoreOptions::default(), None,
)?;

for operation in batch.operations {
    custom_submit(operation.data).await?;
}
if let Some(manifest_bytes) = manifest_data {
    custom_submit(manifest_bytes).await?;
}
```

Authorization checking works the same as [basic storage](./basic-storage.md#authorization-checking) - use `.with_account()` to enable pre-flight checks.
