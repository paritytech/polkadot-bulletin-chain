# TypeScript Examples

TypeScript/JavaScript examples for interacting with Polkadot Bulletin Chain using PAPI (Polkadot API).

## Examples

### authorize-and-store

Basic workflow demonstrating account authorization and data storage.

**Files:**
- `papi.js` - WebSocket RPC connection mode
- `smoldot.js` - Light client mode (no node required)

**Usage:**
```bash
# WebSocket mode
node authorize-and-store/papi.js [ws_url] [seed]

# Light client mode
node authorize-and-store/smoldot.js [relay_chainspec] [parachain_chainspec]
```

### store-chunked-data

Demonstrates storing large files by:
1. Splitting files into chunks
2. Storing each chunk on-chain
3. Creating a DAG-PB manifest for IPFS compatibility
4. Retrieving the full file via IPFS gateway

**Usage:**
```bash
node store-chunked-data/index.js [ws_url] [seed]
```

### store-big-data

Handles very large files with parallel chunk uploads and optimized throughput.

**Usage:**
```bash
node store-big-data/index.js [ws_url] [seed]
```

### authorize-preimage-and-store

Content-addressed authorization using preimage hashes. Allows storing specific content without per-account authorization.

**Usage:**
```bash
node authorize-preimage-and-store/index.js [ws_url] [seed] [http_ipfs_api]
```

## Shared Files

- `api.js` - Core API functions for authorization and storage
- `common.js` - Utility functions and helpers
- `cid_dag_metadata.js` - CID and DAG-PB utilities
- `native_ipfs_dag_pb_chunked_data.js` - Native IPFS DAG-PB implementation
- `package.json` - Dependencies and scripts

## Setup

### Install Dependencies

```bash
npm install
```

### Generate PAPI Descriptors

PAPI requires type descriptors generated from the chain metadata:

```bash
# Add the chain
npx papi add -w ws://localhost:10000 bulletin

# Generate descriptors
npx papi
```

Or use npm scripts:
```bash
npm run papi:generate  # Generate from scratch
npm run papi:update    # Update existing descriptors
```

## Default Values

All examples support command-line arguments but have sensible defaults:

- **WebSocket URL**: `ws://localhost:10000`
- **Seed**: `//Alice` (sudo account for authorization)
- **IPFS HTTP API**: `http://127.0.0.1:8080`

## Requirements

- Node.js 18+
- Running Bulletin Chain node (or use smoldot mode)
- IPFS daemon (for chunked data examples)
