# DAG-PB Manifests

## What is a Manifest?

In the context of the SDK, a **Manifest** is a small data structure that describes how to reassemble a large file from its chunks. We use [**DAG-PB**](https://ipld.io/specs/codecs/dag-pb/) (Merkle DAG Protobuf), a standard content-addressed format.

When you upload a large file using the SDK:
1. The file is split into chunks (leaves).
2. Each chunk is hashed and uploaded.
3. A "Root Node" (the manifest) is created. It contains:
   - `Links`: A list of CIDs pointing to the chunks.
   - `Data`: UnixFS metadata (file size, type).

## Why DAG-PB?

- **Content Addressing**: Each chunk and the manifest have unique CIDs based on their content
- **Merkle Tree Structure**: Enables efficient verification and partial retrieval
- **Standardized Format**: Well-documented specification with broad tooling support
- **Efficient Retrieval**: Clients can fetch chunks in parallel once they have the manifest

## Manifest Structure

A simplified view of a manifest node:

```protobuf
message PBNode {
  repeated PBLink Links = 2;
  optional bytes Data = 1;
}

message PBLink {
  optional bytes Hash = 1; // CID of the chunk
  optional string Name = 2;
  optional uint64 Tsize = 3; // Size of the chunk
}
```

The SDKs include a `DagBuilder` (Rust) or `UnixFsDagBuilder` (TS) that constructs this binary format for you.

## Example: Chunked File Structure

```
                    ┌─────────────────┐
                    │   Root Manifest │
                    │   (DAG-PB node) │
                    └────────┬────────┘
                             │
            ┌────────────────┼────────────────┐
            │                │                │
            ▼                ▼                ▼
      ┌──────────┐    ┌──────────┐    ┌──────────┐
      │ Chunk 0  │    │ Chunk 1  │    │ Chunk 2  │
      │ (raw)    │    │ (raw)    │    │ (raw)    │
      └──────────┘    └──────────┘    └──────────┘
```

The root manifest contains links to all chunks with their sizes, allowing clients to:
- Verify the total file size before downloading
- Fetch chunks in parallel
- Resume interrupted downloads
- Verify integrity of each chunk independently

## SDK Usage

### TypeScript

```typescript
import { BulletinClient } from "@bulletin/sdk";

const client = new BulletinClient();
const largeFile = new Uint8Array(10_000_000); // 10 MB

const { operations, manifest } = client.prepareStoreChunked(largeFile, {
  chunkSize: 1024 * 1024, // 1 MiB chunks
  createManifest: true,
});

// operations contains the individual chunk store operations
// manifest contains the DAG-PB root node to submit last
```

### Rust

```rust
use bulletin_sdk_rust::prelude::*;

let client = BulletinClient::new();
let large_file = vec![0u8; 10_000_000]; // 10 MB

let config = ChunkerConfig {
    chunk_size: 1024 * 1024, // 1 MiB
    create_manifest: true,
    ..Default::default()
};

let (batch, manifest) = client.prepare_store_chunked(
    &large_file,
    Some(config),
    StoreOptions::default(),
    None,
)?;
```

## Next Steps

- [Storage Model](./storage.md) - How data is stored on-chain
- [Data Retrieval](./retrieval.md) - How to retrieve chunked data
- [Chunked Uploads (Rust)](../rust/chunked-uploads.md) - Detailed Rust guide
- [Chunked Uploads (TypeScript)](../typescript/chunked-uploads.md) - Detailed TypeScript guide
