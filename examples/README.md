# How to run

### Build Bulletin
```
git clone https://github.com/paritytech/polkadot-bulletin-chain.git
cd polkadot-bulletin-chain
cargo build --release -p polkadot-bulletin-chain
```

### Download Zombienet
```
cd polkadot-bulletin-chain # make sure we are within the folder
```

#### Mac OS
`zombienet-macos-arm64`

```
curl -L -o zombienet-macos-arm64 https://github.com/paritytech/zombienet/releases/download/v1.3.133/zombienet-macos-arm64
chmod +x zombienet-macos-arm64 
```

### Run Kubo locally

#### Either locally
```
wget https://dist.ipfs.tech/kubo/v0.38.1/kubo_v0.38.1_darwin-arm64.tar.gz
tar -xvzf kubo_v0.38.1_darwin-arm64.tar.gz
# cd kubo
# sudo bash install.sh
./kubo/ipfs version
./kubo/ipfs init
./kubo/ipfs daemon & # run in background
```

## Run Bulletin solochain with `--ipfs-server`
```
# Bulletin Solochain
POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain ./zombienet-macos-arm64 -p native spawn ./zombienet/bulletin-polkadot-local.toml
```

```
# Connect nodes
# ./kubo/ipfs swarm connect /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
# connect 12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm success
# ./kubo/ipfs swarm connect /ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
# connect 12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby success

# Or run script which reconnects every 2 seconds
./scripts/ipfs-reconnect-solo.sh
```

## Run Bulletin (Westend) parachain with `--ipfs-server`
```
mkdir -p ~/local_bridge_testing/bin
cd ~/projects/polkadot-sdk

cargo build -p polkadot -r
ls -la target/release/polkadot
cp target/release/polkadot ~/local_bridge_testing/bin
~/local_bridge_testing/bin/polkadot --version

cd ~/projects/polkadot-sdk
cargo build -p polkadot-parachain-bin -r
ls -la target/release/polkadot-parachain
cp target/release/polkadot-parachain ~/local_bridge_testing/bin
~/local_bridge_testing/bin/polkadot-parachain --version
```

```
# Bulletin parachain (Westend)
cd ~/projects/polkadot-bulletin-chain
./scripts/create_bulletin_westend_spec.sh
POLKADOT_BINARY_PATH=~/local_bridge_testing/bin/polkadot POLKADOT_PARACHAIN_BINARY_PATH=~/local_bridge_testing/bin/polkadot-parachain ./zombienet-macos-arm64 -p native spawn ./zombienet/bulletin-westend-local.toml
```

```
# Or run script which reconnects every 2 seconds
./scripts/ipfs-reconnect-westend.sh
```

## Trigger authorize, store and IPFS get
```
# cd polkadot-bulletin-chain - make sure we are here
cd examples
npm install @polkadot/api @polkadot/keyring @polkadot/util-crypto @polkadot/util multiformats ipfs-http-client ipfs-unixfs
```

### Example for simple authorizing and store
```
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
```
node store_chunked_data.js
```
