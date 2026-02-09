# Chunked Uploads

The Bulletin SDK automatically handles chunking for large files. When you call `store()`, files larger than the threshold (default 2 MiB) are automatically split into chunks.

## Automatic Chunking (Recommended)

For most use cases, simply use `store()` - it automatically chunks large files:

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
    let client = AsyncBulletinClient::new(submitter);

    // Load file of any size
    let data = std::fs::read("any-size-file.bin")?;

    // Automatically chunks if > 2 MiB
    let result = client
        .store(data)
        .with_callback(|event| {
            match event {
                ProgressEvent::ChunkCompleted { index, total, .. } => {
                    tracing::info!(chunk = index + 1, total = total, "Chunk uploaded");
                }
                ProgressEvent::Completed { .. } => {
                    tracing::info!("Upload complete");
                }
                _ => {}
            }
        })
        .send()
        .await?;

    tracing::info!(cid = %hex::encode(&result.cid), "Stored successfully");
    if let Some(chunks) = result.chunks {
        tracing::info!(num_chunks = chunks.num_chunks, "File was chunked");
    }

    Ok(())
}
```

### Configuring Automatic Chunking

You can configure the threshold and chunk size:

```rust
use bulletin_sdk_rust::async_client::AsyncClientConfig;

let config = AsyncClientConfig {
    chunking_threshold: 5 * 1024 * 1024,  // Chunk files > 5 MiB
    default_chunk_size: 2 * 1024 * 1024,   // 2 MiB chunks
    max_parallel: 8,                        // Upload 8 chunks in parallel
    create_manifest: true,                  // Create DAG-PB manifest
    check_authorization_before_upload: true,
};

let client = AsyncBulletinClient::with_config(submitter, config);
```

## Advanced: Manual Chunking

For advanced use cases where you need explicit control over chunking parameters, use `store_chunked()`:

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup client (see Basic Storage guide)
    let ws_url = std::env::var("BULLETIN_WS_URL")
        .unwrap_or_else(|_| "ws://localhost:10000".to_string());
    let signer = /* your PairSigner */;

    let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
    let client = AsyncBulletinClient::new(submitter);

    // Load large file
    let large_data = std::fs::read("large-file.bin")?;
    println!("File size: {} bytes", large_data.len());

    // Configure chunking explicitly
    let config = ChunkerConfig {
        chunk_size: 1024 * 1024,  // 1 MiB chunks
        max_parallel: 8,           // Upload 8 chunks in parallel
        create_manifest: true,     // Create DAG-PB manifest
    };

    // Optional: track progress
    let progress_callback = |event: ProgressEvent| {
        match event {
            ProgressEvent::ChunkStarted { index, total } => {
                println!("Uploading chunk {}/{}", index + 1, total);
            }
            ProgressEvent::ChunkCompleted { index, total, cid } => {
                println!("✓ Chunk {}/{} complete: {}", index + 1, total, hex::encode(cid));
            }
            ProgressEvent::ChunkFailed { index, total, error } => {
                eprintln!("✗ Chunk {}/{} failed: {}", index + 1, total, error);
            }
            ProgressEvent::ManifestCreated { cid } => {
                println!("📦 Manifest created: {}", hex::encode(cid));
            }
            ProgressEvent::Completed { manifest_cid } => {
                if let Some(cid) = manifest_cid {
                    println!("✅ All done! Manifest CID: {}", hex::encode(cid));
                }
            }
            _ => {}
        }
    };

    // Upload with manual chunking configuration and progress tracking
    let result = client
        .store_chunked(
            &large_data,
            Some(config),
            StoreOptions::default(),
            Some(progress_callback),
        )
        .await?;

    println!("\n📊 Upload Summary:");
    println!("   Total size: {} bytes", result.total_size);
    println!("   Chunks: {}", result.num_chunks);
    println!("   Chunk CIDs: {} items", result.chunk_cids.len());
    if let Some(manifest_cid) = result.manifest_cid {
        println!("   Manifest CID: {}", hex::encode(manifest_cid));
    }

    Ok(())
}
```

## How It Works

The `store_chunked()` method:

1. **Splits data** into chunks (default 1 MiB)
2. **Calculates CIDs** for each chunk
3. **Submits chunks** sequentially or in parallel
4. **Creates DAG-PB manifest** linking all chunks
5. **Submits manifest** as final transaction
6. **Returns result** with all CIDs

