#!/bin/bash
set -euo pipefail

# Path to your keystore (adjust as needed)
KEYSTORE_DIR=$1

if [ ! -d "$KEYSTORE_DIR" ]; then
  echo "Keystore not found at $KEYSTORE_DIR"
  exit 1
fi

for f in "$KEYSTORE_DIR"/*; do
  # extract key-type from first 8 hex chars of filename
  key_hex=$(basename "$f" | cut -c1-8)
  key_type=$(echo -n "$key_hex" | xxd -r -p)

  # read seed from file (strip quotes)
  seed=$(cat "$f" | tr -d '"')

  echo "Seed: $seed"
  echo "=== $key_type (sr25519)==="
  ./target/release/polkadot-bulletin-chain key inspect --scheme sr25519 "$seed"
  echo "=== $key_type (ed25519)==="
  ./target/release/polkadot-bulletin-chain key inspect --scheme ed25519 "$seed"
  echo
done
