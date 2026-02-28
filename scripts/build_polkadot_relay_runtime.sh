#!/usr/bin/env bash
set -euo pipefail

# Build the Polkadot relay chain runtime with fast-runtime feature from
# polkadot-fellows/runtimes.
#
# The production polkadot runtime was removed from the polkadot binary
# (polkadot-stable2412+), so we build it from the Fellows repo. The
# fast-runtime feature reduces epoch/session lengths from 4 hours to ~60
# seconds, which is essential for local parachain testing (parachains need
# a session change before they start producing blocks).
#
# Usage:
#   ./scripts/build_polkadot_relay_runtime.sh
#
# Environment variables:
#   FELLOWS_VERSION   - Git tag to build (default: v2.0.7)
#   CACHE_DIR         - Clone/build cache directory (default: .cache/fellows-runtimes)
#
# Output:
#   ./examples/production-runtimes/polkadot_runtime.compact.compressed.wasm

FELLOWS_VERSION="${FELLOWS_VERSION:-v2.0.7}"
CACHE_DIR="${CACHE_DIR:-$(pwd)/.cache/fellows-runtimes}"
REPO_URL="https://github.com/polkadot-fellows/runtimes.git"
WASM_FILENAME="polkadot_runtime.compact.compressed.wasm"
OUTPUT_DIR="./examples/production-runtimes"
OUTPUT_PATH="${OUTPUT_DIR}/${WASM_FILENAME}"

# Check if WASM already exists (cached)
if [ -f "${OUTPUT_PATH}" ]; then
    echo "Polkadot relay runtime WASM already exists: ${OUTPUT_PATH}"
    echo "  Size: $(wc -c < "${OUTPUT_PATH}" | tr -d ' ') bytes"
    echo "  Delete it to force a rebuild."
    exit 0
fi

echo "Building Polkadot relay runtime (fast-runtime) from Fellows ${FELLOWS_VERSION}"
echo "  Output: ${OUTPUT_PATH}"
echo "  Cache:  ${CACHE_DIR}"
echo ""

# Check prerequisites
if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo not found. Install Rust: https://rustup.rs" >&2
    exit 1
fi

if ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
    echo "Adding wasm32-unknown-unknown target..."
    rustup target add wasm32-unknown-unknown
fi

# Clone or use cached repo
REPO_DIR="${CACHE_DIR}/${FELLOWS_VERSION}"
if [ -d "${REPO_DIR}/.git" ]; then
    echo "Using cached clone at ${REPO_DIR}"
    cd "${REPO_DIR}"
    CURRENT_TAG="$(git describe --tags --exact-match 2>/dev/null || echo 'none')"
    if [ "${CURRENT_TAG}" != "${FELLOWS_VERSION}" ]; then
        echo "  Checking out tag ${FELLOWS_VERSION}..."
        git fetch --tags
        git checkout "${FELLOWS_VERSION}"
    fi
else
    echo "Cloning ${REPO_URL} at tag ${FELLOWS_VERSION}..."
    mkdir -p "${CACHE_DIR}"
    git clone --depth 1 --branch "${FELLOWS_VERSION}" "${REPO_URL}" "${REPO_DIR}"
    cd "${REPO_DIR}"
fi

echo "  At commit: $(git rev-parse --short HEAD)"
echo ""

# Build polkadot-runtime with fast-runtime feature
echo "Building polkadot-runtime with fast-runtime feature..."
echo "  This may take 10-30 minutes on first build."
cargo build --release -p polkadot-runtime --features fast-runtime

# Copy WASM to output directory
WBUILD_PATH="target/release/wbuild/polkadot-runtime/${WASM_FILENAME}"
if [ ! -f "${WBUILD_PATH}" ]; then
    echo "Error: Expected WASM not found: ${WBUILD_PATH}" >&2
    exit 1
fi

# Return to original directory and copy
cd - > /dev/null
mkdir -p "${OUTPUT_DIR}"
cp "${REPO_DIR}/${WBUILD_PATH}" "${OUTPUT_PATH}"

echo ""
echo "Polkadot relay runtime built successfully!"
echo "  ${OUTPUT_PATH} ($(wc -c < "${OUTPUT_PATH}" | tr -d ' ') bytes)"
