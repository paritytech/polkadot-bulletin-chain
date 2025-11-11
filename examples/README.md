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
npm install @polkadot/api @polkadot/keyring @polkadot/util-crypto @polkadot/util multiformats ipfs-http-client
node authorize_and_store.js
```
# Troubleshooting

## Build Bulletin Mac OS

### Algorithm file not found error

If you encounter an error similar to:

```
warning: cxx@1.0.186: In file included from /Users/ndk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cxx-1.0.186/src/cxx.cc:1:
warning: cxx@1.0.186: /Users/ndk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/cxx-1.0.186/src/../include/cxx.h:2:10: fatal error: 'algorithm' file not found
warning: cxx@1.0.186:     2 | #include <algorithm>
warning: cxx@1.0.186:       |          ^~~~~~~~~~~
warning: cxx@1.0.186: 1 error generated.
error: failed to run custom build command for `cxx v1.0.186`
```

This typically means your C++ standard library headers can’t be found by the compiler. This is a toolchain setup issue.

To fix:
- Run `xcode-select --install`. 
- If it says “already installed”, reinstall them (sometimes they break after OS updates):

```bash
sudo rm -rf /Library/Developer/CommandLineTools
xcode-select --install
```

- Check the Active Developer Path: `xcode-select -p`. It should output one of: `/Applications/Xcode.app/Contents/Developer`, `/Library/Developer/CommandLineTools`
- If it’s empty or incorrect, set it manually: `sudo xcode-select --switch /Library/Developer/CommandLineTools`

### dyld: Library not loaded: @rpath/libclang.dylib

This means that your build script tried to use `libclang` (from LLVM) but couldn’t find it anywhere on your system or in the `DYLD_LIBRARY_PATH`.

To fix:`brew install llvm`and 
```
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
export LD_LIBRARY_PATH="$LIBCLANG_PATH:$LD_LIBRARY_PATH"
export DYLD_LIBRARY_PATH="$LIBCLANG_PATH:$DYLD_LIBRARY_PATH"
export PATH="$(brew --prefix llvm)/bin:$PATH"
```

Now verify `libclang.dylib` exists:
- `ls "$(brew --prefix llvm)/lib/libclang.dylib"`

If that file exists all good, you can rebuild the project now: 
```
cargo clean
cargo build --release
```
