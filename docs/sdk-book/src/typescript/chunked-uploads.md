# Chunked Uploads

```typescript
import { BulletinClient } from '@bulletin/sdk';

const client = new BulletinClient({
    endpoint: 'ws://localhost:9944'
});

const largeFile = new Uint8Array(10 * 1024 * 1024); // 10 MB

const { chunks, manifest } = await client.prepareStoreChunked(
    largeFile,
    undefined, // default config
    undefined, // default options
    (event) => {
        console.log('Progress:', event.type);
    }
);

// Submit chunks
for (const chunk of chunks) {
    // Submit chunk.data via PAPI
    console.log(`Chunk ${chunk.index} CID: ${chunk.cid}`);
}

// Submit manifest
if (manifest) {
    // Submit manifest.data via PAPI
    console.log('Manifest CID:', manifest.cid);
}
```
