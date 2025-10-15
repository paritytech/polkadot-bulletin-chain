# How to run

### Install Kubo
```
wget https://dist.ipfs.tech/kubo/v0.35.0/kubo_v0.35.0_linux-amd64.tar.gz
tar -xvzf kubo_v0.35.0_linux-amd64.tar.gz
cd kubo
sudo bash install.sh
ipfs version
ipfs init
ipfs daemon
```
Output:
```
Initializing daemon...
Kubo version: 0.38.0
Repo version: 18
System version: amd64/linux
Golang version: go1.25.1
PeerID: 12D3KooWKZcts5N54ukoNujKXMz6J84A6cn2mtBzxaEsU8uB6uDz
2025/10/08 13:25:25 failed to sufficiently increase receive buffer size (was: 208 kiB, wanted: 7168 kiB, got: 416 kiB). See https://github.com/quic-go/quic-go/wiki/UDP-Buffer-Sizes for details.
Swarm listening on 127.0.0.1:4001 (TCP+UDP)
Swarm listening on 172.17.0.1:4001 (TCP+UDP)
Swarm listening on 192.168.3.242:4001 (TCP+UDP)
Swarm listening on 192.168.4.241:4001 (TCP+UDP)
Swarm listening on [::1]:4001 (TCP+UDP)
Run 'ipfs id' to inspect announced and discovered multiaddrs of this node.
RPC API server listening on /ip4/127.0.0.1/tcp/5001
WebUI: http://127.0.0.1:5001/webui
Gateway server listening on /ip4/127.0.0.1/tcp/8080
Daemon is ready
```

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

### Run Bulletin
```
POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain ./zombienet-linux-x64 -p native spawn ./zombienet/bulletin-polkadot-local.toml
```

### IPFS swarm connect of Bulletin nodes with `--ipfs-server`
```
ipfs swarm connect /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm
# connect 12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm success
ipfs swarm connect /ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby
# connect 12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby success
ipfs swarm peers
```

### Trigger authorize, store and IPFS get
```
# cd polkadot-bulletin-chain - make sure we are here
cd examples
npm install @polkadot/api @polkadot/keyring @polkadot/util-crypto @polkadot/util multiformats ipfs-http-client
node authorize_and_store.js
```
