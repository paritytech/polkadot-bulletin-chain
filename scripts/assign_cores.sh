#!/bin/bash
#
# Assign extra cores to a parachain on the relay chain.
#
# Usage:
#   ./assign_cores.sh <relay_endpoint> <para_id> <cores...>
#
# Example:
#   ./assign_cores.sh ws://localhost:9942 1006 0 1 2

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CMD_DIR="${SCRIPT_DIR}/assign_cores"

# Check if node is available
if ! command -v node &> /dev/null; then
    echo "Error: Node.js is required but not installed."
    echo "Please install Node.js and try again."
    exit 1
fi

# Check arguments
if [ $# -lt 3 ]; then
    echo "Usage: $0 <relay_endpoint> <para_id> <cores...>"
    echo ""
    echo "Arguments:"
    echo "  relay_endpoint  WebSocket endpoint of the relay chain (e.g., ws://localhost:9942)"
    echo "  para_id         Parachain ID to assign cores to (e.g., 1006)"
    echo "  cores           Space-separated list of core numbers to assign"
    echo ""
    echo "Example:"
    echo "  $0 ws://localhost:9942 1006 0 1 2"
    exit 1
fi

# Install npm dependencies if node_modules doesn't exist
if [ ! -d "${CMD_DIR}/node_modules" ]; then
    echo "Installing npm dependencies..."
    pushd "${CMD_DIR}" > /dev/null
    npm install
    popd > /dev/null
    echo ""
fi

# Run the assign_cores script
echo "Assigning cores to parachain..."
node "${CMD_DIR}/assign_cores.js" "$@"
