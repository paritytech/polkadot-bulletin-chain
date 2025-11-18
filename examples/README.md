# How to run

### Build Bulletin
```
git clone https://github.com/paritytech/polkadot-bulletin-chain.git
cd polkadot-bulletin-chain
cargo build --release -p polkadot-bulletin-chain
```

### Download Zombienet
```
cd polkadot-bulletin-chain - make sure we are within the folder
```

#### Linux
```
wget https://github.com/paritytech/zombienet/releases/download/v1.3.133/zombienet-linux-x64
chmod +x zombienet-linux-x64
```

#### Mac OS
`zombienet-macos-arm64` or `zombienet-macos-x64`

```
curl -L -o zombienet-macos-arm64 https://github.com/paritytech/zombienet/releases/download/v1.3.133/zombienet-macos-arm64
chmod +x zombienet-macos-arm64 
```

### Run Kubo locally

#### Either locally
```
wget https://dist.ipfs.tech/kubo/v0.38.1/kubo_v0.38.1_linux-amd64.tar.gz
tar -xvzf kubo_v0.38.1_linux-amd64.tar.gz
cd kubo
sudo bash install.sh
ipfs version
ipfs init
ipfs daemon & # run in background
```

#### Or use docker (uses 172.17.0.1 or host.docker.internal for swarm connect)

```
docker pull ipfs/kubo:latest
docker run -d --name ipfs-node   -v ipfs-data:/data/ipfs   -p 4001:4001   -p 8080:8080   -p 5001:5001   ipfs/kubo:latest
docker logs -f ipfs-node
```

## Run Bulletin solochain with `--ipfs-server`
```
# Bulletin Solochain
POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain ./zombienet-linux-x64 -p native spawn ./zombienet/bulletin-polkadot-local.toml

# Connect nodes
ipfs swarm connect /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
# connect 12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm success
ipfs swarm connect /ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
# connect 12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby success

# Or docker (change 127.0.0.1 -> 172.17.0.1)
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
```

## Run Bulletin (Westend) parachain with `--ipfs-server`
```
# Bulletin parachain (Westend)
./scripts/create_bulletin_westend_spec.sh
POLKADOT_BINARY_PATH=~/local_bridge_testing/bin/polkadot POLKADOT_PARACHAIN_BINARY_PATH=~/local_bridge_testing/bin/polkadot-parachain ./zombienet-linux-x64 -p native spawn ./zombienet/bulletin-westend-local.toml

# For docker (change 127.0.0.1 -> 172.17.0.1)
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/10001/ws/p2p/12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/12347/ws/p2p/12D3KooWJ8sqAYtMBX3z3jy2iM98XGLFVzVfUPtmgDzxXSPkVpZZ
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
