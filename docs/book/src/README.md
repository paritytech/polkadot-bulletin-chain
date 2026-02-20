# Polkadot Bulletin Chain

Welcome to the official documentation for the **Polkadot Bulletin Chain** - a decentralized storage ledger for the Polkadot ecosystem.

## What is Bulletin Chain?

Polkadot Bulletin Chain is a specialized blockchain that provides **distributed data storage and retrieval infrastructure**. It allows users to:

- **Store** arbitrary data on-chain with proof-of-storage guarantees
- **Retrieve** data via IPFS using content-addressed identifiers (CIDs)
- **Verify** data existence and timestamps through blockchain consensus

Unlike typical file storage systems (like Filecoin or Arweave), Bulletin Chain focuses on:

1. **Immutability**: Once a CID is on-chain, it cannot be changed
2. **Verifiability**: Data is content-addressed using standard IPFS CIDs
3. **Flexibility**: Supports both small direct storage and large chunked storage
4. **Integration**: Seamlessly works with standard IPFS tools and gateways

## Key Concepts

| Step | Concept | Description |
|------|---------|-------------|
| 1 | **Authorize** | Get permission to store (faucet on testnet) |
| 2 | **Store** | Submit data to the chain, receive a CID |
| 3 | **Retrieve** | Fetch data via IPFS using the CID |
| 4 | **Renew** | Extend storage before the retention period expires |

## Accessing Bulletin Chain

There are multiple ways to interact with Bulletin Chain:

### SDKs (Recommended)

| Language | Package | Status |
|----------|---------|--------|
| **Rust** | `bulletin-sdk-rust` | Alpha |
| **TypeScript** | `@bulletin/sdk` | Alpha |

The SDKs provide high-level abstractions for:
- Automatic data chunking for large files
- CID calculation (IPFS-compatible)
- DAG-PB manifest generation
- Authorization management

### IPFS

Data retrieval happens through IPFS:
- Public gateways: `https://ipfs.io/ipfs/{cid}`
- Direct from Bulletin nodes via Bitswap protocol
- Standard `ipfs` CLI tools

## Quick Start

```typescript
// TypeScript - Store data
import { BulletinClient } from "@bulletin/sdk";

const client = new BulletinClient();
const data = new TextEncoder().encode("Hello, Bulletin!");
const operation = client.prepareStore(data);

// Submit via PAPI, get CID back
// ...

// Retrieve via IPFS gateway
const response = await fetch(`https://ipfs.io/ipfs/${cid}`);
```

```rust
// Rust - Store data
use bulletin_sdk_rust::prelude::*;

let client = BulletinClient::new();
let data = b"Hello, Bulletin!".to_vec();
let operation = client.prepare_store(data, None)?;

// Submit via subxt, get CID back
// ...

// Retrieve via IPFS gateway
let response = reqwest::get(format!("https://ipfs.io/ipfs/{}", cid)).await?;
```

## Networks

| Network | Endpoint | Status |
|---------|----------|--------|
| Paseo (Testnet) | `wss://paseo-bulletin-rpc.polkadot.io` | Active |
| Westend (Testnet) | `wss://westend-bulletin-rpc.polkadot.io` | Active |
| Dotspark | `wss://bulletin.dotspark.app` | Active |
| Local Dev | `ws://localhost:10000` | - |

See [shared/networks.json](https://github.com/paritytech/polkadot-bulletin-chain/blob/main/shared/networks.json) for the full configuration.

## Documentation Structure

- **[Core Concepts](./concepts/README.md)** - Understand how Bulletin Chain works
  - Storage model, authorization, manifests, retrieval, renewal
- **[Rust SDK](./rust/README.md)** - Native Rust client
  - Supports `std` and `no_std` (WASM)
- **[TypeScript SDK](./typescript/README.md)** - JS/TS client
  - Node.js and Browser, integrates with PAPI

## Building This Documentation

This documentation is built using [mdBook](https://github.com/rust-lang/mdBook).

```bash
# Install mdbook
cargo install mdbook

# Serve locally with live reload
cd docs/book
mdbook serve --open

# Build static HTML
mdbook build
```
