# Bulletin Chain Integration Tests

Integration tests for the Polkadot Bulletin Chain (Westend Parachain).

## Prerequisites

```bash
mkdir -p ~/local_bridge_testing/bin

# Build polkadot and polkadot-parachain from polkadot-sdk
# Copy to ~/local_bridge_testing/bin/

# Download zombienet
wget "https://github.com/paritytech/zombienet/releases/download/v1.3.133/zombienet-linux-x64" \
  -O ~/local_bridge_testing/bin/zombienet
chmod +x ~/local_bridge_testing/bin/zombienet

# Generate chain spec
cd polkadot-bulletin-chain
./scripts/create_bulletin_westend_spec.sh

# Install and start Kubo
wget https://dist.ipfs.tech/kubo/v0.38.1/kubo_v0.38.1_darwin-arm64.tar.gz
tar -xvzf kubo_v0.38.1_darwin-arm64.tar.gz
./kubo/ipfs version
./kubo/ipfs init
./kubo/ipfs daemon &
```

## Running Tests

```bash
./integration-tests/run-test.sh 01-store-data
```
