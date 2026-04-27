#!/usr/bin/env bash

set -e

# Resolve repo root relative to this script's location
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

cargo build --release -p bulletin-westend-runtime

# cargo install staging-chain-spec-builder
cd "$ROOT_DIR"

chain-spec-builder create \
        -p 1010 \
        -c westend \
        -i bulletin-westend \
        -n Bulletin \
        -t local \
        -r ./target/release/wbuild/bulletin-westend-runtime/bulletin_westend_runtime.compact.compressed.wasm \
        named-preset local_testnet

mv chain_spec.json ./zombienet/bulletin-westend-spec.json
