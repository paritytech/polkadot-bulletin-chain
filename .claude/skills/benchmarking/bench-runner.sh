#!/usr/bin/env bash
# Reusable runtime-benchmark runner.
#
# Designed to be uploaded to a (remote) machine and launched detached
# (nohup ... &) by the remote-bench skill, but can also run locally.
# Iterates over pallets, runs frame-omni-bencher per pallet, writes weight
# files to the runtime's src/weights/ directory, and records progress.
#
# Required environment variables:
#   BENCH_PROJECT_DIR        absolute path to the project repo
#   BENCH_RUNTIME_PACKAGE    cargo package name (e.g. bulletin-paseo-runtime)
#   BENCH_RUNTIME_PATH       relative runtime path (e.g. runtimes/bulletin-paseo)
#
# Optional (defaults shown):
#   BENCH_BINARY             frame-omni-bencher (looked up via PATH; can be absolute)
#   BENCH_PROFILE            production
#   BENCH_STEPS              50
#   BENCH_REPEAT             20
#   BENCH_HEADER             ./scripts/cmd/file_header.txt
#   BENCH_XCM_TEMPLATE       templates/xcm-bench-template.hbs
#   BENCH_PALLETS            newline-separated list; if unset, auto-detect via --list
#   BENCH_LOG_DIR            /tmp
#   BENCH_TAG                $BENCH_RUNTIME_PACKAGE   (used to prefix log/status/done files)
#
# Output files (in $BENCH_LOG_DIR):
#   $BENCH_TAG.log       full bencher stdout/stderr per pallet
#   $BENCH_TAG.status    one line per pallet: timestamp, [N/total], OK|FAIL, name
#   $BENCH_TAG.done      created (touched) when the run finishes (success or fail)

set -uo pipefail

: "${BENCH_PROJECT_DIR:?must be set (absolute path to repo)}"
: "${BENCH_RUNTIME_PACKAGE:?must be set (e.g. bulletin-paseo-runtime)}"
: "${BENCH_RUNTIME_PATH:?must be set (e.g. runtimes/bulletin-paseo)}"

BENCH_BINARY="${BENCH_BINARY:-frame-omni-bencher}"
BENCH_PROFILE="${BENCH_PROFILE:-production}"
BENCH_STEPS="${BENCH_STEPS:-50}"
BENCH_REPEAT="${BENCH_REPEAT:-20}"
BENCH_HEADER="${BENCH_HEADER:-./scripts/cmd/file_header.txt}"
BENCH_XCM_TEMPLATE="${BENCH_XCM_TEMPLATE:-templates/xcm-bench-template.hbs}"
BENCH_PALLETS="${BENCH_PALLETS:-}"
BENCH_LOG_DIR="${BENCH_LOG_DIR:-/tmp}"
BENCH_TAG="${BENCH_TAG:-$BENCH_RUNTIME_PACKAGE}"

export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"

cd "$BENCH_PROJECT_DIR"

PKG_UNDERSCORED="${BENCH_RUNTIME_PACKAGE//-/_}"
RUNTIME_WASM="target/$BENCH_PROFILE/wbuild/$BENCH_RUNTIME_PACKAGE/$PKG_UNDERSCORED.wasm"

DEFAULT_OUT="./$BENCH_RUNTIME_PATH/src/weights"
XCM_OUT="./$BENCH_RUNTIME_PATH/src/weights/xcm"

LOG="$BENCH_LOG_DIR/$BENCH_TAG.log"
STATUS="$BENCH_LOG_DIR/$BENCH_TAG.status"
DONE_MARKER="$BENCH_LOG_DIR/$BENCH_TAG.done"

mkdir -p "$BENCH_LOG_DIR"
: > "$LOG"
: > "$STATUS"
rm -f "$DONE_MARKER"

TS() { date -u '+%Y-%m-%dT%H:%M:%SZ'; }

if [[ ! -f "$RUNTIME_WASM" ]]; then
  echo "$(TS) ERROR: wasm not found at $RUNTIME_WASM. Build first with:" | tee -a "$LOG" "$STATUS"
  echo "  cargo build --profile $BENCH_PROFILE -p $BENCH_RUNTIME_PACKAGE --features runtime-benchmarks" | tee -a "$LOG" "$STATUS"
  touch "$DONE_MARKER"
  exit 1
fi

if [[ -n "$BENCH_PALLETS" ]]; then
  mapfile -t PALLETS <<< "$BENCH_PALLETS"
else
  echo "$(TS) listing pallets via $BENCH_BINARY --list..." | tee -a "$LOG" "$STATUS"
  mapfile -t PALLETS < <("$BENCH_BINARY" v1 benchmark pallet --no-csv-header --all --list --runtime="$RUNTIME_WASM" 2>/dev/null | awk -F, '{print $1}' | sort -u | grep -v '^$')
fi

if [[ ${#PALLETS[@]} -eq 0 ]]; then
  echo "$(TS) ERROR: no pallets to bench (was the wasm built with --features runtime-benchmarks?)" | tee -a "$LOG" "$STATUS"
  touch "$DONE_MARKER"
  exit 1
fi

mkdir -p "$DEFAULT_OUT" "$XCM_OUT"

echo "$(TS) starting bench: ${#PALLETS[@]} pallets | runtime=$BENCH_RUNTIME_PACKAGE | profile=$BENCH_PROFILE | steps=$BENCH_STEPS | repeat=$BENCH_REPEAT" | tee -a "$LOG" "$STATUS"

SUCCESS=()
FAILED=()

for i in "${!PALLETS[@]}"; do
  P="${PALLETS[$i]}"
  IDX=$((i+1))
  echo "$(TS) [$IDX/${#PALLETS[@]}] start $P" | tee -a "$LOG" "$STATUS"

  EXTRA=()
  if [[ "$P" == pallet_xcm_benchmarks::* ]]; then
    OUT="$XCM_OUT"
    [[ -n "$BENCH_XCM_TEMPLATE" ]] && EXTRA+=( "--template=$BENCH_XCM_TEMPLATE" )
  else
    OUT="$DEFAULT_OUT"
  fi

  if "$BENCH_BINARY" v1 benchmark pallet \
    --extrinsic='*' \
    --runtime="$RUNTIME_WASM" \
    --pallet="$P" \
    --header="$BENCH_HEADER" \
    --output="$OUT" \
    --wasm-execution=compiled \
    --steps="$BENCH_STEPS" \
    --repeat="$BENCH_REPEAT" \
    --heap-pages=4096 \
    --no-storage-info \
    --no-min-squares \
    --no-median-slopes \
    "${EXTRA[@]}" >>"$LOG" 2>&1; then
    SUCCESS+=("$P")
    echo "$(TS) [$IDX/${#PALLETS[@]}] OK   $P" | tee -a "$STATUS"
  else
    FAILED+=("$P")
    echo "$(TS) [$IDX/${#PALLETS[@]}] FAIL $P" | tee -a "$STATUS"
  fi
done

echo "$(TS) finished. success=${#SUCCESS[@]} failed=${#FAILED[@]}" | tee -a "$LOG" "$STATUS"
echo "success: ${SUCCESS[*]:-(none)}" | tee -a "$STATUS"
echo "failed:  ${FAILED[*]:-(none)}" | tee -a "$STATUS"

touch "$DONE_MARKER"
