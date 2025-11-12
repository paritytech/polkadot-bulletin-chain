# How to run

### Download zombienet
```
wget https://github.com/paritytech/zombienet/releases/download/v1.3.133/zombienet-linux-x64
chmod +x zombienet-linux-x64
```

### Build Bulletin
```
git clone https://github.com/paritytech/polkadot-bulletin-chain.git
cd polkadot-bulletin-chain
cargo build --release -p polkadot-bulletin-chain
```

### Download Zombienet
```
# cd polkadot-bulletin-chain - make sure we are here
wget https://github.com/paritytech/zombienet/releases/download/v1.3.133/zombienet-linux-x64
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
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
docker exec -it ipfs-node ipfs swarm connect /ip4/172.17.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby

# specific version
# docker pull ipfs  /kubo:v0.35.0
# docker run -d --name ipfs-node-v0.35.0   -v ipfs-data:/data/ipfs-v0.35   -p 4001:4001   -p 8080:8080   -p 5001:5001   ipfs/kubo:v0.35.0
# docker logs -f ipfs-node-v0.35.0
# Connect nodes
# docker exec -it ipfs-node-v0.35.0 ipfs swarm connect /ip4/172.17.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
# docker exec -it ipfs-node-v0.35.0 ipfs swarm connect /ip4/172.17.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
```

### Trigger authorize, store and IPFS get
```
# cd polkadot-bulletin-chain - make sure we are here
cd examples
npm install @polkadot/api @polkadot/keyring @polkadot/util-crypto @polkadot/util multiformats ipfs-http-client ipfs-unixfs
node authorize_and_store.js
node store_chunked_data.js
```
