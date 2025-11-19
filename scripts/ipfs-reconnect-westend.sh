#!/bin/bash

# Peers to monitor
PEERS_TO_CHECK=(
    "/ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ"
    "/ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWJ8sqAYtMBX3z3jy2iM98XGLFVzVfUPtmgDzxXSPkVpZZ"
)

while true; do
    # Read all current connections once
    PEERS=$(./kubo/ipfs swarm peers)

    for PEER in "${PEERS_TO_CHECK[@]}"; do
        echo "$PEERS" | grep -q "$PEER"
        if [ $? -ne 0 ]; then
            echo "$(date) - $PEER disconnected. Reconnecting..."
            ./kubo/ipfs swarm connect "$PEER"
        else
            echo "$(date) - $PEER connected."
        fi
    done

    sleep 2
done
