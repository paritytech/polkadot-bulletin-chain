# How to run

## Build Bulletin

```shell
git clone https://github.com/paritytech/polkadot-bulletin-chain.git
cd polkadot-bulletin-chain
cargo build --release -p polkadot-bulletin-chain
```

```shell
cd polkadot-bulletin-chain # make sure we are within the folder for the following steps
```

## Download Zombienet

```shell
ZB_VER=v1.3.133

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

wget "https://github.com/paritytech/zombienet/releases/download/${ZB_VER}/${zb_bin}"
chmod +x "${zb_bin}"
```

## Run Kubo

#### Locally

```shell
wget https://dist.ipfs.tech/kubo/v0.38.1/kubo_v0.38.1_darwin-arm64.tar.gz
tar -xvzf kubo_v0.38.1_darwin-arm64.tar.gz
./kubo/ipfs version
./kubo/ipfs init
./kubo/ipfs daemon & # run in background
```

#### Use Docker

* Uses `172.17.0.1` or  `host.docker.internal` for swarm connect

```
docker pull ipfs/kubo:latest
docker run -d --name ipfs-node -v ipfs-data:/data/ipfs -p 4001:4001 -p 8080:8080 -p 5001:5001 ipfs/kubo:latest
docker logs -f ipfs-node
```

## Run Bulletin Solochain with `--ipfs-server`

```shell
zb_bin=$(ls zombienet-*-*)

# Bulletin Solochain
POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain ./"$zb_bin" -p native spawn ./zombienet/bulletin-polkadot-local.toml
```

### Connect IPFS Nodes

```shell
# Uses Kubo
./kubo/ipfs swarm connect /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
# connect 12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm success

./kubo/ipfs swarm connect /ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
# connect 12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby success
```

```shell
# Use Docker (change 127.0.0.1 -> 172.17.0.1)
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
```

```shell
# Runs script which reconnects every 2 seconds
./scripts/ipfs-reconnect-solo.sh
```

## Run Bulletin (Westend) Parachain with `--ipfs-server`

### Prerequisites 

```shell
mkdir -p ~/local_bridge_testing/bin

# Ensures `polkadot` and `polkadot-parachain` existing
git clone https://github.com/paritytech/polkadot-sdk.git
cd ~/polkadot-sdk

cargo build -p polkadot -r
ls -la target/release/polkadot
cp target/release/polkadot ~/local_bridge_testing/bin
~/local_bridge_testing/bin/polkadot --version
# polkadot 1.20.2-165ba47dc91

cargo build -p polkadot-parachain-bin -r
ls -la target/release/polkadot-parachain
cp target/release/polkadot-parachain ~/local_bridge_testing/bin
~/local_bridge_testing/bin/polkadot-parachain --version
# polkadot-parachain 1.20.2-165ba47dc91
```

```shell
zb_bin=$(ls zombienet-*-*)

# Bulletin Parachain (Westend)
cd ~/projects/polkadot-bulletin-chain
./scripts/create_bulletin_westend_spec.sh
POLKADOT_BINARY_PATH=~/local_bridge_testing/bin/polkadot POLKADOT_PARACHAIN_BINARY_PATH=~/local_bridge_testing/bin/polkadot-parachain ./"$zb_bin" -p native spawn ./zombienet/bulletin-westend-local.toml
```

```shell
# Or run script which reconnects every 2 seconds
./scripts/ipfs-reconnect-westend.sh
```

## Trigger authorize, store and IPFS get

```shell
# cd polkadot-bulletin-chain # make sure we are here
cd examples
npm install @polkadot/api @polkadot/keyring @polkadot/util-crypto @polkadot/util multiformats ipfs-http-client ipfs-unixfs
```

### Example for simple authorizing and store

```shell
node authorize_and_store.js
```

### Example for multipart / chunked content / big files
The code stores one file, splits into chunks and then uploads those chunks to the Bulletin.
It collects all the partial CIDs for each chunk and saves them as a custom metadata JSON file in the Bulletin.

Now we have two examples:
1. **Manual reconstruction** — return the metadata and chunk CIDs, then reconstruct the original file manually.
2. **IPFS DAG feature** —
    * converts the metadata into a DAG-PB descriptor,
    * stores it directly in IPFS,
    * and allows fetching the entire file using a single root CID from an IPFS HTTP gateway (for example: `http://localhost:8080/ipfs/QmW2WQi7j6c7UgJTarActp7tDNikE4B2qXtFCfLPdsgaTQ`).

```shell
node store_chunked_data.js
```
