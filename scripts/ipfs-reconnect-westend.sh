#!/bin/bash

THIS_DIR=$(cd $(dirname $0); pwd)

# Arguments: mode [sleep_interval]
mode="${1:-local}"
sleep_interval="${2:-2}"
if [ "$mode" = "docker" ]; then
    check_cmd="docker exec ipfs-node ipfs"

    if [[ "$OSTYPE" == "darwin"* ]]; then
      # macOS - use dns4/host.docker.internal (bridge network)
      check_protocol="dns4"
      check_host="host.docker.internal"
    else
      # Linux - use ip4/127.0.0.1 (host network mode)
      check_protocol="ip4"
      check_host="127.0.0.1"
    fi
else
    check_cmd="${THIS_DIR}/../kubo/ipfs"
    check_protocol="ip4"
    check_host="127.0.0.1"
fi

# Peers to monitor (WebSocket ports: 10002, 12348)
PEERS_TO_CHECK=(
    "/${check_protocol}/${check_host}/tcp/10002/ws/p2p/12D3KooWJKVVNYByvML4Pgx1GWAYryYo6exA68jQX9Mw3AJ6G5gQ"
    "/${check_protocol}/${check_host}/tcp/12348/ws/p2p/12D3KooWJ8sqAYtMBX3z3jy2iM98XGLFVzVfUPtmgDzxXSPkVpZZ"
)

while true; do
    # Read all current connections once
    PEERS="$(${check_cmd} swarm peers)"
    echo "Connected peers: $PEERS"

    for PEER in "${PEERS_TO_CHECK[@]}"; do
        echo "$PEERS" | grep -q "$PEER"
        if [ $? -ne 0 ]; then
            echo "$(date) - $PEER disconnected. Reconnecting..."
            ${check_cmd} swarm connect "$PEER"
        else
            echo "$(date) - $PEER connected."
        fi
    done

    sleep "$sleep_interval"
done
