# Chunked Uploads

The Bulletin SDK automatically handles chunking for large files up to **64 MiB**. When you call `store()`, files larger than the threshold (default 2 MiB) are automatically split into chunks of up to **2 MiB** each (matching the Bitswap block size limit for IPFS compatibility).

## Automatic Chunking (Recommended)

For most use cases, simply use `store()` - it automatically chunks large files:

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';

const client = new AsyncBulletinClient(api, signer, papiClient.submit);

// Load file of any size (up to 64 MiB)
const data = new Uint8Array(50 * 1024 * 1024); // 50 MB

// Automatically chunks if > 2 MiB
const result = await client
    .store(data)
    .withCallback((event) => {
        if (event.type === 'chunk_completed') {
            console.log(`Chunk ${event.index + 1}/${event.total} uploaded`);
        } else if (event.type === 'completed') {
            console.log('Done!');
        }
    })
    .send();

console.log('Stored with CID:', result.cid.toString());
if (result.chunks) {
    console.log('Chunked into', result.chunks.numChunks, 'pieces');
}
```

### Configuring Automatic Chunking

You can configure the threshold and chunk size via the client constructor:

```typescript
const client = new AsyncBulletinClient(api, signer, papiClient.submit, {
    chunkingThreshold: 5 * 1024 * 1024,   // Chunk files > 5 MiB
    defaultChunkSize: 1024 * 1024,         // 1 MiB chunks (max: 2 MiB)
    createManifest: true,                  // Create DAG-PB manifest
});
```

## Advanced: Manual Chunking

For advanced use cases where you need explicit control over chunking parameters, use `storeChunked()`:

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';

const client = new AsyncBulletinClient(api, signer, papiClient.submit);

const largeFile = new Uint8Array(50 * 1024 * 1024); // 50 MB

// Configure chunking explicitly
const config = {
    chunkSize: 1024 * 1024,  // 1 MiB chunks
    createManifest: true,     // Create DAG-PB manifest
};

// Progress tracking
const progressCallback = (event) => {
    switch (event.type) {
        case 'chunk_started':
            console.log(`Uploading chunk ${event.index + 1}/${event.total}`);
            break;
        case 'chunk_completed':
            console.log(`✓ Chunk ${event.index + 1}/${event.total} complete:`, event.cid.toString());
            break;
        case 'chunk_failed':
            console.error(`✗ Chunk ${event.index + 1}/${event.total} failed:`, event.error);
            break;
        case 'manifest_created':
            console.log('📦 Manifest created:', event.cid.toString());
            break;
        case 'completed':
            if (event.manifestCid) {
                console.log('✅ All done! Manifest CID:', event.manifestCid.toString());
            }
            break;
    }
};

// Upload with manual chunking configuration and progress tracking
const result = await client.storeChunked(
    largeFile,
    config,
    undefined, // default store options
    progressCallback
);

console.log('\n📊 Upload Summary:');
console.log('   Total size:', result.totalSize, 'bytes');
console.log('   Chunks:', result.numChunks);
console.log('   Chunk CIDs:', result.chunkCids.length, 'items');
if (result.manifestCid) {
    console.log('   Manifest CID:', result.manifestCid.toString());
}
```

## How It Works

The `storeChunked()` method:

1. **Splits data** into chunks (default 1 MiB)
2. **Calculates CIDs** for each chunk
3. **Submits chunks** sequentially or in parallel
4. **Creates DAG-PB manifest** linking all chunks
5. **Submits manifest** as final transaction
6. **Returns result** with all CIDs

### Upload Timing

Bulletin Chain has a **6 second block time**. Multiple chunks can fit into a single block. Sequential upload timing depends on how quickly transactions are included:

| File Size | Chunk Size | Chunks | Transactions |
|-----------|------------|--------|--------------|
| 2 MiB | 1 MiB | 2 | 3 (2 data + manifest) |
| 10 MiB | 1 MiB | 10 | 11 |
| 50 MiB | 2 MiB | 25 | 26 |
| 64 MiB | 2 MiB | 32 | 33 |

**Note**: Actual times depend on network conditions and the `waitFor` setting (`"in_block"` vs `"finalized"`).

### When to Use `storeChunked()` vs `store()`

**Use `store()` (recommended):**
- For most use cases - it automatically handles everything
- When you don't need detailed chunk information
- For both small and large files

