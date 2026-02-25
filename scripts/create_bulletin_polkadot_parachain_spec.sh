#!/usr/bin/env bash

set -e

cargo build --release -p bulletin-polkadot-parachain-runtime

# cargo install staging-chain-spec-builder
chain-spec-builder create \
        -p 1006 \
        -c westend \
        -i bulletin-polkadot \
        -n Bulletin \
        -t local \
        -r ./target/release/wbuild/bulletin-polkadot-parachain-runtime/bulletin_polkadot_parachain_runtime.compact.compressed.wasm \
        named-preset local_testnet

mv chain_spec.json ./zombienet/bulletin-polkadot-parachain-spec.json
