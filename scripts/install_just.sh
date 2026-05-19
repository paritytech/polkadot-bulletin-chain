#!/usr/bin/env bash
# Install `just` from $JUST_VERSION (read from .github/env). Sha256-verified against
# $JUST_<PLATFORM>_SHA256 when set.
# Usage: scripts/install_just.sh [install_dir]   (default: /usr/local/bin)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL_DIR="${1:-/usr/local/bin}"

# In CI `.github/env` is sourced into $GITHUB_ENV before this runs. Locally, source it on demand
# so callers don't have to.
if [ -z "${JUST_VERSION:-}" ] && [ -f "$REPO_ROOT/.github/env" ]; then
	set -a
	# shellcheck disable=SC1091
	. "$REPO_ROOT/.github/env"
	set +a
fi
[ -n "${JUST_VERSION:-}" ] || { echo "JUST_VERSION not set" >&2; exit 1; }

case "$(uname -s)-$(uname -m)" in
	Linux-x86_64)  TARGET=x86_64-unknown-linux-musl ;;
	Linux-aarch64) TARGET=aarch64-unknown-linux-musl ;;
	Darwin-arm64)  TARGET=aarch64-apple-darwin ;;
	Darwin-x86_64) TARGET=x86_64-apple-darwin ;;
	*) echo "unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

ASSET="just-${JUST_VERSION}-${TARGET}.tar.gz"
URL="https://github.com/casey/just/releases/download/${JUST_VERSION}/${ASSET}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "[install-just] downloading $URL" >&2
curl -fL --retry 3 --retry-delay 5 -o "$TMP/$ASSET" "$URL"

pin_var="JUST_$(echo "$TARGET" | tr '[:lower:]-' '[:upper:]_')_SHA256"
expected="${!pin_var:-}"
if [ -n "$expected" ]; then
	actual="$(shasum -a 256 "$TMP/$ASSET" | awk '{print $1}')"
	[ "$expected" = "$actual" ] \
		|| { echo "sha256 mismatch for $ASSET (pinned via $pin_var): expected $expected, got $actual" >&2; exit 1; }
	echo "[install-just] sha256 verified" >&2
else
	echo "[install-just] WARNING: $pin_var not set — $ASSET installed UNVERIFIED" >&2
fi

mkdir -p "$INSTALL_DIR"
tar -xzf "$TMP/$ASSET" -C "$INSTALL_DIR" just
chmod +x "$INSTALL_DIR/just"
echo "[install-just] installed $INSTALL_DIR/just" >&2
