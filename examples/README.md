# Polkadot Bulletin Chain - Examples

This directory contains examples demonstrating how to interact with Polkadot Bulletin Chain using different SDKs and languages.

## Directory Structure

```
examples/
├── typescript/              # TypeScript/JavaScript examples
│   ├── authorize-and-store/ # Basic authorization and storage
│   ├── store-chunked-data/  # Large file chunking example
│   ├── store-big-data/      # Big data handling
│   ├── authorize-preimage-and-store/  # Preimage authorization
│   └── *.js                 # Shared utilities (api.js, common.js, etc.)
├── rust/                    # Rust examples
│   └── authorize-and-store/ # Rust subxt example
└── justfile                 # Task automation
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

#### TypeScript Examples

```bash
# Install dependencies
cd examples
just npm-install

# Run authorize-and-store example (WebSocket mode)
just run-authorize-and-store bulletin-westend-runtime papi

# Run authorize-and-store example (Smoldot light client mode)
just run-authorize-and-store bulletin-westend-runtime smoldot

# Run specific TypeScript example
just run-test-ts store-chunked-data <test_dir> <runtime>
just run-test-ts store-big-data <test_dir> <runtime>
just run-test-ts authorize-preimage-and-store <test_dir> <runtime>
```

#### Rust Examples

```bash
# Run Rust authorize-and-store example
just run-test-rust authorize-and-store <test_dir>

# With custom parameters
just run-test-rust authorize-and-store <test_dir> ws://localhost:9944 "//Bob"
```

## Available Examples

### TypeScript Examples

Located in `typescript/` directory:

1. **authorize-and-store** - Basic workflow demonstrating account authorization and data storage
   - `papi.js` - WebSocket RPC connection
   - `smoldot.js` - Light client connection

2. **store-chunked-data** - Store large files using automatic chunking with DAG-PB manifest

3. **store-big-data** - Handle very large files with parallel chunk uploads

4. **authorize-preimage-and-store** - Content-addressed authorization using preimage hashes

### Rust Examples

Located in `rust/` directory:

1. **authorize-and-store** - Demonstrates using subxt with Polkadot Bulletin Chain
   - Auto-discovers signed extensions from metadata
   - Shows proper authorization flow
   - Direct blockchain interaction

## Justfile Commands

### Running Examples

```bash
# Run TypeScript example
just run-test-ts <example-name> <test_dir> <runtime> [mode]
# Examples:
#   just run-test-ts authorize-and-store ./test bulletin-westend-runtime papi
#   just run-test-ts store-chunked-data ./test bulletin-polkadot-runtime

# Run Rust example
just run-test-rust <example-name> <test_dir> [ws_url] [seed]
# Examples:
#   just run-test-rust authorize-and-store ./test
#   just run-test-rust authorize-and-store ./test ws://localhost:9944 "//Bob"
```

### Full Workflow Commands

These commands handle setup, running the example, and teardown:

```bash
# Default - run authorize-and-store with full setup/teardown
just

# Run authorize-and-store with PAPI (WebSocket)
just run-authorize-and-store bulletin-westend-runtime papi

# Run authorize-and-store with Smoldot (light client)
just run-authorize-and-store bulletin-westend-runtime smoldot
```

### Service Management

```bash
# Start all services (IPFS, zombienet, etc.)
just setup-services <test_dir> <runtime>

# Stop all services
just teardown-services <test_dir>

# Run all tests (requires services to be running)
just run-all-tests <test_dir> <runtime>
```

## Manual Setup

If you prefer to run examples manually without `just`:

### 1. Setup Dependencies

```bash
# Install TypeScript dependencies
cd examples/typescript
npm install
cd ..

# Generate PAPI descriptors
cd typescript
npx papi add -w ws://localhost:10000 bulletin
npx papi
```

### 2. Start Services

#### IPFS Setup

**Local Kubo:**
```bash
wget https://dist.ipfs.tech/kubo/v0.38.1/kubo_v0.38.1_darwin-arm64.tar.gz
tar -xvzf kubo_v0.38.1_darwin-arm64.tar.gz
./kubo/ipfs init
./kubo/ipfs daemon &
```

**Docker:**
```bash
docker pull ipfs/kubo:latest
docker run -d --name ipfs-node -v ipfs-data:/data/ipfs -p 4011:4011 -p 8283:8283 -p 5011:5011 ipfs/kubo:latest
docker logs -f ipfs-node
```

#### Bulletin Chain Setup

**Solochain (bulletin-polkadot-runtime):**
```bash
cargo build --release -p polkadot-bulletin-chain
POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain \
  zombienet -p native spawn ./zombienet/bulletin-polkadot-local.toml
