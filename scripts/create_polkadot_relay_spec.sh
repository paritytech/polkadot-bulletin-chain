#!/usr/bin/env bash
set -e

# Generate a polkadot-local relay chain spec from a pre-built fast-runtime WASM.
#
# The polkadot native runtime was removed from the polkadot binary, so we use
# chain-spec-builder with an externally-built runtime WASM (from
# polkadot-fellows/runtimes with fast-runtime feature) to generate the spec.
#
# Prerequisites:
#   - The WASM must exist at examples/production-runtimes/ (checked into the repo,
#     or rebuilt via ./scripts/build_polkadot_relay_runtime.sh)
#   - chain-spec-builder must be in PATH (cargo install staging-chain-spec-builder)
#
# Output:
#   ./zombienet/polkadot-local-relay-spec.json

WASM_PATH="./examples/production-runtimes/polkadot_runtime.compact.compressed.wasm"
OUTPUT_PATH="./zombienet/polkadot-local-relay-spec.json"

if [ ! -f "${WASM_PATH}" ]; then
    echo "Error: Polkadot relay runtime WASM not found: ${WASM_PATH}" >&2
    echo "  Run ./scripts/build_polkadot_relay_runtime.sh first." >&2
    exit 1
fi

echo "Generating polkadot-local relay chain spec..."
echo "  Runtime WASM: ${WASM_PATH}"

chain-spec-builder create \
    -c polkadot \
    -t local \
    -r "${WASM_PATH}" \
    named-preset local_testnet

mv chain_spec.json "${OUTPUT_PATH}"

echo "  Output: ${OUTPUT_PATH}"
echo "Done."