### When to Use `store_chunked()` vs `store()`

**Use `store()` (recommended):**
- ✅ For most use cases - it automatically handles everything
- ✅ When you don't need detailed chunk information
- ✅ For both small and large files

**Use `store_chunked()` (advanced):**
- ⚙️ When you need detailed control over chunking parameters
- ⚙️ When you need the full `ChunkedStoreResult` with all chunk CIDs
- ⚙️ When you want to force chunking on small files
- ⚙️ For testing or debugging chunking behavior

**Key Difference:**
- `store()` returns `StoreResult` with optional chunk info
- `store_chunked()` returns `ChunkedStoreResult` with detailed chunk information

## Configuration Options

### Chunk Size

```rust
let config = ChunkerConfig {
    chunk_size: 2 * 1024 * 1024,  // 2 MiB chunks (max is 2 MiB for Bitswap)
    max_parallel: 4,
    create_manifest: true,
};
```

**Guidelines:**
- Minimum: 1 MiB (1,048,576 bytes)
- Maximum: 2 MiB (2,097,152 bytes) - Bitswap compatibility limit
- Default: 1 MiB - good balance of efficiency and compatibility

### Parallel Uploads

```rust
let config = ChunkerConfig {
    chunk_size: 1024 * 1024,
    max_parallel: 8,  // Upload up to 8 chunks simultaneously
    create_manifest: true,
};
```

**Note**: Current implementation uploads sequentially. Parallel support is planned for a future release.

### Manifest Creation

```rust
// With manifest (IPFS-compatible, recommended)
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

## Progress Tracking

Track upload progress with callbacks:

```rust
let progress = |event: ProgressEvent| {
    match event {
        ProgressEvent::ChunkStarted { index, total } => {
            println!("[{}/{}] Starting chunk...", index + 1, total);
        }
        ProgressEvent::ChunkCompleted { index, total, cid } => {
            println!("[{}/{}] ✓ Uploaded: {}", index + 1, total, hex::encode(cid));
        }
        ProgressEvent::ChunkFailed { index, total, error } => {
            eprintln!("[{}/{}] ✗ Failed: {}", index + 1, total, error);
        }
        ProgressEvent::ManifestStarted => {
            println!("Creating manifest...");
        }
        ProgressEvent::ManifestCreated { cid } => {
            println!("Manifest CID: {}", hex::encode(cid));
        }
        ProgressEvent::Completed { manifest_cid } => {
            println!("Upload complete!");
        }
    }
};

let result = client
    .store_chunked(&data, Some(config), options, Some(progress))
    .await?;
```

## Result Structure

```rust
pub struct ChunkedStoreResult {
    pub chunk_cids: Vec<Vec<u8>>,    // CID for each chunk
    pub manifest_cid: Option<Vec<u8>>, // CID of the DAG-PB manifest
    pub total_size: u64,              // Total bytes uploaded
    pub num_chunks: u32,              // Number of chunks
}
```

Access results:

```rust
let result = client.store_chunked(&data, config, options, None).await?;

println!("Uploaded {} chunks", result.num_chunks);
println!("Total: {} bytes", result.total_size);

// Print all chunk CIDs
for (i, cid) in result.chunk_cids.iter().enumerate() {
    println!("Chunk {}: {}", i, hex::encode(cid));
}

// Use manifest CID for retrieval
if let Some(manifest_cid) = result.manifest_cid {
    println!("Retrieve via: {}", hex::encode(manifest_cid));
}
```

## Error Handling

```rust
match client.store_chunked(&data, config, options, progress).await {
    Ok(result) => {
        println!("Success! {} chunks uploaded", result.num_chunks);
    }
    Err(Error::EmptyData) => {
        eprintln!("Error: No data to upload");
    }
    Err(Error::ChunkTooLarge(size)) => {
        eprintln!("Error: Chunk size {} exceeds limit", size);
    }
    Err(Error::SubmissionFailed(msg)) => {
        eprintln!("Upload failed: {}", msg);
    }
    Err(e) => {
        eprintln!("Unexpected error: {:?}", e);
    }
}
```

## Testing Chunked Uploads

Use `MockSubmitter` for testing:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bulletin_sdk_rust::prelude::*;

    #[tokio::test]
    async fn test_chunked_upload() {
        let submitter = MockSubmitter::new();
        let client = AsyncBulletinClient::new(submitter);

        // Create 10 MB test data
        let data = vec![0u8; 10 * 1024 * 1024];

        let config = ChunkerConfig {
            chunk_size: 1024 * 1024,
            max_parallel: 8,
            create_manifest: true,
        };

        let result = client
            .store_chunked(&data, Some(config), StoreOptions::default(), None)
            .await
            .unwrap();

        assert_eq!(result.num_chunks, 10);
        assert_eq!(result.chunk_cids.len(), 10);
        assert!(result.manifest_cid.is_some());
    }
}
```

