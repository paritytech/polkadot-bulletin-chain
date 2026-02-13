# Storage Models

## Simple Storage

For data < 8 MiB: a single `TransactionStorage.store(data)` call. Atomic.

## Chunked Storage (DAG-PB)

For larger files, the SDK:

1. Splits data into 1 MiB chunks
2. Uploads each chunk as a separate transaction
3. Creates a DAG-PB manifest listing all chunk CIDs
4. Uploads the manifest last - its CID represents the entire file

The manifest uses the DAG-PB (UnixFS) standard from IPFS, so the root CID matches what `ipfs add` would produce and chunks can be retrieved via standard IPFS tools.
