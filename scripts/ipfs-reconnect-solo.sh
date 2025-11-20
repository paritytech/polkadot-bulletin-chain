#!/bin/bash

# Choose mode based on argument
mode="${1:-docker}"
if [ "$mode" = "docker" ]; then
    check_cmd="docker exec ipfs-node ipfs"
    check_host="172.17.0.1"
else
    check_cmd="./kubo/ipfs"
    check_host="127.0.0.1"
fi

# Peers to monitor
PEERS_TO_CHECK=(
    "/ip4/${check_host}/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm"
    "/ip4/${check_host}/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby"
)

while true; do
    # Read all current connections once
    PEERS="$($check_cmd swarm peers)"

    for PEER in "${PEERS_TO_CHECK[@]}"; do
        echo "$PEERS" | grep -q "$PEER"
        if [ $? -ne 0 ]; then
            echo "$(date) - $PEER disconnected. Reconnecting..."
            $check_cmd swarm connect "$PEER"
        else
            echo "$(date) - $PEER connected."
        fi
    done

    sleep 2
done
