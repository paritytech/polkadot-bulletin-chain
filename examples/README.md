# Polkadot Bulletin Chain - Examples

Examples demonstrating how to interact with the Polkadot Bulletin Chain.

## Directory Structure

```
examples/
├── *.js                       # JavaScript examples and shared utilities
├── package.json               # JS dependencies
├── rust/                      # Rust examples
│   └── authorize-and-store/   # Rust subxt example
└── justfile                   # Task automation
```

## Quick Start

### Prerequisites

Install `just` command runner:
```bash
cargo install just      # Using cargo
brew install just       # Using Homebrew (macOS)
sudo apt install just   # Using apt (Linux)
```

### Run Examples

#### JavaScript Examples

```bash
cd examples

# Install dependencies
just npm-install

# Run authorize-and-store (WebSocket mode, full setup/teardown)
just run-authorize-and-store bulletin-westend-runtime ws

# Run authorize-and-store (Smoldot light client mode)
just run-authorize-and-store bulletin-westend-runtime smoldot

# Run other standalone examples (full setup/teardown)
just run-store-chunked-data bulletin-polkadot-runtime
just run-store-big-data bulletin-polkadot-runtime
just run-authorize-preimage-and-store-papi bulletin-polkadot-runtime
```

#### Rust Examples

```bash
cd examples

# Run Rust SDK tests (services must already be running)
just test-rust-sdk <test_dir> <runtime>

# Run individual Rust example
just run-test-rust authorize-and-store <test_dir> <runtime>
```

## Available Examples

### JavaScript

| File | Description |
|------|-------------|
| `authorize_and_store_papi.js` | Basic authorization and storage via WebSocket RPC |
| `authorize_and_store_papi_smoldot.js` | Same workflow using Smoldot light client |
| `authorize_preimage_and_store_papi.js` | Content-addressed authorization using preimage hashes |
| `store_chunked_data.js` | Large file storage with DAG-PB chunking |
| `store_big_data.js` | Very large file handling with parallel chunk uploads |
| `native_ipfs_dag_pb_chunked_data.js` | Native IPFS DAG-PB chunked data example |
| `api.js` | Shared transaction and storage API helpers |
| `common.js` | Shared utilities (signers, image generation, etc.) |
| `logger.js` | Unified logging functions |
| `cid_dag_metadata.js` | CID and DAG metadata utilities |

### Rust

| Directory | Description |
|-----------|-------------|
| `rust/authorize-and-store/` | Authorization and storage using subxt |

## Justfile Commands

### Service Management

```bash
# Start all services (zombienet, IPFS, PAPI descriptors)
just start-services <test_dir> <runtime>

# Stop all services
just stop-services <test_dir>
```

### Individual Test Recipes (services must be running)

```bash
just run-test-authorize-and-store <test_dir> <runtime> [mode]
just run-test-store-chunked-data <test_dir>
just run-test-store-big-data <test_dir> [image_size]
just run-test-authorize-preimage-and-store <test_dir>
just run-test-rust <example> <test_dir> <runtime>
just test-rust-sdk <test_dir> <runtime>
```

### Live Network Tests

```bash
just run-live-tests-westend <seed> [ipfs_gateway_url] [image_size]
just run-live-tests-paseo <seed> [ipfs_gateway_url] [image_size]
```

## Manual Setup

If you prefer to run examples without `just`:

### 1. Install Dependencies

```bash
cd examples
npm install
npx papi add -w ws://localhost:10000 bulletin
```

### 2. Start Services

See the justfile for full setup details. At minimum you need:
- A running Bulletin Chain node (solochain or parachain via zombienet)
- An IPFS node connected to the chain's IPFS peers

### 3. Run Examples

```bash
cd examples

# JavaScript
node authorize_and_store_papi.js [ws_url] [seed] [http_ipfs_api]
node store_chunked_data.js [ws_url] [seed] [http_ipfs_api]
node store_big_data.js [ws_url] [seed] [ipfs_gateway_url] [image_size]

# Rust
cd rust/authorize-and-store
./fetch_metadata.sh ws://localhost:10000
cargo build --release
./target/release/authorize-and-store --ws ws://localhost:10000 --seed "//Alice"
```

## Troubleshooting

**PAPI descriptors not found:**
```bash
cd examples
npx papi add -w ws://localhost:10000 bulletin
```

**Metadata errors (Rust):**
```bash
cd examples/rust/authorize-and-store
./fetch_metadata.sh ws://localhost:10000
```
