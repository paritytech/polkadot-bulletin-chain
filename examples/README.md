# Polkadot Bulletin Chain - Examples

Examples demonstrating how to interact with the Polkadot Bulletin Chain.

## Directory Structure

```
examples/
├── *.js                       # JavaScript examples and shared utilities
├── package.json               # JS dependencies
├── typescript/                # TypeScript SDK examples
│   └── authorize_and_store.js # TS SDK authorize-and-store example
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

### Run prerequisites

It's only needed once after checkout or when dependencies change:
- `just npm-install`

### Run full workflow example (standalone)

Standalone recipes handle full setup/teardown automatically:

```bash
# Solochain (Polkadot runtime) with WebSocket + Kubo Docker IPFS (default)
just run-authorize-and-store bulletin-polkadot-runtime ws

# Solochain with WebSocket + Kubo native (no Docker required)
just run-authorize-and-store bulletin-polkadot-runtime ws kubo-native

# Westend parachain with smoldot light client
just run-authorize-and-store bulletin-westend-runtime smoldot
```

### IPFS modes

Two IPFS backends are supported:

- **`kubo-docker`** (default) — Runs Kubo inside a Docker container. Requires Docker.
- **`kubo-native`** — Runs Kubo as a local binary (downloaded automatically). No Docker required.

### Run individual commands for manual testing

```bash
# Start services (zombienet + IPFS with Peering.Peers auto-reconnect)
just start-services /tmp/my-test bulletin-polkadot-runtime kubo-native

# Generate PAPI descriptors from running node
just papi-generate

# Run individual tests (services must be running)
just run-test-authorize-and-store /tmp/my-test bulletin-polkadot-runtime ws
just run-test-store-chunked-data /tmp/my-test
just run-test-store-big-data /tmp/my-test big32

# Stop services
just stop-services /tmp/my-test kubo-native
```

### Run Rust Examples

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
just start-services <test_dir> <runtime> [ipfs_mode]

# Stop all services
just stop-services <test_dir> [ipfs_mode]
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

## Manually

```shell
cd polkadot-bulletin-chain   # make you are inside the project directory for the following steps
```

### Download Zombienet

```shell
OS="$(uname -s)"
ARCH="$(uname -m)"

if [ "$OS" = "Linux" ]; then
  zb_os=linux
else
  zb_os=macos
fi

if [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
  zb_arch=arm64
else
  zb_arch=x64
fi

zb_bin="zombienet-${zb_os}-${zb_arch}"

wget "https://github.com/paritytech/zombienet/releases/download/v1.3.133/${zb_bin}"
chmod +x "${zb_bin}"
```

### Run Kubo

#### Execute Locally

```shell
wget https://dist.ipfs.tech/kubo/v0.38.1/kubo_v0.38.1_darwin-arm64.tar.gz
tar -xvzf kubo_v0.38.1_darwin-arm64.tar.gz
./kubo/ipfs version
./kubo/ipfs init
./kubo/ipfs daemon &   # run in the background
```

#### Use Docker

```shell
docker pull ipfs/kubo:latest
docker run -d --name ipfs-node -v ipfs-data:/data/ipfs \
  -p 127.0.0.1:4011:4011 -p 127.0.0.1:8283:8283 -p 127.0.0.1:5011:5011 \
  --add-host=host.docker.internal:host-gateway \
  ipfs/kubo:latest
docker logs -f ipfs-node
```

### Run Bulletin Solochain with `--ipfs-server`

```shell
cargo build --release -p polkadot-bulletin-chain

POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain \
  ./$(ls zombienet-*-*) -p native spawn ./zombienet/bulletin-polkadot-local.toml
```

### Connect IPFS Nodes

Kubo's **Peering.Peers** feature handles automatic (re)connection to chain nodes.
The `just` recipes configure this automatically before starting the IPFS daemon.

For manual setup, configure Peering.Peers in your Kubo config:

```shell
# Local Kubo -- configure peering before starting the daemon
./kubo/ipfs config --json Peering.Peers '[
  {"ID":"12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm","Addrs":["/ip4/127.0.0.1/tcp/10001/ws"]},
  {"ID":"12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby","Addrs":["/ip4/127.0.0.1/tcp/12347/ws"]}
]'
```

```shell
# Docker Kubo -- configure peering, then restart the container
docker exec ipfs-node ipfs config --json Peering.Peers '[
  {"ID":"12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm","Addrs":["/dns4/host.docker.internal/tcp/10001/ws"]},
  {"ID":"12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby","Addrs":["/dns4/host.docker.internal/tcp/12347/ws"]}
]'
docker restart ipfs-node
```

### Run Bulletin (Westend) Parachain with `--ipfs-server`

#### Prerequisites

```shell
mkdir -p ~/local_bridge_testing/bin

