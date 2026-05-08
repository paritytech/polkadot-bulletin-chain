#!/usr/bin/env bash
#
# Build the bulletin-polkadot runtime WASM from the Polkadot Fellows
# `runtimes` repo and generate the chain spec used by the local zombienet
# config.
#
# The runtime is not part of this Cargo workspace, so we clone the upstream
# repository at a configurable ref and build it out-of-tree.
#
# Override the source via env vars (defaults track Fellows PR #1170):
#   FELLOWS_RUNTIMES_REPO  - git URL (default: bkontur/runtimes fork)
#   FELLOWS_RUNTIMES_REF   - branch / tag / sha (default: bko-bulletin-stage1)
#   FELLOWS_RUNTIMES_DIR   - local checkout dir (default: target/fellows-runtimes)

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

FELLOWS_RUNTIMES_REPO="${FELLOWS_RUNTIMES_REPO:-https://github.com/bkontur/runtimes.git}"
FELLOWS_RUNTIMES_REF="${FELLOWS_RUNTIMES_REF:-bko-bulletin-stage1}"
FELLOWS_RUNTIMES_DIR="${FELLOWS_RUNTIMES_DIR:-$ROOT_DIR/target/fellows-runtimes}"

mkdir -p "$(dirname "$FELLOWS_RUNTIMES_DIR")"

if [ ! -d "$FELLOWS_RUNTIMES_DIR/.git" ]; then
	echo "📥 Cloning $FELLOWS_RUNTIMES_REPO into $FELLOWS_RUNTIMES_DIR..."
	git clone --filter=blob:none "$FELLOWS_RUNTIMES_REPO" "$FELLOWS_RUNTIMES_DIR"
else
	echo "♻️  Reusing existing checkout at $FELLOWS_RUNTIMES_DIR"
	git -C "$FELLOWS_RUNTIMES_DIR" remote set-url origin "$FELLOWS_RUNTIMES_REPO"
fi

echo "🔀 Fetching ref: $FELLOWS_RUNTIMES_REF..."
git -C "$FELLOWS_RUNTIMES_DIR" fetch --depth 1 origin "$FELLOWS_RUNTIMES_REF"
git -C "$FELLOWS_RUNTIMES_DIR" checkout -q FETCH_HEAD

echo "🔨 Building bulletin-polkadot-runtime..."
(cd "$FELLOWS_RUNTIMES_DIR" && cargo build --release -p bulletin-polkadot-runtime)

WASM_PATH="$FELLOWS_RUNTIMES_DIR/target/release/wbuild/bulletin-polkadot-runtime/bulletin_polkadot_runtime.compact.compressed.wasm"
if [ ! -f "$WASM_PATH" ]; then
	echo "❌ Expected WASM not found at: $WASM_PATH"
	exit 1
fi

cd "$ROOT_DIR"

chain-spec-builder create \
	-p 1010 \
	-c westend \
	-i bulletin-polkadot \
	-n Bulletin \
	-t local \
	-r "$WASM_PATH" \
	named-preset local_testnet

mv chain_spec.json ./zombienet/bulletin-polkadot-spec.json
echo "✅ Wrote ./zombienet/bulletin-polkadot-spec.json"
