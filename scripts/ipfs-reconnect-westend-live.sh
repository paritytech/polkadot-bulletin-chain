#!/bin/bash

THIS_DIR=$(cd $(dirname $0); pwd)

# IPFS reconnect script for LIVE Westend Bulletin network
#
# This script connects your local IPFS (kubo) to the live Westend Bulletin collators.
#
# Usage:
#   ./ipfs-reconnect-westend-live.sh [mode] [sleep_interval]
#
# Arguments:
#   mode           - "local" (uses local kubo) or "docker" (uses docker exec)
#   sleep_interval - seconds between reconnect checks (default: 10)
#
# Environment variables (set these with the actual live collator peer info):
#   WESTEND_PEER1_ID   - Peer ID of first collator
#   WESTEND_PEER1_ADDR - Multiaddr of first collator (e.g., /dns4/collator1.example.com/tcp/30333/ws)
#   WESTEND_PEER2_ID   - Peer ID of second collator (optional)
#   WESTEND_PEER2_ADDR - Multiaddr of second collator (optional)

mode="${1:-docker}"
sleep_interval="${2:-10}"

if [ "$mode" = "docker" ]; then
    check_cmd="docker exec ipfs-node ipfs"
else
    check_cmd="${THIS_DIR}/../kubo/ipfs"
fi

# Check if peer info is configured
if [ -z "$WESTEND_PEER1_ID" ] || [ -z "$WESTEND_PEER1_ADDR" ]; then
    echo "ERROR: Live Westend peer info not configured."
    echo ""
    echo "Please set the following environment variables with the actual collator IPFS peer info:"
    echo "  export WESTEND_PEER1_ID='12D3KooW...'"
    echo "  export WESTEND_PEER1_ADDR='/dns4/collator1.example.com/tcp/30333/ws'"
    echo ""
    echo "You can obtain these from:"
    echo "  - Westend Bulletin collator operators"
    echo "  - Polkadot Bulletin documentation"
    echo "  - The collator's libp2p identity in logs"
    echo ""
    exit 1
fi

# Build peer addresses
PEER_IDS=("$WESTEND_PEER1_ID")
declare -A PEER_ADDRS
PEER_ADDRS["$WESTEND_PEER1_ID"]="${WESTEND_PEER1_ADDR}/p2p/${WESTEND_PEER1_ID}"

if [ -n "$WESTEND_PEER2_ID" ] && [ -n "$WESTEND_PEER2_ADDR" ]; then
    PEER_IDS+=("$WESTEND_PEER2_ID")
    PEER_ADDRS["$WESTEND_PEER2_ID"]="${WESTEND_PEER2_ADDR}/p2p/${WESTEND_PEER2_ID}"
fi

echo "Starting IPFS reconnect for LIVE Westend Bulletin..."
echo "Mode: $mode"
echo "Check interval: ${sleep_interval}s"
echo "Peers:"
for PEER_ID in "${PEER_IDS[@]}"; do
    echo "  - ${PEER_ADDRS[$PEER_ID]}"
done
echo ""

while true; do
    # Read all current connections once
    PEERS="$(${check_cmd} swarm peers 2>/dev/null)"

    for PEER_ID in "${PEER_IDS[@]}"; do
        if echo "$PEERS" | grep -q "$PEER_ID"; then
            echo "$(date) - $PEER_ID connected."
        else
            echo "$(date) - $PEER_ID disconnected. Reconnecting..."
            ${check_cmd} swarm connect "${PEER_ADDRS[$PEER_ID]}" 2>&1 || true
        fi
    done

    sleep "$sleep_interval"
done
