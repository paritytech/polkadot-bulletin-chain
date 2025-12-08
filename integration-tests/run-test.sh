#!/bin/bash
#
# Integration test runner for Polkadot Bulletin Chain (Westend Parachain).
#
# Usage:
#   ./run-test.sh <test_name>
#
# Example:
#   ./run-test.sh 01-store-data
#   ./run-test.sh 02-elastic-scaling
#
# Prerequisites:
#   - polkadot binary at ~/local_bridge_testing/bin/polkadot
#   - polkadot-parachain binary at ~/local_bridge_testing/bin/polkadot-parachain
#   - zombienet binary at ~/local_bridge_testing/bin/zombienet
#   - Node.js installed

set -e

# Kill any leftover processes from previous runs (kills all zombienet processes
# maybe too strict? Solution to problems when killing and restarting tests)
cleanup_old_processes() {
    echo "Killing any running zombienet processes..."
    pkill -9 -f "zombienet.*spawn.*bulletin-westend" 2>/dev/null || true
    pkill -9 -f "polkadot.*westend-local" 2>/dev/null || true
    pkill -9 -f "polkadot-parachain.*bulletin-westend" 2>/dev/null || true
    pkill -9 -f "ipfs-reconnect-westend" 2>/dev/null || true
    sleep 1
}

# Cleanup function for this test run
cleanup() {
    echo ""
    echo "Cleaning up test environment..."
    
    # Kill all child processes
    pkill -9 -P $$ 2>/dev/null || true
    
    # Also kill by pattern in case they escaped the process tree
    pkill -9 -f "zombienet.*spawn.*bulletin-westend" 2>/dev/null || true
    pkill -9 -f "polkadot.*westend-local" 2>/dev/null || true
    pkill -9 -f "polkadot-parachain.*bulletin-westend" 2>/dev/null || true
    pkill -9 -f "ipfs-reconnect-westend" 2>/dev/null || true
    
    echo "Cleanup complete."
}

# Set trap for cleanup on exit
trap cleanup SIGINT SIGTERM EXIT

test=$1

if [ -z "$test" ]; then
    echo "Usage: $0 <test_name>"
    echo ""
    echo "Available tests:"
    for dir in "${BASH_SOURCE%/*}"/tests/*/; do
        if [ -d "$dir" ]; then
            basename "$dir"
        fi
    done
    exit 1
fi

# Cleanup old processes first
cleanup_old_processes

# Setup paths
export LOCAL_BRIDGE_TESTING_PATH=${LOCAL_BRIDGE_TESTING_PATH:-~/local_bridge_testing}
export BULLETIN_REPO_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Binary paths (default paths can be overridden by env vars)
export ZOMBIENET_BINARY=${ZOMBIENET_BINARY:-$LOCAL_BRIDGE_TESTING_PATH/bin/zombienet}
export POLKADOT_BINARY_PATH=${POLKADOT_BINARY_PATH:-$LOCAL_BRIDGE_TESTING_PATH/bin/polkadot}
export POLKADOT_PARACHAIN_BINARY_PATH=${POLKADOT_PARACHAIN_BINARY_PATH:-$LOCAL_BRIDGE_TESTING_PATH/bin/polkadot-parachain}

# Verify binaries exist
for bin in "$ZOMBIENET_BINARY" "$POLKADOT_BINARY_PATH" "$POLKADOT_PARACHAIN_BINARY_PATH"; do
    if [ ! -f "$bin" ]; then
        echo "Error: Required binary not found: $bin"
        echo ""
        echo "Please ensure the following binaries are installed:"
        echo "  - $ZOMBIENET_BINARY"
        echo "  - $POLKADOT_BINARY_PATH"
        echo "  - $POLKADOT_PARACHAIN_BINARY_PATH"
        exit 1
    fi
done

# Create test directory
export TEST_DIR=$(mktemp -d /tmp/bulletin-tests-XXXXX)
mkdir -p "$TEST_DIR/logs"
echo "Test directory: $TEST_DIR"
echo ""

# Run the test
test_script="${BASH_SOURCE%/*}/tests/$test/run.sh"
if [ ! -f "$test_script" ]; then
    echo "Error: Test '$test' not found at $test_script"
    exit 1
fi

echo "Running test: $test"
echo "================================================"
echo ""

bash "$test_script"
