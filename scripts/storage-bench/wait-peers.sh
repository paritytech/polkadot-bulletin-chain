#!/usr/bin/env bash
# Watch the sync peers catch up to the author and record when each one finishes.
#
# The full-sync peers are a one-shot cohort: every peer must be running before
# the author seals its first block on top of the snapshot. From that block on,
# the author prunes bodies from the front of the chain (blocks-pruning ==
# snapshot height), so a full sync from genesis started later can never
# complete. One cohort per snapshot restore; a crashed peer cannot rejoin.
#
# Emits one CSV row per peer per poll and exits once every peer is caught up.
# The caught_up timestamps are the window boundaries for collect-metrics.sh
# (loaded vs quiet), and the moment to switch the bitswap driver to the
# saturation step so read pressure does not disappear with the peers.
#
# Usage:
#   AUTHOR_RPC=http://127.0.0.1:9944 \
#   PEER_RPCS=http://127.0.0.1:9945,http://127.0.0.1:9946,http://127.0.0.1:9947 \
#     scripts/storage-bench/wait-peers.sh
set -euo pipefail

AUTHOR_RPC="${AUTHOR_RPC:?author RPC URL}"
PEER_RPCS="${PEER_RPCS:?comma-separated peer RPC URLs}"
OUT="${OUT:-peers.csv}"
# Blocks behind the author that still count as caught up (tip chatter).
LAG="${LAG:-5}"
POLL="${POLL:-30}"

height() {
  curl -s -m 5 -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"chain_getHeader","params":[]}' "$1" \
    | python3 -c 'import sys,json; print(int(json.load(sys.stdin)["result"]["number"],16))' \
    2>/dev/null || echo -1
}

IFS=',' read -ra PEERS <<< "$PEER_RPCS"
declare -A done_at
echo "ts,peer,height,author_height,event" > "$OUT"

while :; do
  author=$(height "$AUTHOR_RPC")
  all_done=1
  for peer in "${PEERS[@]}"; do
    [ -n "${done_at[$peer]:-}" ] && continue
    h=$(height "$peer")
    ts=$(date -u +%s)
    if [ "$h" -ge 0 ] && [ "$author" -ge 0 ] && [ $((author - h)) -le "$LAG" ]; then
      done_at[$peer]=$ts
      echo "$ts,$peer,$h,$author,caught_up" >> "$OUT"
      echo "peer $peer caught up at epoch $ts (height $h, author $author)" >&2
    else
      echo "$ts,$peer,$h,$author,syncing" >> "$OUT"
      all_done=0
    fi
  done
  [ "$all_done" -eq 1 ] && { echo "all peers caught up" >&2; exit 0; }
  sleep "$POLL"
done
