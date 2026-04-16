# Polkadot Bulletin Chain

Welcome to the official documentation for the **Polkadot Bulletin Chain** - a decentralized storage ledger for the Polkadot ecosystem.

## What is Bulletin Chain?

Polkadot Bulletin Chain is a specialized blockchain that provides **distributed data storage and retrieval infrastructure**. It allows users to:

- **Store** arbitrary data on-chain with proof-of-storage guarantees
- **Retrieve** data directly from validator nodes via the Bitswap protocol
- **Verify** data existence and timestamps through blockchain consensus

Unlike typical file storage systems (like Filecoin or Arweave), Bulletin Chain focuses on:

1. **Immutability**: Once a CID is on-chain, it cannot be changed
2. **Verifiability**: Data is content-addressed using standard CIDs
3. **Flexibility**: Supports both small direct storage and large chunked storage
4. **Decentralization**: Data retrieval via light client (smoldot) without centralized gateways

## Key Concepts

| Step | Concept | Description |
|------|---------|-------------|
| 1 | **Authorize** | Get permission to store (faucet on testnet) |
| 2 | **Store** | Submit data to the chain, receive a CID |
| 3 | **Retrieve** | Fetch data from validator nodes using the CID |
| 4 | **Renew** | Extend storage before the retention period expires |

## Accessing Bulletin Chain

There are multiple ways to interact with Bulletin Chain:

### SDKs (Recommended)

| Language | Package | Status |
|----------|---------|--------|
| **Rust** | `bulletin-sdk-rust` | Alpha |
| **TypeScript** | `@parity/bulletin-sdk` | Alpha |

The SDKs provide high-level abstractions for:
- Automatic data chunking for large files
- CID calculation (content-addressed identifiers)
- DAG-PB manifest generation
- Authorization management

> **Note**: The SDKs currently support storage operations only. Data retrieval will be added once the smoldot `bitswap_block` RPC is production-ready. See [Data Retrieval](./concepts/retrieval.md) for current options.

### Data Retrieval

| Method | Status | Description |
|--------|--------|-------------|
| **Smoldot Light Client** | Coming Soon | Decentralized retrieval via `bitswap_block` RPC |
| **Direct P2P (Helia)** | Available | Connect to validator nodes via libp2p |
| **IPFS Gateways** | Deprecated | Centralized, not recommended |

See [Data Retrieval](./concepts/retrieval.md) for details.

## Quick Start

```typescript
// TypeScript - Store data
import { AsyncBulletinClient } from "@parity/bulletin-sdk";
import { createClient, Binary } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws";

const papiClient = createClient(getWsProvider("wss://paseo-bulletin-rpc.polkadot.io"));
const api = papiClient.getTypedApi(bulletinDescriptor);
const client = new AsyncBulletinClient(api, signer, papiClient.submit);

const result = await client.store(Binary.fromText("Hello, Bulletin!")).send();
console.log("CID:", result.cid.toString());
```

```rust
// Rust - Store data
use bulletin_sdk_rust::prelude::*;

let client = TransactionClient::new("wss://paseo-bulletin-rpc.polkadot.io").await?;
let data = b"Hello, Bulletin!".to_vec();
let receipt = client.store(data, &signer).await?;
println!("Stored in block: {}", receipt.block_hash);
```

## Networks

| Network | Endpoint | Status |
|---------|----------|--------|
| Polkadot | - | Not released yet |
| Paseo (Testnet) | `wss://paseo-bulletin-rpc.polkadot.io` | Active |
| Westend (Testnet) | `wss://westend-bulletin-rpc.polkadot.io` | Active |
| PoP Testnet (stable) | `wss://pop3-testnet.parity-lab.parity.io/bulletin` | Active |
| Previewnet | `wss://previewnet.substrate.dev/bulletin` | Active |
| Local Dev | `ws://localhost:10000` | - |

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
