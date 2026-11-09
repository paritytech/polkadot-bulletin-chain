#!/usr/bin/env bash
# Emulate nearline-HDD seek latency with dm-delay, so the fio pre-screen can answer the HDD
# question. throttle.sh caps IOPS/bandwidth but NOT per-IO latency, and latency is exactly the
# HDD killer for sync-serve (a peer's random body reads each pay a ~7 ms seek). dm-delay adds a
# fixed per-read delay over a backing device; put a filesystem on the result and point TARGET
# there. Reads are delayed; writes are not (sequential blob writes are not seek-bound).
#
# Usage:
#   sudo scripts/storage-bench/hdd-emulate.sh setup /dev/nvme1n1 7   # 7 ms read delay
#   sudo mkfs.ext4 -q /dev/mapper/bulletin-hdd
#   sudo mkdir -p /mnt/hdd && sudo mount /dev/mapper/bulletin-hdd /mnt/hdd
#   sudo TARGET=/mnt/hdd SIZE=1700g fio scripts/storage-bench/fio-block-critical.fio
#   sudo umount /mnt/hdd
#   sudo scripts/storage-bench/hdd-emulate.sh teardown
#
# WARNING: the backing device is used raw - use a scratch disk, its contents are not preserved.
set -euo pipefail

NAME=bulletin-hdd

case "${1:?setup <backing-dev> [read_ms] | teardown}" in
  setup)
    DEV="${2:?backing block device, e.g. /dev/nvme1n1 (scratch - will be overwritten)}"
    READ_MS="${3:-7}"   # ~7 ms ~= a 7200 rpm nearline HDD random seek
    SECTORS=$(blockdev --getsz "$DEV")
    # dm-delay table: <start> <len> delay <read_dev> <read_off> <read_ms> <write_dev> <write_off> <write_ms>
    # write_ms=0 so sequential blob writes are not penalised; only the seek-bound reads are.
    echo "0 $SECTORS delay $DEV 0 $READ_MS $DEV 0 0" | dmsetup create "$NAME"
    echo "created /dev/mapper/$NAME over $DEV: ${READ_MS} ms read delay, 0 ms write delay"
    ;;
  teardown)
    dmsetup remove "$NAME"
    echo "removed /dev/mapper/$NAME"
    ;;
  *)
    echo "usage: $0 setup <backing-dev> [read_ms] | teardown" >&2
    exit 1
    ;;
esac
