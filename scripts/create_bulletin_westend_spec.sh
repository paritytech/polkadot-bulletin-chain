#!/usr/bin/env bash

set -e

cargo build --release -p bulletin-westend-runtime

# cargo install staging-chain-spec-builder
chain-spec-builder create \
        -p 1006 \
        -c westend \
        -i bulletin-westend \
        -n Bulletin \
        -t local \
        -r ./target/release/wbuild/bulletin-westend-runtime/bulletin_westend_runtime.compact.compressed.wasm \
        named-preset local_testnet

mv chain_spec.json bulletin-westend-spec.json
cp bulletin-westend-spec.json ./zombienet/bulletin-westend-spec.json
