#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BULLETIN_REPO_PATH="${BULLETIN_REPO_PATH:-$(cd "$SCRIPT_DIR/../../.." && pwd)}"

ZOMBIENET_CONFIG="$BULLETIN_REPO_PATH/zombienet/bulletin-westend-local.toml"
RELAY_ENDPOINT="ws://127.0.0.1:9942"
PARACHAIN_ENDPOINT="ws://127.0.0.1:10000"

SPAWNED_PIDS=()
cleanup() {
    for pid in "${SPAWNED_PIDS[@]}"; do kill -9 "$pid" 2>/dev/null || true; done
    [ -n "$zombienet_pid" ] && pkill -9 -P "$zombienet_pid" 2>/dev/null || true
}
trap cleanup EXIT

echo "=== Bulletin Westend Elastic Scaling Test ==="

[ -f "$BULLETIN_REPO_PATH/zombienet/bulletin-westend-spec.json" ] || bash "$BULLETIN_REPO_PATH/scripts/create_bulletin_westend_spec.sh"
[ -d "$SCRIPT_DIR/node_modules" ] || (cd "$SCRIPT_DIR" && npm install)

echo "Starting network..."
pushd "$BULLETIN_REPO_PATH" > /dev/null
"$ZOMBIENET_BINARY" -p native spawn "$ZOMBIENET_CONFIG" > "$TEST_DIR/logs/zombienet.log" 2>&1 &
zombienet_pid=$!
SPAWNED_PIDS+=($zombienet_pid)
popd > /dev/null

echo "Waiting for relay chain..."
node "$SCRIPT_DIR/wait-for-chain.js" "$RELAY_ENDPOINT" 2>/dev/null
echo "Waiting for parachain..."
node "$SCRIPT_DIR/wait-for-chain.js" "$PARACHAIN_ENDPOINT" 2>/dev/null

echo "Assigning extra cores for elastic scaling..."
bash "$BULLETIN_REPO_PATH/scripts/assign_cores.sh" "$RELAY_ENDPOINT" 1006 0 1 2>/dev/null

echo "Waiting for cores to take effect..."
node "$SCRIPT_DIR/wait-for-chain.js" "$PARACHAIN_ENDPOINT" 3 2>/dev/null

echo "Testing elastic scaling with data storage..."
node "$SCRIPT_DIR/store-and-check-blocktime.js" "$PARACHAIN_ENDPOINT" "//Alice"

echo "=== Elastic scaling test passed! ==="
