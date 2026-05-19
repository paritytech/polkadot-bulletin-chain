#!/usr/bin/env bash

set -e

# Resolve repo root relative to this script's location
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

PARA_ID="${PARACHAIN_ID:-2487}"
SPEC_PATH="$ROOT_DIR/zombienet/bulletin-westend-spec.json"

# Idempotent on cache hit: if the spec already exists, skip the (~4-min) runtime build.
# Caller can force regeneration with FORCE_REBUILD_SPEC=1.
if [ -f "$SPEC_PATH" ] && [ "${FORCE_REBUILD_SPEC:-0}" != "1" ]; then
    echo "Chain spec already at $SPEC_PATH — skipping build (set FORCE_REBUILD_SPEC=1 to override)"
    exit 0
fi

cargo build --release -p bulletin-westend-runtime

# Requires chain-spec-builder from polkadot-sdk on PATH (run `just binaries-chain-spec-builder`).
cd "$ROOT_DIR"

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
