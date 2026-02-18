#!/usr/bin/env bash

set -e

PARA_ID="${PARACHAIN_ID:-2487}"

cargo build --release -p bulletin-westend-runtime

# Requires chain-spec-builder from polkadot-sdk (see scripts/setup_parachain_prerequisites.sh)
chain-spec-builder create \
        -p "$PARA_ID" \
        -c westend \
        -i bulletin-westend \
        -n Bulletin \
        -t local \
        -r ./target/release/wbuild/bulletin-westend-runtime/bulletin_westend_runtime.compact.compressed.wasm \
        named-preset local_testnet

mkdir -p ./zombienet
mv chain_spec.json ./zombienet/bulletin-westend-spec.json
echo "Chain spec generated at ./zombienet/bulletin-westend-spec.json (para_id=$PARA_ID)"
