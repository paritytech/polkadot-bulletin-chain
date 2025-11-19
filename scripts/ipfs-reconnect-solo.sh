#!/bin/bash

# Peers to monitor
PEERS_TO_CHECK=(
    "/ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm"
    "/ip4/127.0.0.1/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby"
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
