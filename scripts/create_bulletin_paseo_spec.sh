#!/usr/bin/env bash
# Copyright (C) Parity Technologies (UK) Ltd.
# SPDX-License-Identifier: Apache-2.0


set -e

# Resolve repo root relative to this script's location
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

SPEC_PATH="$ROOT_DIR/zombienet/bulletin-paseo-spec.json"

# Idempotent on cache hit: if the spec already exists, skip the (~4-min) runtime build.
# Caller can force regeneration with FORCE_REBUILD_SPEC=1.
if [ -f "$SPEC_PATH" ] && [ "${FORCE_REBUILD_SPEC:-0}" != "1" ]; then
    echo "Chain spec already at $SPEC_PATH — skipping build (set FORCE_REBUILD_SPEC=1 to override)"
    exit 0
fi

cargo build --release -p bulletin-paseo-runtime

# Requires chain-spec-builder from polkadot-sdk on PATH (run `just binaries-chain-spec-builder`).
cd "$ROOT_DIR"

chain-spec-builder create \
        -p 1501 \
        -c westend \
        -i bulletin-paseo \
        -n Bulletin \
        -t local \
        -r ./target/release/wbuild/bulletin-paseo-runtime/bulletin_paseo_runtime.compact.compressed.wasm \
        named-preset local_testnet

mv chain_spec.json ./zombienet/bulletin-paseo-spec.json