```

**Parachain (bulletin-westend-runtime):**
```bash
# Setup prerequisites (one-time)
just setup-parachain-prerequisites

# Launch parachain
./scripts/create_bulletin_westend_spec.sh
POLKADOT_BINARY_PATH=~/local_bulletin_testing/bin/polkadot \
  POLKADOT_PARACHAIN_BINARY_PATH=~/local_bulletin_testing/bin/polkadot-parachain \
  zombienet -p native spawn ./zombienet/bulletin-westend-local.toml
```

#### Connect IPFS Nodes

**Local Kubo:**
```bash
./kubo/ipfs swarm connect /ip4/127.0.0.1/tcp/10001/ws/p2p/<peer-id>
./kubo/ipfs swarm connect /ip4/127.0.0.1/tcp/12347/ws/p2p/<peer-id>
```

**Docker (macOS/Windows):**
```bash
docker exec -it ipfs-node ipfs swarm connect \
  /dns4/host.docker.internal/tcp/10001/ws/p2p/<peer-id>
```

**Docker (Linux):**
```bash
docker exec -it ipfs-node ipfs swarm connect \
  /ip4/172.17.0.1/tcp/10001/ws/p2p/<peer-id>
```

### 3. Run Examples

**TypeScript:**
```bash
cd examples/typescript

# Authorize and store (PAPI)
node authorize-and-store/papi.js

# Store chunked data
node store-chunked-data/index.js

# Store big data
node store-big-data/index.js

# Authorize preimage and store
node authorize-preimage-and-store/index.js
```

**Rust:**
```bash
cd examples/rust/authorize-and-store

# Generate metadata first
./fetch_metadata.sh ws://localhost:10000

# Build and run
cargo build --release
./target/release/authorize-and-store --ws ws://localhost:10000 --seed "//Alice"
```

## Command Line Arguments

### TypeScript Examples

**authorize-and-store/papi.js:**
```bash
node authorize-and-store/papi.js [ws_url] [seed]
# Example: node authorize-and-store/papi.js ws://localhost:9944 "//Bob"
```

**authorize-and-store/smoldot.js:**
```bash
node authorize-and-store/smoldot.js [relay_chainspec_path] [parachain_chainspec_path]
```

**store-chunked-data/index.js:**
```bash
node store-chunked-data/index.js [ws_url] [seed]
```

**store-big-data/index.js:**
```bash
node store-big-data/index.js [ws_url] [seed]
```

**authorize-preimage-and-store/index.js:**
```bash
node authorize-preimage-and-store/index.js [ws_url] [seed] [http_ipfs_api]
```

### Rust Examples

**authorize-and-store:**
```bash
./target/release/authorize-and-store --ws <websocket_url> --seed <seed_phrase>
# Example: ./target/release/authorize-and-store --ws ws://localhost:9944 --seed "//Bob"
```

## Learn More

- **TypeScript SDK**: See `../sdk/typescript/` for the high-level Bulletin SDK
- **Rust SDK**: See `../sdk/rust/` for the Rust SDK implementation
- **Documentation**: See `../docs/sdk-book/` for comprehensive SDK documentation

## Troubleshooting

**PAPI descriptors not found:**
```bash
cd typescript
npx papi add -w ws://localhost:10000 bulletin
npx papi
```

**IPFS connection issues:**
- Ensure IPFS daemon is running
- Check firewall settings for ports 4001, 8080, 5001
- Verify peer IDs match your node configuration

**Metadata errors (Rust):**
```bash
cd rust/authorize-and-store
./fetch_metadata.sh ws://localhost:10000
```

**Port conflicts:**
- Default Bulletin solochain: ws://localhost:10000
- Default Bulletin parachain: ws://localhost:10000
- IPFS: 4001 (swarm), 8080 (gateway), 5001 (API)
