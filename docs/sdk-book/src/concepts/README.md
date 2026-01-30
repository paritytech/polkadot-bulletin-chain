# Core Concepts

To effectively use the SDKs, it's helpful to understand a few underlying concepts of the Bulletin Chain.

## The "Authorize -> Store" Flow

To prevent spam and manage storage growth, Bulletin Chain requires a two-step process for storing data:

1.  **Authorize**: A user reserves space on the chain. This costs tokens (fees) based on the size of the data and the number of transactions required.
2.  **Store**: The user (or anyone with the data) uploads the actual data. This transaction is free (or very cheap) because the space was already paid for.

The SDKs help you calculate the required authorization and track the status of your uploads.

## Data Limits

- **Max Transaction Size**: ~8 MiB (typical Substrate limit).
- **Recommended Chunk Size**: 1 MiB.

If your file is larger than the transaction limit, it *must* be split into chunks. The SDKs handle this automatically.

## CIDs (Content Identifiers)

Bulletin Chain uses CIDs to identify data. A CID is a self-describing label used in IPFS.
- **Codec**: Describes the format (e.g., `0x55` for Raw, `0x70` for DAG-PB).
- **Multihash**: Describes the hash algorithm (e.g., `blake2b-256`, `sha2-256`).

When you store data, the chain records the CID. This proves that *this specific data* existed at *this specific block number*.
