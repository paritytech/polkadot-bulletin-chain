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

### Run Bulletin nodes with `--ipfs-server`
```
POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain ./zombienet-linux-x64 -p native spawn ./zombienet/bulletin-polkadot-local.toml
```

### Run Kubo locally and connect Bulletin nodes

#### Either locally
```
wget https://dist.ipfs.tech/kubo/v0.38.1/kubo_v0.38.1_linux-amd64.tar.gz
tar -xvzf kubo_v0.38.1_linux-amd64.tar.gz
cd kubo
sudo bash install.sh
ipfs version
ipfs init
ipfs daemon & # run in background

# Connect nodes
ipfs swarm connect /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
# connect 12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm success
ipfs swarm connect /ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
# connect 12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby success
```

#### Or use docker (uses 172.17.0.1 or host.docker.internal for swarm connect)

```
docker pull ipfs/kubo:latest
docker run -d --name ipfs-node   -v ipfs-data:/data/ipfs   -p 4001:4001   -p 8080:8080   -p 5001:5001   ipfs/kubo:latest
docker logs -f ipfs-node
# Connect nodes
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm && docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby

# specific version
# docker pull ipfs  /kubo:v0.35.0
# docker run -d --name ipfs-node-v0.35.0   -v ipfs-data:/data/ipfs-v0.35   -p 4001:4001   -p 8080:8080   -p 5001:5001   ipfs/kubo:v0.35.0
# docker logs -f ipfs-node-v0.35.0
# Connect nodes
# docker exec -it ipfs-node-v0.35.0 ipfs swarm connect /ip4/172.17.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
# docker exec -it ipfs-node-v0.35.0 ipfs swarm connect /ip4/172.17.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
```

### Install dependencies
```bash
# From the root of the repository
npm install
```

### Example for simple authorizing and store

**Using legacy @polkadot/api:**
```
node authorize_and_store.js
```

**Using modern PAPI (Polkadot API):**
```bash
# First, generate the PAPI descriptors (from the root of the repository)
npm run papi:generate

# Then run the PAPI version (from the examples directory)
cd examples
node authorize_and_store_papi.js
```

See [README_PAPI.md](./README_PAPI.md) for more details on using PAPI.

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
