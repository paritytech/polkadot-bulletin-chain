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
`substrate_proposer_block_constructed`, `substrate_dev_seal_block_import_time` and
`substrate_dev_seal_inherent_data_time` (polkadot-sdk#12984 / #12985: the author's own import
and the proof read never reach the two stock histograms), `substrate_block_height{status="best"}`,
`kubelet_volume_stats_*`, Bitswap p95.

The Bitswap serve histogram (`substrate_sub_libp2p_bitswap_inbound_request_duration_seconds`)
exists only on the litep2p backend; the libp2p backend registers no bitswap metrics at all. The
dev node hardcodes litep2p, but no peer may be started with `--network-backend libp2p` or its
serve rows read NA.

## Serve-load duty cycle

The full-sync peers are the main serve load and a one-shot cohort: start every peer before the
author seals its first block on top of the snapshot. From that block on the author prunes bodies
from the front of the chain (`--blocks-pruning` == snapshot height), so a full sync from genesis
started any later can never complete. One cohort per snapshot restore; a crashed peer cannot
rejoin the run.

Peers are expected to finish inside the run, so the serve load is not constant and a single 24 h
aggregate dilutes the loaded hours. Record when each peer catches up
(`scripts/storage-bench/wait-peers.sh`, one CSV row per poll) and report two windows via
`collect-metrics.sh` `AT`/`WINDOW`:

1. loaded: run start until the last peer catches up (sync serve + paced bitswap reads)
2. quiet: from there to run end; switch the bitswap driver to the saturation step
   (`bulletin-stress-test bitswap bulk-read --rate 0`) so read pressure does not disappear
   with the peers.

## Bare box metrics

The box has no ops stack, and the node's raw `/metrics` endpoint cannot answer quantile
queries, so run a local Prometheus with `scripts/storage-bench/prometheus-box.yml` (author on
9615, peers 9616+, node_exporter on 9100) and point `collect-metrics.sh` at it:

    PROM=http://127.0.0.1:9090 SEL='job="author"' MOUNT=/mnt/<arm> \
      scripts/storage-bench/collect-metrics.sh <arm>

`SEL` must single out the author or the peers' import histograms (their DBs sit on the fast
disk) blend into the percentiles. `MOUNT` switches the disk-free row from kubelet to
node_exporter. `bulletin_permanent_storage_used_ratio` has no exporter outside the ops stack
and reads NA on the box.

## Scripts

`scripts/storage-bench/`: `fio-block-critical.fio` (pre-screen), `throttle.sh` (emulate a tier
via cgroup v2 io.max), `collect-metrics.sh` (Prometheus snapshot, window-sliceable via
`AT`/`WINDOW`, bare-box capable via `SEL`/`MOUNT`), `wait-peers.sh` (peer catch-up watcher,
emits the window boundaries), `prometheus-box.yml` (local Prometheus for the box).
