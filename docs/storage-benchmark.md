# Storage tier benchmark

Answers one question: does a Bulletin node meet block deadlines with the bulk data
(`columns::TRANSACTION`) on a cheaper tier (SSD, ideally HDD) instead of NVMe? The cheapest
tier that passes sets the storage floor and decides how far the NVMe requirement can drop.

Not runnable on a laptop. Needs the real tiers under a real node: a cluster with node pools on
different storage classes, or a Linux box with the volumes attached.

## Arms

NVMe (baseline), SSD, HDD, at 8 / 24 / 100 TB, at 6 s (current) and 2 s / 500 ms (target)
blocks.
Pre-grow the DB so proof reads hit a cold random location in the full set, not a cached one.

## Method

1. fio pre-screen (no node): per-block IO = small random state read/write + ~8 MiB sequential
   blob write + one random cold read. `fio scripts/storage-bench/fio-block-critical.fio`.
2. Node under load: point the data volume at each tier (real, or emulate on bare metal with
   `scripts/storage-bench/throttle.sh`), then `bulletin-stress-test ... throughput --variants
   MIXED` plus `bitswap`.
3. Capture: `scripts/storage-bench/collect-metrics.sh <arm>`.

## Pass criteria

Block import p99 + authoring p99 well inside the block time; no growing import lag; proof-read p99
low-ms; Bitswap >= 95% within 2 s. Metrics: `substrate_block_verification_and_import_time`,
`substrate_proposer_block_constructed`, `substrate_block_height{status="best"}`,
`kubelet_volume_stats_*`, Bitswap p95.

## Scripts

`scripts/storage-bench/`: `fio-block-critical.fio` (pre-screen), `throttle.sh` (emulate a tier
via cgroup v2 io.max), `collect-metrics.sh` (Prometheus snapshot).
