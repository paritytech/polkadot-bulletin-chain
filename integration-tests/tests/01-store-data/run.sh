#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BULLETIN_REPO_PATH="${BULLETIN_REPO_PATH:-$(cd "$SCRIPT_DIR/../../.." && pwd)}"

ZOMBIENET_CONFIG="$BULLETIN_REPO_PATH/zombienet/bulletin-westend-local.toml"
RELAY_ENDPOINT="ws://127.0.0.1:9942"
PARACHAIN_ENDPOINT="ws://127.0.0.1:10000"
IPFS_URL="http://127.0.0.1:5001"
TEST_DATA="0x48656c6c6f20576f726c64"
ALICE="5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"

SPAWNED_PIDS=()
cleanup() {
    for pid in "${SPAWNED_PIDS[@]}"; do kill -9 "$pid" 2>/dev/null || true; done
    [ -n "$zombienet_pid" ] && pkill -9 -P "$zombienet_pid" 2>/dev/null || true
}
trap cleanup EXIT

echo "=== Bulletin Westend Data Storage Test ==="

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

"$BULLETIN_REPO_PATH/scripts/ipfs-reconnect-westend.sh" > "$TEST_DIR/logs/ipfs.log" 2>&1 &
disown

echo "Checking no initial authorization..."
node "$SCRIPT_DIR/check-authorization.js" "$PARACHAIN_ENDPOINT" "$ALICE" false 2>/dev/null

echo "Authorizing and storing data..."
output=$(node "$SCRIPT_DIR/authorize-and-store.js" "$PARACHAIN_ENDPOINT" "//Alice" "$TEST_DATA" 2>/dev/null)
echo "$output"
CID=$(echo "$output" | grep "OUTPUT_CID=" | cut -d= -f2)

echo "Verifying authorization..."
node "$SCRIPT_DIR/check-authorization.js" "$PARACHAIN_ENDPOINT" "$ALICE" true 2>/dev/null

echo "Verifying data via IPFS..."
sleep 10
node "$SCRIPT_DIR/verify-ipfs.js" "$CID" "$TEST_DATA" "$IPFS_URL" 2>/dev/null

echo "=== All tests passed! ==="