## Complete Example

```rust
use bulletin_sdk_rust::prelude::*;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup
    let ws_url = std::env::var("BULLETIN_WS_URL")
        .unwrap_or_else(|_| "ws://localhost:10000".to_string());
    let signer = /* your signer */;

    let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
    let client = AsyncBulletinClient::new(submitter);

    // Load file
    let file_path = Path::new("large-video.mp4");
    let data = std::fs::read(file_path)?;
    println!("Uploading {} ({} bytes)", file_path.display(), data.len());

    // Configure
    let config = ChunkerConfig {
        chunk_size: 1024 * 1024,  // 1 MiB
        max_parallel: 8,
        create_manifest: true,
    };

    // Upload with progress
    let start = std::time::Instant::now();

    let result = client
        .store_chunked(
            &data,
            Some(config),
            StoreOptions::default(),
            Some(|event| {
                if let ProgressEvent::ChunkCompleted { index, total, .. } = event {
                    let percent = ((index + 1) as f64 / total as f64) * 100.0;
                    print!("\rProgress: {:.1}%", percent);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                }
            }),
        )
        .await?;

    let duration = start.elapsed();
    println!("\n\n✅ Upload complete in {:.2}s", duration.as_secs_f64());
    println!("   Manifest CID: {}", hex::encode(result.manifest_cid.unwrap()));
    println!("   {} chunks uploaded", result.num_chunks);

    Ok(())
}
```

## Two-Step Approach (Advanced)

For more control, prepare chunks separately:

```rust
use bulletin_sdk_rust::client::BulletinClient;

// Step 1: Prepare chunks locally
let client = BulletinClient::new();
let (batch, manifest_data) = client.prepare_store_chunked(
    &data,
    Some(config),
    StoreOptions::default(),
    Some(progress),
)?;

// Step 2: Submit manually
for operation in batch.operations {
    // Submit operation.data with your own method
    let receipt = custom_submit(operation.data).await?;
}

if let Some(manifest_bytes) = manifest_data {
    let receipt = custom_submit(manifest_bytes).await?;
}
```

## Authorization Checking (Fail Fast)

For large chunked uploads, authorization checking is especially important to avoid wasting time uploading many chunks only to fail at the end.

### Automatic Checking

By default, the SDK checks authorization before starting any chunk uploads:

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ws_url = std::env::var("BULLETIN_WS_URL")
        .unwrap_or_else(|_| "ws://localhost:10000".to_string());
    let signer = /* your signer */;
    let account = /* your AccountId32 */;

    // Create client with account for authorization checking
    let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
    let client = AsyncBulletinClient::new(submitter)
        .with_account(account);

    // Load large file
    let data = std::fs::read("large-file.bin")?;
    println!("File size: {} bytes", data.len());

    // Configure chunking
    let config = ChunkerConfig {
        chunk_size: 1024 * 1024,  // 1 MiB
        max_parallel: 8,
        create_manifest: true,
    };

    // Upload - authorization is checked BEFORE uploading any chunks
    let result = client
        .store_chunked(&data, Some(config), StoreOptions::default(), None)
        .await?;
    //           ⬆️ Fails immediately if insufficient authorization
    //              No chunks uploaded if auth is insufficient!

    println!("✅ Success! Manifest CID: {}", hex::encode(result.manifest_cid.unwrap()));
    Ok(())
}
```

### What Gets Checked

Before uploading **any** chunks, the SDK:
1. **Calculates** total requirements:
   - Number of transactions = number of chunks + 1 (for manifest)
   - Total bytes = file size + estimated manifest size
2. **Queries** blockchain for current authorization
3. **Validates** sufficient transactions and bytes are authorized
4. **Fails immediately** if insufficient (saves uploading time!)
5. **Proceeds** only if authorization is sufficient

### Estimate Before Upload

Check authorization requirements before starting:

```rust
// Estimate what's needed
let file_size = std::fs::metadata("large-file.bin")?.len();
let (txs_needed, bytes_needed) = client.estimate_authorization(file_size, true);

