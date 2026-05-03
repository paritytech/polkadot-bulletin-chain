# Storage Models

The SDK supports two primary modes of operation.

## Simple Storage (Direct)

For small data (less than 8 MiB), you can store the data directly in a single transaction.

- **Pros**: Simple, atomic (all or nothing).
- **Cons**: Limited by block size and transaction size limits.
- **Underlying Call**: `TransactionStorage.store(data)`

The SDK calculates the CID for you and wraps the data in a `StorageOperation`.

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
- `client.prepare_store_chunked` (Rust)
- `client.prepareStoreChunked` (TS)
