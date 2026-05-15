#!/usr/bin/env bash
# Build westend-local relay chain-spec from the cached westend-runtime WASM.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SPEC_PATH="$ROOT_DIR/zombienet/westend-local-spec.json"

RELAY_RUNTIME_DIR="$(cd "$ROOT_DIR" && just binaries-relay-runtime)"
WASM_PATH="$RELAY_RUNTIME_DIR/westend_runtime.compact.compressed.wasm"
[ -f "$WASM_PATH" ] || { echo "relay-runtime WASM not found at $WASM_PATH" >&2; exit 1; }

# Skip when spec is newer than WASM; FORCE_REBUILD_RELAY_SPEC=1 to override.
if [ -f "$SPEC_PATH" ] \
		&& [ "$SPEC_PATH" -nt "$WASM_PATH" ] \
		&& [ "${FORCE_REBUILD_RELAY_SPEC:-0}" != "1" ]; then
	echo "Relay chain spec already at $SPEC_PATH — skipping build (set FORCE_REBUILD_RELAY_SPEC=1 to override)"
	exit 0
fi

PRESET="${RELAY_SPEC_PRESET:-local_testnet}"

mkdir -p "$ROOT_DIR/zombienet"
cd "$ROOT_DIR"
chain-spec-builder create \
	-n "Westend Local" \
	-i westend_local \
	-t local \
	-r "$WASM_PATH" \
	named-preset "$PRESET"

mv chain_spec.json "$SPEC_PATH"
echo "Relay chain spec generated at $SPEC_PATH (preset=$PRESET)"
