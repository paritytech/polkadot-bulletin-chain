# Storage-tier benchmark (Phase 1)

Answers: does a Bulletin node need NVMe, or does a cheaper tier (SSD / HDD) meet block
deadlines at scale? Full plan and verdict logic: `docs/storage-benchmark.md`.

Not runnable on a laptop - it compares storage tiers under a real node. Run on a cluster with
two node pools on different `storageClassName`, or a Linux box with the volumes attached.

## Steps

1. **Pre-screen (fast, no node)** - run on each tier's mounted volume:
   ```bash
   sudo TARGET=/mnt/data/bench SIZE=1700g fio fio-block-critical.fio
   ```
   The go/no-go number is the `sync-serve-random-read` job (serving historical bodies to a
   syncing peer); it runs alongside the block-critical jobs, so a tier passes only if state and
   proof p99 stay well inside the block time (6 s now, 2 s / 500 ms targets) while sync-serve
   saturates the disk. If it can't, the tier already fails here. Needs SIZE + ~100g free on TARGET.

   HDD needs latency, not just an IOPS cap: on bare metal use `hdd-emulate.sh` (dm-delay) to
   inject ~7 ms per-read seek before the pre-screen; `throttle.sh hdd` only models the IOPS wall.

2. **Node under load** - point the node's data volume at each tier (real gp3/SATA volume, or
   emulate on bare metal with `throttle.sh`), sync or pre-grow the DB to ~1.7 TiB, then:
   ```bash
   bulletin-stress-test --ws-url ws://<node>:9944 --authorizer-seed <auth> \
     --submitters 16 --target-blocks 500 --output-file thr.json throughput --variants MIXED
   ```

3. **Capture metrics** during the load window:
   ```bash
   PROM=http://prometheus:9090 CHAIN=<chain> scripts/storage-bench/collect-metrics.sh gp3-6s
   ```

4. **Verdict** - compare each arm x block-time against the budgets in the plan doc. PASS on
   gp3/SATA -> config change (update hardware rec). FAIL -> the per-column split is justified.

Repeat for arms: nvme (baseline), capacity-flash (gp3/sata), hdd (blob tier only - state never
moves off NVMe) x block-time (6 s, 2 s, 500 ms).
