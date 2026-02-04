#!/usr/bin/env bash
# Fetch metadata from a running Bulletin Chain node
# Usage: ./fetch_metadata.sh [ws_url]

set -e

WS_URL="${1:-ws://localhost:10000}"

echo "Fetching metadata from $WS_URL..."

# Check if subxt CLI is installed
if ! command -v subxt &> /dev/null; then
    echo "Error: subxt CLI not found"
    echo "Install it with: cargo install subxt-cli"
    exit 1
fi

# Fetch metadata (request V14 format for subxt 0.37 compatibility)
# Modern Polkadot SDK nodes support serving multiple metadata versions
subxt metadata --url "$WS_URL" --version 14 -f bytes > bulletin_metadata.scale

echo "✅ Metadata saved to bulletin_metadata.scale"
echo "File size: $(wc -c < bulletin_metadata.scale) bytes"
