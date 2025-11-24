# How to Run


```shell
cd polkadot-bulletin-chain   # make you are inside the project directory for the following steps
```

## Download Zombienet

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

## Run Kubo

#### Execute Locally

```shell
wget https://dist.ipfs.tech/kubo/v0.38.1/kubo_v0.38.1_darwin-arm64.tar.gz
tar -xvzf kubo_v0.38.1_darwin-arm64.tar.gz
./kubo/ipfs version
./kubo/ipfs init
./kubo/ipfs daemon &   # run in the background
```

#### Use Docker

* Use `172.17.0.1` or  `host.docker.internal` for swarm connections

```shell
docker pull ipfs/kubo:latest
docker run -d --name ipfs-node -v ipfs-data:/data/ipfs -p 4001:4001 -p 8080:8080 -p 5001:5001 ipfs/kubo:latest
docker logs -f ipfs-node
```

## Run Bulletin Solochain with `--ipfs-server`

```shell
# Bulletin Solochain

```shell
# cd polkadot-bulletin-chain   # make you are in this directory
cargo build --release -p polkadot-bulletin-chain

POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain \
  ./$(ls zombienet-*-*) -p native spawn ./zombienet/bulletin-polkadot-local.toml

### Connect IPFS Nodes

```shell
# Uses Kubo
./kubo/ipfs swarm connect /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
# connect 12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm success

./kubo/ipfs swarm connect /ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
# connect 12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby success
```

```shell
# Uses Docker (replace 127.0.0.1 with 172.17.0.1)
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
```

```shell
# Runs a script that reconnects every 2 seconds
# Defaults to 'local' (local Kubo); use 'docker' for the Docker setup
./scripts/ipfs-reconnect-solo.sh
```

## Run Bulletin (Westend) Parachain with `--ipfs-server`

### Prerequisites 

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

### Launch Parachain

```shell
# Bulletin Parachain (Westend)
./scripts/create_bulletin_westend_spec.sh
POLKADOT_BINARY_PATH=~/local_bridge_testing/bin/polkadot \
  POLKADOT_PARACHAIN_BINARY_PATH=~/local_bridge_testing/bin/polkadot-parachain \
  ./$(ls zombienet-*-*) -p native spawn ./zombienet/bulletin-westend-local.toml
```

### Connect IPFS Nodes

```shell
# Uses Kubo
./kubo/ipfs swarm connect /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ
# connect 12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ success

./kubo/ipfs swarm connect /ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWJ8sqAYtMBX3z3jy2iM98XGLFVzVfUPtmgDzxXSPkVpZZ
# connect 12D3KooWJ8sqAYtMBX3z3jy2iM98XGLFVzVfUPtmgDzxXSPkVpZZ success
```

```shell
# Uses Docker (replace 127.0.0.1 with 172.17.0.1)
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/10001/ws/p2p/12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/12347/ws/p2p/12D3KooWJ8sqAYtMBX3z3jy2iM98XGLFVzVfUPtmgDzxXSPkVpZZ
```

```shell
# Runs a script that reconnects every 2 seconds
# Defaults to 'local' (local Kubo); use 'docker' for the Docker setup
./scripts/ipfs-reconnect-westend.sh
```

## Trigger Authorize, Store and IPFS Get

### Example for Simple Authorizing and Store

#### Using Legacy @polkadot/api (PJS)
```
cd examples
npm install

node authorize_and_store.js
```

#### Using Modern PAPI (Polkadot API)
```bash
cd examples
npm install

# First, generate the PAPI descriptors:
#  (Generate TypeScript types in `.papi/descriptors/`)
#  (Create metadata files in `.papi/metadata/bulletin.scale`)
npm run papi:generate
# or if you already have .papi folder you can always update it
npm run papi:update

# Then run the PAPI version (from the examples directory)
node authorize_and_store_papi.js
```

### Example for Multipart / Chunked Content / Big Files

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
