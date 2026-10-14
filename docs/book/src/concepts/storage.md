# Storage Models

The SDK supports two primary modes of operation.

## Simple Storage (Direct)

For small data (up to one chunk, 1 MiB by default; the chain accepts up to 2 MiB per transaction), the data is stored directly in a single transaction.

- **Pros**: Simple, atomic (all or nothing).
- **Cons**: Limited by block size and transaction size limits.
- **Underlying Call**: `TransactionStorage.store(data)`, or `TransactionStorage.store_with_cid_config(cid, data)` for a non-default codec or hash

The SDK calculates the CID for you (Rust offline API: `BulletinClient::prepare_store` returns a `StorageOperation`).

## Chunked Storage (DAG-PB)

For larger files, the data is split into a **Merkle DAG** (Directed Acyclic Graph).

1.  **Chunking**: The file is split into 1 MiB chunks.
2.  **Upload**: Each chunk is uploaded as a separate transaction.
3.  **Manifest**: A "Manifest" node is created. This is a small Protobuf message that lists the links (CIDs) to all the chunks.
4.  **Finalize**: The Manifest is uploaded last. Its CID represents the *entire file*.

### Why DAG-PB?

We use the **DAG-PB** (UnixFS) standard for chunked data:
- **Content addressing**: Each chunk and the manifest have unique CIDs based on their content
- **Merkle verification**: Clients can verify data integrity without downloading everything
- **Parallel retrieval**: Chunks can be fetched independently and in parallel

The SDK manages this complexity for you:
- Rust: `TransactionClient::estimate_upload` → `submit` (offline preparation: `BulletinClient::prepare_store_chunked`)
- TypeScript: `client.submit(await client.estimateUpload(src), src).send()` (offline preparation: `new BulletinPreparer().prepareStoreChunked(data)`)
