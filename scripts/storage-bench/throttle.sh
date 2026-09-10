#!/usr/bin/env bash
# Emulate a standard-SSD tier (gp3 or SATA) on a bare-metal NVMe box by capping block-IO with
# cgroup v2 io.max. Use this only when you cannot attach a real gp3/SATA volume.
#
# Real volumes are always preferred (network latency of gp3 is not reproducible by throttling
# a local NVMe). This gives a conservative upper bound: if the node passes under the throttle,
# it will pass on the real tier.
#
# Usage:
#   sudo scripts/storage-bench/throttle.sh gp3   /sys/fs/cgroup/bulletin-bench <device>
#   sudo scripts/storage-bench/throttle.sh sata  /sys/fs/cgroup/bulletin-bench <device>
#   sudo scripts/storage-bench/throttle.sh hdd   /sys/fs/cgroup/bulletin-bench <device>
#   sudo scripts/storage-bench/throttle.sh clear /sys/fs/cgroup/bulletin-bench <device>
# NB: io.max caps IOPS/bandwidth, NOT per-IO latency. An HDD's real killer is seek latency, so
# the `hdd` cap here is only a random-IOPS proxy - for a faithful HDD run use hdd-emulate.sh
# (dm-delay), which injects per-read latency.
# <device> e.g. /dev/nvme0n1 (the disk backing the node's data volume).
# Then launch the node inside the cgroup: sudo cgexec -g io:bulletin-bench <node cmd>
# (or: echo <node_pid> > $CGROUP/cgroup.procs).
set -euo pipefail

TIER="${1:?tier: gp3|sata|hdd|clear}"
CGROUP="${2:?cgroup path, e.g. /sys/fs/cgroup/bulletin-bench}"
DEV="${3:?block device, e.g. /dev/nvme0n1}"

# Resolve MAJ:MIN of the underlying device.
MAJMIN=$(lsblk -ndo MAJ:MIN "$DEV")

mkdir -p "$CGROUP"
# Ensure io controller is delegated to the cgroup.
if ! grep -qw io "$(dirname "$CGROUP")/cgroup.subtree_control" 2>/dev/null; then
  echo "+io" > "$(dirname "$CGROUP")/cgroup.subtree_control" 2>/dev/null || true
fi

case "$TIER" in
  gp3)
    # gp3 baseline: 3000 IOPS, 125 MB/s. (Bump to 16000 / 1000MB/s for a provisioned gp3.)
    echo "$MAJMIN riops=3000 wiops=3000 rbps=131072000 wbps=131072000" > "$CGROUP/io.max"
    ;;
  sata)
    # Representative SATA SSD ceiling: ~550 MB/s, SATA-limited IOPS.
    echo "$MAJMIN riops=80000 wiops=80000 rbps=576716800 wbps=576716800" > "$CGROUP/io.max"
    ;;
  hdd)
    # Nearline HDD random-IOPS ceiling (~200, seek-bound) + ~200 MB/s sequential. This only
    # models the IOPS wall; use hdd-emulate.sh for the latency that actually breaks sync-serve.
    echo "$MAJMIN riops=200 wiops=200 rbps=209715200 wbps=209715200" > "$CGROUP/io.max"
    ;;
  clear)
    echo "$MAJMIN riops=max wiops=max rbps=max wbps=max" > "$CGROUP/io.max"
    ;;
  *) echo "unknown tier: $TIER" >&2; exit 1 ;;
esac

echo "io.max for $DEV ($MAJMIN) in $CGROUP:"
cat "$CGROUP/io.max"
