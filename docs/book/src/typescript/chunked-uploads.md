# Chunked Uploads

When you estimate a `BlobSource`, the SDK streams it once, splits it into chunks of **2 MiB** by default (also the maximum: the chain's per-transaction limit and the Bitswap block-size limit for IPFS compatibility), and builds a DAG-PB manifest linking them. `submit()` then stores each chunk plus the manifest, fetching chunk bytes on demand so the whole file never sits in memory at once.

Data up to 2 MiB is a single chunk and needs no manifest — its own CID is the retrieval id.

## Uploading a File

```typescript
import { BulletinClient, blobFromBytes, UploadStatus } from '@parity/bulletin-sdk';
import { getWsProvider } from 'polkadot-api/ws';

const client = new BulletinClient({
  providers: () => [getWsProvider('ws://localhost:9944')],
  uploadSigner: signer,
});

const src = blobFromBytes(new Uint8Array(50 * 1024 * 1024)); // 50 MB

const { cids } = await client
  .submit(await client.estimateUpload(src), src)
  .withCallback((ev) => {
    if (ev.type === UploadStatus.ItemFinalized) {
      console.log(`item ${ev.index + 1}/${ev.total} finalized @#${ev.blockNumber}`);
    }
  })
  .send();

// One CID per chunk plus the manifest last; that last CID is the file's root.
const rootCid = cids[cids.length - 1];
console.log('Stored', cids.length, 'units, root', rootCid.toString());
```

## Reading from a File Without Buffering

`blobFromBytes` holds the whole buffer in memory. `submit()` instead takes a `SeekableSource` — it fetches each chunk lazily by `read(offset, length)` and frees it on finalization — so a file-backed source keeps only the in-flight chunks resident (a wave spans up to about 2 blocks' worth of chunks):

```typescript
import { open } from 'node:fs/promises';
import { statSync } from 'node:fs';
import type { SeekableSource } from '@parity/bulletin-sdk';

function fileSource(path: string): SeekableSource {
  const size = statSync(path).size;
  return {
    size,
    // Forward read, used by estimateUpload to hash the file.
    async *open() {
      const fh = await open(path);
      try {
        for await (const chunk of fh.createReadStream()) yield chunk as Uint8Array;
      } finally {
        await fh.close();
      }
    },
    // Random access, used by submit to fetch one chunk at a time.
    async read(offset, length) {
      const fh = await open(path);
      try {
        const buf = new Uint8Array(length);
        await fh.read(buf, 0, length, offset);
        return buf;
      } finally {
        await fh.close();
      }
    },
  };
}

const src = fileSource('big.bin');
const { cids } = await client.submit(await client.estimateUpload(src), src).send();
```

For estimation-only or offline planning you can also wrap a re-openable forward stream with `blobFromFactory(() => createReadStream(path))`; that yields a plain `BlobSource` (no random access), so it works with `estimateUpload` but not `submit`.

## Chunk Size

The chunk size comes from the client config (`defaultChunkSize`, 2 MiB by default):

```typescript
const client = new BulletinClient({
  providers: () => [getWsProvider(url)],
  uploadSigner: signer,
  defaultChunkSize: 1024 * 1024, // 1 MiB
});
```

Guidelines:
- Maximum: 2 MiB (`MAX_CHUNK_SIZE`) — the chain's per-transaction limit and the Bitswap block-size limit.
- Default: 2 MiB, the maximum.
- `MAX_FILE_SIZE` (64 MiB) applies only to `FixedSizeChunker` / `BulletinPreparer.prepareStoreChunked`; `estimateUpload` + `submit` stream sources of any size.

## Sizing Authorization

A chunked file consumes one transaction per chunk plus one for the manifest. `estimateUpload` reports the exact figures:

```typescript
const estimate = await client.estimateUpload(src);
console.log('Need', estimate.transactions, 'txs and', estimate.bytes, 'bytes');
```

| File Size | Chunk Size | Chunks | Transactions |
|-----------|------------|--------|--------------|
| 2 MiB | 2 MiB | 1 | 1 (no manifest) |
| 10 MiB | 2 MiB | 5 | 6 |
| 50 MiB | 2 MiB | 25 | 26 |
| 64 MiB | 2 MiB | 32 | 33 |

## Progress

The callback receives an `UploadEvent` per unit. `index` is the unit's position (chunks first, manifest last); `total` is the unit count.

```typescript
import { UploadStatus } from '@parity/bulletin-sdk';

const chunkCids: string[] = [];
await client
  .submit(estimate, src)
  .withCallback((ev) => {
    switch (ev.type) {
      case UploadStatus.ItemStarted:
        console.log(`[${ev.index + 1}/${ev.total}] started`);
        break;
      case UploadStatus.ItemInBlock:
        console.log(`[${ev.index + 1}/${ev.total}] in block #${ev.blockNumber}`);
        break;
      case UploadStatus.ItemFinalized:
        // The last unit is the manifest; everything before is a chunk.
        if (ev.index < ev.total - 1) chunkCids.push(ev.cid.toString());
        break;
      case UploadStatus.ItemFailed:
        console.error(`[${ev.index + 1}/${ev.total}] failed:`, ev.error.message);
        break;
    }
  })
  .send();
```

## Atomicity

Chunked uploads are **not atomic**. If submission fails partway, chunks already stored remain on chain. Within one `send()`, the SDK retries transient stalls and skips what already landed (so it never double-stores), but a hard failure leaves the earlier chunks in place. A new `submit()` stores every chunk again unless the estimate was made with `{ skipExisting: true }`, which leaves out the chunks already on chain.