println!("This upload will need:");
println!("  {} transactions", txs_needed);
println!("  {} bytes authorized", bytes_needed);

// For 100 MB file with 1 MiB chunks:
// - 100 chunk transactions
// - 1 manifest transaction
// - Total: 101 transactions, ~100 MB authorized
```

### Handle Insufficient Authorization

```rust
match client.store_chunked(&data, config, options, None).await {
    Ok(result) => {
        println!("✅ Uploaded {} chunks", result.num_chunks);
        println!("   Manifest: {}", hex::encode(result.manifest_cid.unwrap()));
    }
    Err(Error::InsufficientAuthorization { need, available }) => {
        eprintln!("❌ Insufficient authorization:");
        eprintln!("   Need: {} bytes and {} transactions", need, /* calculate txs */);
        eprintln!("   Have: {} bytes available", available);
        eprintln!("\n💡 Authorize your account first:");
        eprintln!("   client.authorize_account(account, {}, {}).await?",
                  txs_needed, bytes_needed);
    }
    Err(e) => {
        eprintln!("❌ Error: {:?}", e);
    }
}
```

### Complete Example with Authorization

```rust
use bulletin_sdk_rust::prelude::*;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup
    let ws_url = std::env::var("BULLETIN_WS_URL")
        .unwrap_or_else(|_| "ws://localhost:10000".to_string());
    let signer = /* your signer */;
    let account = /* your AccountId32 */;

    let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
    let client = AsyncBulletinClient::new(submitter)
        .with_account(account.clone());

    // Load file
    let file_path = Path::new("large-video.mp4");
    let data = std::fs::read(file_path)?;
    println!("📁 File: {} ({} bytes)", file_path.display(), data.len());

    // Estimate authorization needed
    let (txs_needed, bytes_needed) = client.estimate_authorization(data.len() as u64);
    println!("\n📊 Authorization Required:");
    println!("   Transactions: {}", txs_needed);
    println!("   Bytes: {}", bytes_needed);

    // Check if we need to authorize
    // (In real code, query current auth state first)
    println!("\n🔐 Authorizing account...");
    client.authorize_account(account, txs_needed, bytes_needed).await?;
    println!("✅ Authorization complete");

    // Configure chunking
    let config = ChunkerConfig {
        chunk_size: 1024 * 1024,  // 1 MiB
        max_parallel: 8,
        create_manifest: true,
    };

    // Upload with authorization checking (enabled by default)
    println!("\n⬆️  Uploading...");
    let start = std::time::Instant::now();

    let result = client
        .store_chunked(
            &data,
            Some(config),
            StoreOptions::default(),
            Some(|event| {
                if let ProgressEvent::ChunkCompleted { index, total, .. } = event {
                    let percent = ((index + 1) as f64 / total as f64) * 100.0;
                    print!("\r   Progress: {:.1}%", percent);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                }
            }),
        )
        .await?;

    let duration = start.elapsed();
    println!("\n\n✅ Upload complete in {:.2}s", duration.as_secs_f64());
    println!("   Manifest CID: {}", hex::encode(result.manifest_cid.unwrap()));
    println!("   {} chunks uploaded", result.num_chunks);

    Ok(())
}
```

## Best Practices

1. **Check authorization first** - Use `.with_account()` to enable automatic checking before upload
2. **Estimate requirements** - Call `client.estimate_authorization(file_size)` before large uploads
3. **Choose appropriate chunk size** - 1 MiB is a good default
4. **Enable progress tracking** - Show users what's happening
5. **Handle failures gracefully** - Check for `InsufficientAuthorization` errors
6. **Keep manifest CID** - Use it to retrieve the complete file
7. **Test with MockSubmitter** - Fast tests without a node

## Next Steps

- [Authorization](./authorization.md) - Manage storage authorization
- [Transaction Submitters](./submitters.md) - Custom submitter implementations
- [Basic Storage](./basic-storage.md) - For small files < 8 MiB
