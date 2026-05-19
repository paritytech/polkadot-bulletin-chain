#!/usr/bin/env bash
# Boot `polkadot-omni-node` against a chainspec and wait until the parachain
# imports a few blocks.
#
# Usage: chainspec_sync_check.sh <chainspec> <relay-chain-rpc-url>
#
# Env overrides (with defaults):
#   TARGET_BLOCKS=3          Best-block height the node must reach to pass.
#   TIMEOUT_SECONDS=600      Hard cap on how long to wait for that height.
#   RPC_PORT=9944            Local RPC port to bind for progress polling.
#   LOG=omni-node.log        File where the node's stdout/stderr is captured.

set -euo pipefail

CHAINSPEC="${1:-}"
RELAY_RPC="${2:-}"
TARGET_BLOCKS="${TARGET_BLOCKS:-3}"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-600}"
RPC_PORT="${RPC_PORT:-9944}"
LOG="${LOG:-omni-node.log}"

if [ -z "$CHAINSPEC" ] || [ -z "$RELAY_RPC" ]; then
    echo "usage: $0 <chainspec> <relay-chain-rpc-url>" >&2
    exit 2
fi

polkadot-omni-node \
    --chain "$CHAINSPEC" \
    --tmp \
    --no-hardware-benchmarks \
    --rpc-port "$RPC_PORT" \
    --relay-chain-rpc-urls "$RELAY_RPC" \
    --log info \
    > "$LOG" 2>&1 &
NODE_PID=$!

cleanup() {
    kill "$NODE_PID" 2>/dev/null || true
    wait "$NODE_PID" 2>/dev/null || true
}
trap cleanup EXIT

deadline=$(( $(date +%s) + TIMEOUT_SECONDS ))
best=0
while [ "$(date +%s)" -lt "$deadline" ]; do
    if ! kill -0 "$NODE_PID" 2>/dev/null; then
        echo "polkadot-omni-node exited before reaching block $TARGET_BLOCKS"
        tail -n 200 "$LOG"
        exit 1
    fi

    response=$(curl -fs -m 5 -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"chain_getHeader","params":[],"id":1}' \
        "http://127.0.0.1:${RPC_PORT}" || true)
    hex=$(echo "$response" | jq -r '.result.number // "0x0"' 2>/dev/null || echo "0x0")
    best=$(( hex ))
    echo "best block: $best"

    if [ "$best" -ge "$TARGET_BLOCKS" ]; then
        echo "imported $best blocks for $CHAINSPEC"
        exit 0
    fi

    sleep 10
done

echo "did not import $TARGET_BLOCKS blocks within ${TIMEOUT_SECONDS}s (last best=$best)"
tail -n 200 "$LOG"
exit 1
