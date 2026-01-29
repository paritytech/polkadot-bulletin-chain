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

# Peers to monitor
PEERS_TO_CHECK=(
    "/${check_protocol}/${check_host}/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm"
    "/${check_protocol}/${check_host}/tcp/12347/ws/p2p/12D3KooWRkZhiRhsqmrQ28rt73K7V3aCBpqKrLGSXmZ99PTcTZby"
)

while true; do
    # Read all current connections once
    PEERS="$(${check_cmd} swarm peers)"

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
