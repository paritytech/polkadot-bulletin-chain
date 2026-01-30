# Chunked Uploads

The Bulletin SDK automatically handles chunking for large files. When you call `store()`, files larger than the threshold (default 2 MiB) are automatically split into chunks.

## Automatic Chunking (Recommended)

For most use cases, simply use `store()` - it automatically chunks large files:

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';

const submitter = new PAPITransactionSubmitter(api, signer);
const client = new AsyncBulletinClient(submitter);

// Load file of any size
const data = new Uint8Array(50 * 1024 * 1024); // 50 MB

// Automatically chunks if > 2 MiB
const result = await client.store(data, undefined, (event) => {
    if (event.type === 'chunk_completed') {
        console.log(`Chunk ${event.index + 1}/${event.total} uploaded`);
    } else if (event.type === 'completed') {
        console.log('Done!');
    }
});

console.log('Stored with CID:', result.cid.toString());
if (result.chunks) {
    console.log('Chunked into', result.chunks.numChunks, 'pieces');
}
```

### Configuring Automatic Chunking

You can configure the threshold and chunk size:

```typescript
const client = new AsyncBulletinClient(submitter, {
    chunkingThreshold: 5 * 1024 * 1024,  // Chunk files > 5 MiB
    defaultChunkSize: 2 * 1024 * 1024,   // 2 MiB chunks
    maxParallel: 8,                       // Upload 8 chunks in parallel
    createManifest: true,                 // Create DAG-PB manifest
    checkAuthorizationBeforeUpload: true, // Check auth before upload
});
```

## Advanced: Manual Chunking

For advanced use cases where you need detailed control over chunking, use `storeChunked()`:

```typescript
import { AsyncBulletinClient } from '@bulletin/sdk';

const submitter = new PAPITransactionSubmitter(api, signer);
const client = new AsyncBulletinClient(submitter);

const largeFile = new Uint8Array(100 * 1024 * 1024); // 100 MB

// Configure chunking
const config = {
    chunkSize: 1024 * 1024,  // 1 MiB chunks
    maxParallel: 8,           // Upload 8 chunks in parallel
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

// Upload with automatic chunking and progress tracking
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

### When to Use `storeChunked()` vs `store()`

**Use `store()` (recommended):**
- ✅ For most use cases - it automatically handles everything
- ✅ When you don't need detailed chunk information
- ✅ For both small and large files

**Use `storeChunked()` (advanced):**
- ⚙️ When you need detailed control over chunking parameters
- ⚙️ When you need the full `ChunkedStoreResult` with all chunk CIDs
- ⚙️ When you want to force chunking on small files
- ⚙️ For testing or debugging chunking behavior

**Key Difference:**
- `store()` returns `StoreResult` with optional chunk info
- `storeChunked()` returns `ChunkedStoreResult` with detailed chunk information

## Configuration Options

### Chunk Size

```typescript
const config = {
    chunkSize: 2 * 1024 * 1024,  // 2 MiB chunks (max is 2 MiB for Bitswap)
    maxParallel: 4,
    createManifest: true,
};
```

**Guidelines:**
- Minimum: 1 MiB (1,048,576 bytes)
- Maximum: 2 MiB (2,097,152 bytes) - Bitswap compatibility limit
- Default: 1 MiB - good balance of efficiency and compatibility

### Parallel Uploads

```typescript
const config = {
    chunkSize: 1024 * 1024,
    maxParallel: 8,  // Upload up to 8 chunks simultaneously
    createManifest: true,
};
```

**Note**: Current implementation uploads sequentially. Parallel support is planned for a future release.

### Manifest Creation

```typescript
// With manifest (IPFS-compatible, recommended)
const config = {
    chunkSize: 1024 * 1024,
    maxParallel: 8,
    createManifest: true,  // Creates DAG-PB manifest
};

// Without manifest (just upload chunks)
const config = {
    chunkSize: 1024 * 1024,
    maxParallel: 8,
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

const result = await client.store(largeData, undefined, progress);
```

## Authorization Checking (Fail Fast)

By default, the SDK checks authorization **before** uploading chunked data to avoid wasted time and fees.

### How It Works

```typescript
// 1. Create client with account
const client = new AsyncBulletinClient(submitter)
    .withAccount(account);  // Enable automatic authorization checking

// 2. Upload large file - authorization checked automatically
const largeData = new Uint8Array(50 * 1024 * 1024);
const result = await client.store(largeData);
//   ⬆️ Internally:
//   - Queries authorization (if account set)
//   - Checks expiration (if expires_at present)
//   - Validates sufficient bytes and transactions
//   - Fails with AuthorizationExpired or InsufficientAuthorization
//   - Only proceeds if all checks pass
```

### Error Handling

```typescript
try {
    const result = await client.store(largeData);
    console.log('Success!');
} catch (error) {
    if (error.code === 'INSUFFICIENT_AUTHORIZATION') {
        console.error('Need more authorization for', largeData.length, 'bytes');

        // Estimate what's needed
        const estimate = client.estimateAuthorization(largeData.length);
        console.error('Need:', estimate.transactions, 'txs,', estimate.bytes, 'bytes');
    } else if (error.code === 'AUTHORIZATION_EXPIRED') {
        console.error('Authorization expired - please refresh');
    }
}
```

## Complete Example with Authorization

```typescript
import { AsyncBulletinClient, PAPITransactionSubmitter } from '@bulletin/sdk';

const submitter = new PAPITransactionSubmitter(api, signer);
const account = 'your-account-address';
const client = new AsyncBulletinClient(submitter)
    .withAccount(account);

// Large file
const largeFile = new Uint8Array(100 * 1024 * 1024); // 100 MB

// Estimate authorization needed
const estimate = client.estimateAuthorization(largeFile.length);
console.log('📊 Authorization needed:');
console.log('   Transactions:', estimate.transactions);
console.log('   Bytes:', estimate.bytes);

// Authorize (if needed - requires sudo)
// await client.authorizeAccount(account, estimate.transactions, BigInt(estimate.bytes));

try {
    // Upload with progress tracking
    const result = await client.store(largeFile, undefined, (event) => {
        if (event.type === 'chunk_completed') {
            console.log(`Chunk ${event.index + 1}/${event.total} ✓`);
        }
    });

    console.log('✅ Upload complete!');
    console.log('   CID:', result.cid.toString());
    if (result.chunks) {
        console.log('   Chunks:', result.chunks.numChunks);
    }
} catch (error) {
    if (error.code === 'INSUFFICIENT_AUTHORIZATION') {
        console.error('❌ Insufficient authorization');
        console.error('   Please authorize your account first');
    } else if (error.code === 'AUTHORIZATION_EXPIRED') {
        console.error('❌ Authorization expired');
        console.error('   Please refresh your authorization');
    } else {
        console.error('❌ Error:', error.message);
    }
}
```