# Ensures `polkadot` and `polkadot-parachain` exist
git clone https://github.com/paritytech/polkadot-sdk.git
# TODO: unless not merged: https://github.com/paritytech/polkadot-sdk/pull/10370
git reset --hard origin/bko-bulletin-para-support
cd polkadot-sdk

cargo build -p polkadot -r
ls -la target/release/polkadot
cp target/release/polkadot ~/local_bridge_testing/bin
cp target/release/polkadot-prepare-worker ~/local_bridge_testing/bin
cp target/release/polkadot-execute-worker ~/local_bridge_testing/bin
~/local_bridge_testing/bin/polkadot --version
# polkadot 1.20.2-165ba47dc91 or higher

cargo build -p polkadot-parachain-bin -r
ls -la target/release/polkadot-parachain
cp target/release/polkadot-parachain ~/local_bridge_testing/bin
~/local_bridge_testing/bin/polkadot-parachain --version
# polkadot-parachain 1.20.2-165ba47dc91 or higher
```

#### Launch Parachain

```shell
# Bulletin Parachain (Westend)
./scripts/create_bulletin_westend_spec.sh
POLKADOT_BINARY_PATH=~/local_bridge_testing/bin/polkadot \
  POLKADOT_PARACHAIN_BINARY_PATH=~/local_bridge_testing/bin/polkadot-parachain \
  ./$(ls zombienet-*-*) -p native spawn ./zombienet/bulletin-westend-local.toml
```

#### Connect IPFS Nodes

Configure Peering.Peers for the Westend parachain nodes:

```shell
# Local Kubo
./kubo/ipfs config --json Peering.Peers '[
  {"ID":"12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ","Addrs":["/ip4/127.0.0.1/tcp/10001/ws"]},
  {"ID":"12D3KooWJ8sqAYtMBX3z3jy2iM98XGLFVzVfUPtmgDzxXSPkVpZZ","Addrs":["/ip4/127.0.0.1/tcp/12347/ws"]}
]'
```

```shell
# Docker Kubo
docker exec ipfs-node ipfs config --json Peering.Peers '[
  {"ID":"12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ","Addrs":["/dns4/host.docker.internal/tcp/10001/ws"]},
  {"ID":"12D3KooWJ8sqAYtMBX3z3jy2iM98XGLFVzVfUPtmgDzxXSPkVpZZ","Addrs":["/dns4/host.docker.internal/tcp/12347/ws"]}
]'
docker restart ipfs-node
```

### Trigger Authorize, Store and IPFS Get

#### Example for Simple Authorizing and Store

##### Using Modern PAPI (Polkadot API)
```bash
cd examples
npm install

# First, generate the PAPI descriptors:
#  (Generate TypeScript types in `.papi/descriptors/`)
#  (Create metadata files in `.papi/metadata/bulletin.scale`)
# Generate PAPI descriptors using local node:
#   npx papi add -w ws://localhost:10000 bulletin
#   npx papi
# or:
npm run papi:generate
# or if you already have .papi folder you can always update it
npm run papi:update

# Then run the PAPI version (from the examples directory)
node authorize_and_store_papi.js
```

#### Example for Multipart / Chunked Content / Big Files

The code stores one file, splits it into chunks, and then uploads those chunks to Bulletin.

It collects all the partial CIDs for each chunk and saves them as a custom metadata JSON file in Bulletin.

Now we have two examples:
1. **Manual reconstruction** -- return the metadata and chunk CIDs, then reconstruct the original file manually.
2. **IPFS DAG feature** --
    * converts the metadata into a DAG-PB descriptor,
    * stores it directly in IPFS,
    * and allows fetching the entire file using a single root CID from an IPFS HTTP gateway (for example: `http://localhost:8080/ipfs/QmW2WQi7j6c7UgJTarActp7tDNikE4B2qXtFCfLPdsgaTQ`).

```shell
node store_chunked_data.js
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
