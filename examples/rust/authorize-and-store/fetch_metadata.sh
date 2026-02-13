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

# Fetch metadata (V15 format, supported by subxt 0.44+)
subxt metadata --url "$WS_URL" -f bytes > bulletin_metadata.scale

echo "✅ Metadata saved to bulletin_metadata.scale"
echo "File size: $(wc -c < bulletin_metadata.scale) bytes"
