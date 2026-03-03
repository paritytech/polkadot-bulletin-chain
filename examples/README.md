# How to Run

## Using `just`

[`just`](https://github.com/casey/just) is a command runner (similar to `make`) that helps execute project tasks.

Install just with:
- `cargo install just`, if you have cargo package manager,
- `brew install just`, if you're on Mac OS and have `brew` package manager installed,
- `sudo apt install just`, if you're using a Linux distribution.

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
# Bulletin Solochain

```shell
# cd polkadot-bulletin-chain   # make you are in this directory
cargo build --release -p polkadot-bulletin-chain

POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain \
  ./$(ls zombienet-*-*) -p native spawn ./zombienet/bulletin-polkadot-local.toml

### Connect IPFS Nodes

Kubo's **Peering.Peers** feature handles automatic (re)connection to chain nodes.
The `just` recipes configure this automatically before starting the IPFS daemon.

For manual setup, configure Peering.Peers in your Kubo config:

```shell
# Local Kubo — configure peering before starting the daemon
./kubo/ipfs config --json Peering.Peers '[
  {"ID":"12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm","Addrs":["/ip4/127.0.0.1/tcp/10002/ws"]},
  {"ID":"12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby","Addrs":["/ip4/127.0.0.1/tcp/12348/ws"]}
]'
```

```shell
# Docker Kubo — configure peering, then restart the container
docker exec ipfs-node ipfs config --json Peering.Peers '[
  {"ID":"12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm","Addrs":["/dns4/host.docker.internal/tcp/10002/ws"]},
  {"ID":"12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby","Addrs":["/dns4/host.docker.internal/tcp/12348/ws"]}
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
  {"ID":"12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ","Addrs":["/ip4/127.0.0.1/tcp/10002/ws"]},
  {"ID":"12D3KooWJ8sqAYtMBX3z3jy2iM98XGLFVzVfUPtmgDzxXSPkVpZZ","Addrs":["/ip4/127.0.0.1/tcp/12348/ws"]}
]'
```

```shell
# Docker Kubo
docker exec ipfs-node ipfs config --json Peering.Peers '[
  {"ID":"12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ","Addrs":["/dns4/host.docker.internal/tcp/10002/ws"]},
  {"ID":"12D3KooWJ8sqAYtMBX3z3jy2iM98XGLFVzVfUPtmgDzxXSPkVpZZ","Addrs":["/dns4/host.docker.internal/tcp/12348/ws"]}
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
1. **Manual reconstruction** — return the metadata and chunk CIDs, then reconstruct the original file manually.
2. **IPFS DAG feature** —
    * converts the metadata into a DAG-PB descriptor,
    * stores it directly in IPFS,
    * and allows fetching the entire file using a single root CID from an IPFS HTTP gateway (for example: `http://localhost:8080/ipfs/QmW2WQi7j6c7UgJTarActp7tDNikE4B2qXtFCfLPdsgaTQ`).

```shell
node store_chunked_data.js
```