**Use `storeChunked()` (advanced):**
- When you need detailed control over chunking parameters
- When you need the full `ChunkedStoreResult` with all chunk CIDs
- When you want to force chunking on small files
- For testing or debugging chunking behavior

**Key Difference:**
- `store()` returns `StoreResult` with optional chunk info
- `storeChunked()` returns `ChunkedStoreResult` with detailed chunk information

## Configuration Options

### Chunk Size

```typescript
const config = {
    chunkSize: 2 * 1024 * 1024,  // 2 MiB chunks (MAX_CHUNK_SIZE)
    createManifest: true,
};
```

**Guidelines:**
- Minimum: 1 byte (practical minimum: 1 KiB)
- Maximum: 2 MiB (2,097,152 bytes) - matches Bitswap block size limit for IPFS compatibility
- Default: 1 MiB - good balance of efficiency and compatibility
- Maximum file size: 64 MiB (MAX_FILE_SIZE)

### Parallel Uploads

```typescript
const config = {
    chunkSize: 1024 * 1024,
    createManifest: true,
};
```

**Note**: Current implementation uploads sequentially. Parallel support is planned for a future release.

### Manifest Creation

```typescript
// With manifest (recommended for large files)
const config = {
    chunkSize: 1024 * 1024,
    createManifest: true,  // Creates DAG-PB manifest
};

// Without manifest (just upload chunks)
const config = {
    chunkSize: 1024 * 1024,
    createManifest: false,  // No manifest, just chunks
};
```

## Progress Tracking

Track upload progress with callbacks:

```typescript
const progress = (event) => {
    switch (event.type) {
        case 'chunk_started':
            console.log(`[${event.index + 1}/${event.total}] Starting chunk...`);
            break;
        case 'chunk_completed':
            console.log(`[${event.index + 1}/${event.total}] ✓ Uploaded:`, event.cid.toString());
            break;
        case 'chunk_failed':
            console.error(`[${event.index + 1}/${event.total}] ✗ Failed:`, event.error.message);
            break;
        case 'manifest_started':
            console.log('Creating manifest...');
            break;
        case 'manifest_created':
            console.log('Manifest CID:', event.cid.toString());
            break;
        case 'completed':
            if (event.manifestCid) {
                console.log('All done! Manifest:', event.manifestCid.toString());
            }
            break;
    }
};

const result = await client
    .store(largeData)
    .withCallback(progress)
    .send();
```

## Authorization Estimation

Before uploading, estimate the authorization you'll need:

```typescript
const largeData = new Uint8Array(50 * 1024 * 1024); // 50 MB
const estimate = client.estimateAuthorization(largeData.length);
console.log('Need:', estimate.transactions, 'txs,', estimate.bytes, 'bytes');
```

### Error Handling

```typescript
import { BulletinError } from '@bulletin/sdk';

try {
    const result = await client.store(largeData).send();
    console.log('Success!');
} catch (error) {
    if (error instanceof BulletinError) {
        console.error('Error:', error.code, error.message);
    }
}
```

## Complete Example with Authorization

```typescript
import { AsyncBulletinClient, BulletinError } from '@bulletin/sdk';

const client = new AsyncBulletinClient(api, signer, papiClient.submit);

// Large file (up to 64 MiB)
const largeFile = new Uint8Array(50 * 1024 * 1024); // 50 MB

// Estimate authorization needed
const estimate = client.estimateAuthorization(largeFile.length);
console.log('Authorization needed:');
console.log('   Transactions:', estimate.transactions);
console.log('   Bytes:', estimate.bytes);

// Authorize (if needed - requires sudo)
const account = 'your-account-address';
// await client.authorizeAccount(account, estimate.transactions, BigInt(estimate.bytes), { sudo: true });

try {
    // Upload with progress tracking
    const result = await client
        .store(largeFile)
        .withCallback((event) => {
            if (event.type === 'chunk_completed') {
                console.log(`Chunk ${event.index + 1}/${event.total} done`);
            }
        })
        .send();

    console.log('Upload complete!');
    console.log('   CID:', result.cid.toString());
    if (result.chunks) {
        console.log('   Chunks:', result.chunks.numChunks);
    }
} catch (error) {
    if (error instanceof BulletinError) {
        console.error('Error:', error.code, error.message);
    } else {
        console.error('Error:', error);
    }
}
```
