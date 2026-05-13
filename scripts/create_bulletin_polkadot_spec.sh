#!/usr/bin/env bash
#
# Generate the zombienet chain spec for bulletin-polkadot from a pre-built
# runtime WASM. The runtime lives in the polkadot-fellows/runtimes repo and
# is built out-of-tree (see the `build-external-runtime` justfile recipe).
#
# Env vars:
#   WASM_PATH - path to bulletin_polkadot_runtime.compact.compressed.wasm

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

: "${WASM_PATH:?WASM_PATH must be set (path to bulletin_polkadot_runtime.compact.compressed.wasm)}"

if [ ! -f "$WASM_PATH" ]; then
	echo "❌ WASM not found at: $WASM_PATH"
	exit 1
fi

cd "$ROOT_DIR"

chain-spec-builder create \
	-p 1010 \
	-c westend \
	-i bulletin-polkadot \
	-n Bulletin \
	-t local \
	-r "$WASM_PATH" \
	named-preset local_testnet

mv chain_spec.json ./zombienet/bulletin-polkadot-spec.json
echo "✅ Wrote ./zombienet/bulletin-polkadot-spec.json"
