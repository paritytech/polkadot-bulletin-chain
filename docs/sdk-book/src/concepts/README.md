# Core Concepts

## Authorize -> Store Flow

Bulletin Chain requires two steps to store data:

1. **Authorize**: Root (sudo) reserves space for an account or preimage. No fees.
2. **Store**: The authorized user uploads data. Transaction fees apply.

The SDKs calculate required authorization and handle this flow automatically.

## Data Limits

- **Max transaction size**: ~8 MiB
- **Recommended chunk size**: 1 MiB
- Files larger than the transaction limit must be chunked. The SDKs handle this automatically.

## CIDs (Content Identifiers)

Bulletin Chain uses IPFS-style CIDs to identify data. A CID encodes the codec (e.g. `0x55` Raw, `0x70` DAG-PB) and hash algorithm (e.g. blake2b-256). When you store data, the chain records the CID as proof that this data existed at a specific block.
