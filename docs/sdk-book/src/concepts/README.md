# Core Concepts

This section covers the fundamental concepts you need to understand when working with Bulletin Chain.

## Data Lifecycle

```
1. AUTHORIZE    2. STORE        3. RETRIEVE     4. RENEW (optional)
   ↓               ↓               ↓               ↓
Grant storage   Submit data    Fetch via      Extend before
permission      to chain       IPFS           expiration
```

### 1. Authorization

Before storing data, accounts must be **authorized**. This prevents spam and manages storage growth.

- A Root/Sudo user grants permission to store a specified amount of data
- Authorization can be for an account or a specific content hash (preimage)
- Learn more: [Authorization](./authorization.md)

### 2. Storage

Once authorized, users can submit data to the chain:

- **Small data** (< 8 MiB): Stored directly in a single transaction
- **Large data** (> 8 MiB): Split into chunks with a DAG-PB manifest
- The chain returns a **CID** (Content Identifier) for retrieval
- Learn more: [Storage Model](./storage.md)

### 3. Retrieval

Data is retrieved via **IPFS**, not directly from the chain:

- Use any IPFS gateway: `https://ipfs.io/ipfs/{cid}`
- Connect directly to Bulletin nodes via Bitswap protocol
- Chunked data is automatically reassembled by IPFS
- Learn more: [Data Retrieval](./retrieval.md)

### 4. Renewal

Data has a **retention period** after which it may be pruned:

- Call `renew(block, index)` to extend the retention period
- Track the block number and index from `Stored`/`Renewed` events
- Learn more: [Data Renewal](./renewal.md)

## CIDs (Content Identifiers)

Bulletin Chain uses **CIDs** to identify data. A CID is a self-describing label used in IPFS:

| Component | Description | Example |
|-----------|-------------|---------|
| **Version** | CID version | `1` (CIDv1) |
| **Codec** | Data format | `0x55` (Raw), `0x70` (DAG-PB) |
| **Multihash** | Hash algorithm | `blake2b-256`, `sha2-256` |

When you store data, the chain records the CID. This proves that *this specific data* existed at *this specific block number*.

## Data Limits

| Limit | Value | Notes |
|-------|-------|-------|
| Max Transaction Size | ~8 MiB | Substrate limit |
| Recommended Chunk Size | 1 MiB | Optimal for most use cases |
| Retention Period | Chain-specific | Check `transactionStorage.retentionPeriod()` |

Files larger than the transaction limit must be chunked. The SDKs handle this automatically.

## Sections

- [Storage Model](./storage.md) - How data is stored on-chain
- [Data Retrieval](./retrieval.md) - Fetching data via IPFS
- [Data Renewal](./renewal.md) - Extending storage retention
- [Authorization](./authorization.md) - Pre-approving storage
- [Manifests & IPFS](./manifests.md) - DAG-PB format for chunked data
