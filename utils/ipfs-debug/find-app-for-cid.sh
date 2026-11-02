#!/usr/bin/env bash
# Find which dotNS app references a (missing) IPFS CID.
#
# Resolves each candidate .dot name to its bundle root via the dotns CLI, then
# walks each bundle on the bulletin gateway for the target CID.
#
# dotNS has no on-chain "list all names", so you pass candidate names (the
# playground/demo apps, or names from an indexer/known owners).
#
# Requires: the `dotns` CLI reachable (default: the published @parity/dotns-cli),
#           python3, and find_cid_parent.py alongside this script.
#
# Usage:
#   ./find-app-for-cid.sh <target-cid> <name> [<name> ...]
# Env overrides:
#   DOTNS="node node_modules/@parity/dotns-cli/dist/cli.js"   # how to invoke the CLI
#   KEY_URI=//Alice                                           # any mapped account (reads are dry-runs)
#   GATEWAY=https://paseo-bulletin-next-ipfs.polkadot.io
set -euo pipefail

TARGET=${1:?usage: find-app-for-cid.sh <target-cid> <name>...}; shift
[ $# -ge 1 ] || { echo "give at least one .dot name"; exit 2; }

DOTNS=${DOTNS:-dotns}
KEY_URI=${KEY_URI:-//Alice}
GATEWAY=${GATEWAY:-https://paseo-bulletin-next-ipfs.polkadot.io}
HERE=$(cd "$(dirname "$0")" && pwd)

roots=()
echo "resolving $# name(s) via dotNS..."
for name in "$@"; do
  name=${name%.dot}
  cid=$(eval "$DOTNS --env paseo-v2 content view \"$name\" --key-uri \"$KEY_URI\"" 2>/dev/null \
        | sed -n 's/.*cid:[[:space:]]*//p' | head -1)
  if [ -n "$cid" ] && [ "$cid" != "(not set)" ]; then
    echo "  $name.dot -> $cid"; roots+=("$cid")
  else
    echo "  $name.dot -> (no content)"
  fi
done

[ ${#roots[@]} -gt 0 ] || { echo "no resolvable roots; nothing to walk"; exit 1; }

echo
echo "walking ${#roots[@]} bundle(s) for $TARGET ..."
python3 "$HERE/find_cid_parent.py" --gateway "$GATEWAY" --target "$TARGET" "${roots[@]}"
