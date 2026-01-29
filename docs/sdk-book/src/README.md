# Polkadot Bulletin SDK

Welcome to the official documentation for the **Polkadot Bulletin Chain SDKs**.

These SDKs provide high-level abstractions for interacting with the Bulletin Chain, specifically designed to simplify the process of storing data, managing authorizations, and generating IPFS-compatible manifests.

## Available SDKs

| Language | Package | Status | Description |
|----------|---------|--------|-------------|
| **Rust** | `bulletin-sdk-rust` | Alpha | Native Rust client, supports `std` and `no_std` (WASM/ink!) |
| **TypeScript** | `@bulletin/sdk` | Alpha | JS/TS client for Node.js and Browser, integrates with PAPI |

## What is Bulletin Chain?

Polkadot Bulletin Chain is a decentralized storage ledger that allows users to prove the existence and timestamp of data. Unlike typical file storage chains (like Filecoin or Arweave), Bulletin Chain focuses on:

1.  **Immutability**: Once the hash/CID is on-chain, it cannot be changed.
2.  **Verifiability**: Data is content-addressed using CIDs (Content Identifiers).
3.  **Flexibility**: Supports small on-chain storage and large off-chain storage with on-chain manifests.

## Key Features

- **Automatic Chunking**: The SDKs handle splitting large files into optimal chunks (default 1 MiB) to fit within blockchain transaction limits.
- **DAG-PB Manifests**: Automatically generates Merkle DAGs (Directed Acyclic Graphs) compliant with IPFS standards.
- **Authorization Management**: Tools to estimate costs and manage the two-step "Authorize -> Store" flow required by the network.

## Next Steps

- Read about [Core Concepts](./concepts/README.md) to understand how storage works.
- Jump to [Rust SDK](./rust/README.md) if you are building a Rust application or Parachain.
- Jump to [TypeScript SDK](./typescript/README.md) if you are building a dApp or web interface.

---

## How to Build & View Locally

This documentation is built using [mdBook](https://github.com/rust-lang/mdBook).

### Prerequisites

You need to have `mdbook` installed. If you have Rust installed, you can install it via Cargo:

```bash
cargo install mdbook
```

### Viewing the Book

1.  Navigate to the book directory:
    ```bash
    cd docs/sdk-book
    ```

2.  Serve the book locally (it will open in your browser and live-reload on changes):
    ```bash
    mdbook serve --open
    ```

3.  Build the static HTML:
    ```bash
    mdbook build
    ```
    The output will be in `docs/sdk-book/book/html`.