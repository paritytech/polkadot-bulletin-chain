# Storage-tier benchmark (Phase -1)

Answers: does a Bulletin node need NVMe, or does a cheaper tier (SSD / HDD) meet block
deadlines at scale? Full plan and verdict logic: `docs/storage-benchmark.md`.

Not runnable on a laptop - it compares storage tiers under a real node. Run on the Summit k8s
setup (two pools with different `storageClassName`) or a Linux box with the volumes attached.

## Steps

1. **Pre-screen (fast, no node)** - run on each tier's mounted volume:
   ```bash
   sudo TARGET=/mnt/data/bench SIZE=1700g fio fio-block-critical.fio
   ```
   If p99 latencies fit well inside the slot (24 s now, 2 s future), proceed; if a tier
   saturates, it already fails.

2. **Node under load** - point the node's data volume at each tier (real gp3/SATA volume, or
   emulate on bare metal with `throttle.sh`), sync or pre-grow the DB to ~1.7 TiB, then:
   ```bash
   bulletin-stress-test --ws-url ws://<node>:9944 --authorizer-seed <auth> \
     --submitters 16 --target-blocks 500 --output-file thr.json throughput --variants MIXED
   ```

3. **Capture metrics** during the load window:
   ```bash
   PROM=http://prometheus:9090 CHAIN=bulletin-summit scripts/storage-bench/collect-metrics.sh gp3-24s
   ```

4. **Verdict** - compare each arm x block-time against the budgets in the plan doc. PASS on
   gp3/SATA -> config change (update hardware rec). FAIL -> the per-column split is justified.

Repeat for arms: nvme (baseline), gp3, sata (+ optional hdd for the blob tier) x block-time
(24 s, 2 s).
