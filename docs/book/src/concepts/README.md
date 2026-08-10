# Core Concepts

> [!WARNING]
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../../../../LICENSE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain).

This section covers the fundamental concepts you need to understand when working with Bulletin Chain.

## Data Lifecycle

```
1. AUTHORIZE       2. STORE           3. RETRIEVE             4. RENEW
   ↓                  ↓                  ↓                       ↓
Get permission    Submit data       Fetch from validators   Extend retention
(faucet/sudo)     + receive CID     via light client        before expiration
```

### 1. Authorization (Faucet)

Before storing data, accounts must be **authorized**. This prevents spam and manages storage costs.

- Use the **Faucet** (testnet) or request authorization from a Root/Sudo user
- Authorization grants permission to store a specified amount of data (bytes + transactions)
- Authorization can be for an account or a specific content hash (preimage)
- Learn more: [Authorization](./authorization.md)

### 2. Storage

Once authorized, submit data to the chain:

- **Small data** (< 2 MiB): Stored directly in a single transaction
- **Large data** (> 2 MiB): Split into chunks with a DAG-PB manifest
- On success, you receive:
  - **CID** (Content Identifier) for retrieval
  - **Block number** and **index** (needed for renewal)
- Learn more: [Storage Model](./storage.md)

### 3. Retrieval

Data is retrieved from Bulletin validator nodes:

- **Coming soon**: Smoldot light client with `bitswap_block` RPC (decentralized)
- **Available now**: Direct P2P connection to validators via libp2p/Helia
- **Deprecated**: Public IPFS gateways (centralized, not recommended)
- Learn more: [Data Retrieval](./retrieval.md)

### 4. Renewal

Data has a **retention period** after which it may be pruned:

- Track the block number and index from `Stored`/`Renewed` events, or use the content hash
- Reference the data with a `TransactionRef` (`Position { block, index }` or `ContentHash(hash)`)
- Renew before expiration: `renew(entry)` (one-shot scheduled), `force_renew(entry)` (immediate), or `enable_auto_renew(content_hash)` (recurring)
- Each `force_renew` gives you a **new** block/index for the next renewal
- Learn more: [Data Renewal](./renewal.md)

## CIDs (Content Identifiers)

Bulletin Chain uses **CIDs** (Content Identifiers) to identify data. A CID is a self-describing, content-addressed label:

| Component | Description | Example |
|-----------|-------------|---------|
| **Version** | CID version | `1` (CIDv1) |
| **Codec** | Data format | `0x55` (Raw), `0x70` (DAG-PB) |
| **Multihash** | Hash algorithm | `blake2b-256`, `sha2-256` |

When you store data, the chain records the CID. This proves that *this specific data* existed at *this specific block number*.

## Data Limits

| Limit | Value | Notes |
|-------|-------|-------|
| Max Transaction Size | ~2 MiB | runtime `MaxTransactionSize` |
| Recommended Chunk Size | 1 MiB | Optimal for most use cases |
| Retention Period | Chain-specific | Storage value `TransactionStorage.RetentionPeriod` (default 201,600 blocks) |

Files larger than the transaction limit must be chunked. The SDKs handle this automatically.

## Sections

- [Authorization](./authorization.md) - Getting permission to store (faucet)
- [Storage Model](./storage.md) - How data is stored on-chain
- [Data Retrieval](./retrieval.md) - Fetching data from validator nodes
- [Data Renewal](./renewal.md) - Extending storage retention
- [DAG-PB Manifests](./manifests.md) - Manifest format for chunked data

## Security

See the [root README](../../../../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
