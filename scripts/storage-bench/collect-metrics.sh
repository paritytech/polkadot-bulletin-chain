#!/usr/bin/env bash
# Snapshot the block-critical metrics for the node under test over the load window.
# Queries a Prometheus/Thanos endpoint (same metrics the ops stack already scrapes).
#
# Usage:
#   PROM=http://prometheus:9090 CHAIN=<chain> WINDOW=10m \
#     scripts/storage-bench/collect-metrics.sh <arm-label>
# Writes <arm-label>.metrics.txt. Run once per arm (nvme / gp3 / sata) x block-time.
set -euo pipefail

PROM="${PROM:?set PROM to the Prometheus base URL}"
ARM="${1:?arm label, e.g. gp3-24s}"
CHAIN="${CHAIN:-bulletin}"
WINDOW="${WINDOW:-10m}"
OUT="${ARM}.metrics.txt"

q() { curl -sG "$PROM/api/v1/query" --data-urlencode "query=$1" \
        | python3 -c 'import sys,json;d=json.load(sys.stdin)["data"]["result"];print(d[0]["value"][1] if d else "NA")'; }

SEL="chain=~\"$CHAIN\""

{
  echo "arm: $ARM   window: $WINDOW   $(date -u +%FT%TZ)"
  echo "block import p99 (s):    $(q "histogram_quantile(0.99, sum(rate(substrate_block_verification_and_import_time_bucket{$SEL}[$WINDOW])) by (le))")"
  echo "authoring p99 (s):       $(q "histogram_quantile(0.99, sum(rate(substrate_proposer_block_constructed_bucket{$SEL}[$WINDOW])) by (le))")"
  echo "best-block rate (1/s):   $(q "sum(rate(substrate_block_height{status=\"best\",$SEL}[$WINDOW]))")"
  echo "disk free ratio:         $(q "min(kubelet_volume_stats_available_bytes{namespace=\"bulletin\"} / kubelet_volume_stats_capacity_bytes{namespace=\"bulletin\"})")"
  echo "permanent storage ratio: $(q "max(bulletin_permanent_storage_used_ratio{$SEL})")"
} | tee "$OUT"
