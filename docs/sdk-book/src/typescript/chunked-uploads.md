# Chunked Uploads

Files larger than 2 MiB are automatically chunked when using `store()`. Use `storeChunked()` for explicit control.

> `store().send()` is not yet fully implemented. Use `storeChunked()` for CID/manifest generation, then submit via PAPI directly.

## Automatic Chunking

```typescript
const data = new Uint8Array(50 * 1024 * 1024); // 50 MB

const result = await client
    .store(data)
    .withCallback((event) => {
        if (event.type === 'chunk_completed') {
            console.log(`Chunk ${event.index + 1}/${event.total}`);
        }
    })
    .send();
```

### Configuration

```typescript
const client = new AsyncBulletinClient(api, signer, {
    chunkingThreshold: 5 * 1024 * 1024,  // chunk files > 5 MiB
    defaultChunkSize: 2 * 1024 * 1024,   // 2 MiB chunks
    createManifest: true,
    checkAuthorizationBeforeUpload: true,
});
```

## Manual Chunking

```typescript
const config = {
    chunkSize: 1024 * 1024,  // 1 MiB
    maxParallel: 8,
    createManifest: true,
};

const result = await client.storeChunked(largeFile, config);
console.log('Chunks:', result.numChunks);
console.log('Manifest CID:', result.manifestCid?.toString());
```

Chunk size guidelines: 1 MiB (min) to 2 MiB (max, Bitswap limit). Default 1 MiB.

Authorization checking works the same as [basic storage](./basic-storage.md#authorization-checking) - use `.withAccount()` to enable pre-flight checks.
