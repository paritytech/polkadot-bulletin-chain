# Core Concepts

To effectively use the SDKs, it's helpful to understand a few underlying concepts of the Bulletin Chain.

## The "Authorize -> Store" Flow

To prevent spam and manage storage growth, Bulletin Chain requires a two-step process for storing data:

1.  **Authorize**: A Root user (sudo) reserves space for an account or preimage. This call must be made by Root and doesn't cost any fees. It grants permission to store a specified amount of data.
2.  **Store**: The authorized user uploads the actual data. The storage transaction costs fees, but the authorization ensures the user has permission to use that chain space.

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
