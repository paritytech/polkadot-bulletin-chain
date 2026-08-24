#!/usr/bin/env bash
# Snapshot the block-critical metrics for the node under test over the load window.
# Queries a Prometheus/Thanos endpoint (same metrics the ops stack already scrapes).
#
# Usage:
#   PROM=http://prometheus:9090 CHAIN=<chain> NS=<namespace> WINDOW=10m \
#     scripts/storage-bench/collect-metrics.sh <arm-label>
# Writes <arm-label>.metrics.txt. Run once per arm (nvme / gp3 / sata) x block-time.
#
# The serve load is not constant (sync peers finish mid-run), so slice each run
# into windows instead of one aggregate: set AT to the evaluation instant
# (epoch seconds or RFC3339) and WINDOW to the slice length. Timestamps come
# from wait-peers.sh:
#   loaded (peers syncing):  AT=<last-peer-caught-up> WINDOW=<sync duration>  <arm>-loaded
#   quiet (after catch-up):  AT=<run end>             WINDOW=<remaining time> <arm>-quiet
#
# Bare benchmark box (no ops stack): run a local Prometheus with
# scripts/storage-bench/prometheus-box.yml, then
#   PROM=http://127.0.0.1:9090 SEL='job="author"' MOUNT=/mnt/<arm> \
#     scripts/storage-bench/collect-metrics.sh <arm>
# SEL must single out the author: without it the peers' import histograms
# (their DBs sit on the fast disk) blend into the percentiles. MOUNT switches
# the disk-free row from kubelet to node_exporter.
set -euo pipefail

PROM="${PROM:?set PROM to the Prometheus base URL}"
ARM="${1:?arm label, e.g. gp3-6s or gp3-6s-loaded}"
CHAIN="${CHAIN:-bulletin}"
NS="${NS:-bulletin}"
WINDOW="${WINDOW:-10m}"
AT="${AT:-}"
MOUNT="${MOUNT:-}"
OUT="${ARM}.metrics.txt"

q() { curl -sG "$PROM/api/v1/query" --data-urlencode "query=$1" \
        ${AT:+--data-urlencode "time=$AT"} \
        | python3 -c 'import sys,json;d=json.load(sys.stdin)["data"]["result"];print(d[0]["value"][1] if d else "NA")'; }

# Series selector for the node under test. Default matches the ops stack's
# chain label; on the box set SEL='job="author"' (label from prometheus-box.yml).
SEL="${SEL-chain=~\"$CHAIN\"}"
# For queries that add their own labels; empty SEL must not leave a trailing comma.
SEL_AND="${SEL:+,$SEL}"

if [ -n "$MOUNT" ]; then
  DISK_Q="min(node_filesystem_avail_bytes{mountpoint=\"$MOUNT\"} / node_filesystem_size_bytes{mountpoint=\"$MOUNT\"})"
else
  DISK_Q="min(kubelet_volume_stats_available_bytes{namespace=\"$NS\"} / kubelet_volume_stats_capacity_bytes{namespace=\"$NS\"})"
fi

{
  echo "arm: $ARM   window: $WINDOW   at: ${AT:-now}   $(date -u +%FT%TZ)"
  echo "block import p99 (s):    $(q "histogram_quantile(0.99, sum(rate(substrate_block_verification_and_import_time_bucket{$SEL}[$WINDOW])) by (le))")"
  echo "authoring p99 (s):       $(q "histogram_quantile(0.99, sum(rate(substrate_proposer_block_constructed_bucket{$SEL}[$WINDOW])) by (le))")"
  # Dev-seal metrics from polkadot-sdk#12984 / #12985: the author's own import
  # and the inherent build (the transaction-storage proof read) do not show up
  # in the two stock histograms above.
  echo "own import p99 (s):      $(q "histogram_quantile(0.99, sum(rate(substrate_dev_seal_block_import_time_bucket{$SEL}[$WINDOW])) by (le))")"
  echo "inherent build p99 (s):  $(q "histogram_quantile(0.99, sum(rate(substrate_dev_seal_inherent_data_time_bucket{$SEL}[$WINDOW])) by (le))")"
  echo "best-block rate (1/s):   $(q "sum(rate(substrate_block_height{status=\"best\"$SEL_AND}[$WINDOW]))")"
  echo "disk free ratio:         $(q "$DISK_Q")"
  # Exporter lives in the ops stack only; NA is expected on a bare box.
  echo "permanent storage ratio: $(q "max(bulletin_permanent_storage_used_ratio{$SEL})")"
} | tee "$OUT"
