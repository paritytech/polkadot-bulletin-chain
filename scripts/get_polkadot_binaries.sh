#!/usr/bin/env bash
# Fetch or build polkadot-sdk-family binaries needed by tests / CI / local dev.
#
# Usage:
#   scripts/get_polkadot_binaries.sh <group>
#
# Groups:
#   polkadot-node        → polkadot, polkadot-prepare-worker, polkadot-execute-worker, polkadot-omni-node
#   frame-omni-bencher   → frame-omni-bencher
#   chain-spec-builder   → chain-spec-builder
#   try-runtime          → try-runtime
#   zombienet            → zombienet  (release-tag only; source-build not supported)
#
# Each group reads `<GROUP>_VERSION` env (uppercase, dashes → underscores):
#   POLKADOT_NODE_VERSION, FRAME_OMNI_BENCHER_VERSION, CHAIN_SPEC_BUILDER_VERSION,
#   TRY_RUNTIME_VERSION, ZOMBIENET_VERSION
#
# Value semantics:
#   * 40-char hex commit hash → clone+build from source.
#   * Anything else           → treat as a release tag; download the prebuilt asset
#                               for the current platform.
#
# Output: prints the absolute path of the directory containing the resolved binaries
# to stdout, so callers can do:
#
#     export PATH="$(scripts/get_polkadot_binaries.sh polkadot-node):$PATH"
#
# All status / log messages go to stderr.
#
# Cache layout:
#   ./.polkadot-binaries/<group>/<ref>/<platform>/<binary>
#   ./.polkadot-binaries/_src/<repo>/                       (one git clone, reused)

set -euo pipefail

log() { printf '[get-binaries] %s\n' "$*" >&2; }
die() { log "error: $*"; exit 1; }

GROUP="${1:-}"
[ -n "$GROUP" ] || die "usage: $0 <group>"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Opt-in shared cache across worktrees: `export BIN_CACHE_DIR=$HOME/.cache/polkadot-bulletin-binaries`.
CACHE_ROOT="${BIN_CACHE_DIR:-$REPO_ROOT/.polkadot-binaries}"
SRC_ROOT="$CACHE_ROOT/_src"

# --- platform detection ------------------------------------------------------
case "$(uname -s)-$(uname -m)" in
	Linux-x86_64)   PLATFORM=linux-x86_64 ;;
	Darwin-arm64)   PLATFORM=darwin-aarch64 ;;
	Darwin-x86_64)  die "macOS Intel is not supported; need Apple Silicon (arm64) or Linux x86_64" ;;
	*)              die "unsupported platform: $(uname -s)-$(uname -m)" ;;
esac

# --- per-group configuration -------------------------------------------------
# Set by configure_group:
#   VERSION_VAR     — name of the env var holding the ref
#   BINARIES        — space-separated list of binary names this group produces
#   RELEASE_REPO    — github "owner/name" for release downloads
#   SOURCE_URL      — git URL for source builds (empty = source-build not supported)
#   SOURCE_DIR      — directory name under SRC_ROOT for the clone (empty if SOURCE_URL empty)
configure_group() {
	case "$1" in
		polkadot-node)
			VERSION_VAR="POLKADOT_NODE_VERSION"
			BINARIES="polkadot polkadot-prepare-worker polkadot-execute-worker polkadot-omni-node"
			RELEASE_REPO="paritytech/polkadot-sdk"
			SOURCE_URL="https://github.com/paritytech/polkadot-sdk.git"
			SOURCE_DIR="polkadot-sdk"
			;;
		frame-omni-bencher)
			VERSION_VAR="FRAME_OMNI_BENCHER_VERSION"
			BINARIES="frame-omni-bencher"
			RELEASE_REPO="paritytech/polkadot-sdk"
			SOURCE_URL="https://github.com/paritytech/polkadot-sdk.git"
			SOURCE_DIR="polkadot-sdk"
			;;
		chain-spec-builder)
			VERSION_VAR="CHAIN_SPEC_BUILDER_VERSION"
			BINARIES="chain-spec-builder"
			RELEASE_REPO="paritytech/polkadot-sdk"
			SOURCE_URL="https://github.com/paritytech/polkadot-sdk.git"
			SOURCE_DIR="polkadot-sdk"
			;;
		try-runtime)
			VERSION_VAR="TRY_RUNTIME_VERSION"
			BINARIES="try-runtime"
			RELEASE_REPO="paritytech/try-runtime-cli"
			SOURCE_URL="https://github.com/paritytech/try-runtime-cli.git"
			SOURCE_DIR="try-runtime-cli"
			;;
		zombienet)
			VERSION_VAR="ZOMBIENET_VERSION"
			BINARIES="zombienet"
			RELEASE_REPO="paritytech/zombienet"
			SOURCE_URL=""
			SOURCE_DIR=""
			;;
		*)
			die "unknown group: $1 (expected one of: polkadot-node, frame-omni-bencher, chain-spec-builder, try-runtime, zombienet)"
			;;
	esac
}

# Build crate per binary for source builds. Echo "crate:binary" pairs.
# Keep this aligned with `BINARIES` order.
build_targets_for_group() {
	case "$1" in
		polkadot-node)
			echo "polkadot:polkadot"
			echo "polkadot:polkadot-prepare-worker"
			echo "polkadot:polkadot-execute-worker"
			echo "polkadot-omni-node:polkadot-omni-node"
			;;
		frame-omni-bencher) echo "frame-omni-bencher:frame-omni-bencher" ;;
		chain-spec-builder) echo "staging-chain-spec-builder:chain-spec-builder" ;;
		try-runtime)        echo "try-runtime:try-runtime" ;;
		*)                  die "no source-build mapping for group $1" ;;
	esac
}

# Map (group, binary, platform) → release asset filename.
release_asset_name() {
	local group="$1" bin="$2"
	case "$group" in
		polkadot-node|frame-omni-bencher|chain-spec-builder)
			case "$PLATFORM" in
				linux-x86_64)   echo "$bin" ;;
				darwin-aarch64) echo "$bin-aarch64-apple-darwin" ;;
			esac
			;;
		try-runtime)
			case "$PLATFORM" in
				linux-x86_64)   echo "try-runtime-x86_64-unknown-linux-musl" ;;
				darwin-aarch64) echo "try-runtime-aarch64-apple-darwin" ;;
			esac
			;;
		zombienet)
			case "$PLATFORM" in
				linux-x86_64)   echo "zombienet-linux-x64" ;;
				darwin-aarch64) echo "zombienet-macos-arm64" ;;
			esac
			;;
	esac
}

configure_group "$GROUP"

REF="${!VERSION_VAR:-}"
[ -n "$REF" ] || die "$VERSION_VAR is unset; cannot resolve $GROUP binaries"

CACHE_DIR="$CACHE_ROOT/$GROUP/$REF/$PLATFORM"

all_present=true
for bin in $BINARIES; do
	if [ ! -x "$CACHE_DIR/$bin" ]; then
		all_present=false
		break
	fi
done

if $all_present; then
	log "$GROUP $REF: cache hit at $CACHE_DIR"
	echo "$CACHE_DIR"
	exit 0
fi

mkdir -p "$CACHE_DIR"

if [[ "$REF" =~ ^[0-9a-fA-F]{40}$ ]]; then
	MODE=source
else
	MODE=release
fi

if [ "$MODE" = release ]; then
	log "$GROUP $REF: downloading release assets for $PLATFORM"
	for bin in $BINARIES; do
		asset="$(release_asset_name "$GROUP" "$bin")"
		url="https://github.com/$RELEASE_REPO/releases/download/$REF/$asset"
		log "  curl $url"
		curl -fL --retry 3 --retry-delay 5 -o "$CACHE_DIR/$bin" "$url" \
			|| die "download failed for $url"
		# Verification order:
		#   1. env-pinned hash (e.g. ZOMBIENET_LINUX_X64_SHA256 in .github/env) — preferred
		#      for binaries whose upstream release doesn't publish a `.sha256` companion.
		#   2. companion `<asset>.sha256` fetched from the same release.
		#   3. (skip + log a warning).
		# (1) wins because the env value comes through a separate channel (PR review).
		pin_var="$(echo "${asset}_SHA256" | tr '[:lower:]-.' '[:upper:]__')"
		expected="${!pin_var:-}"
		if [ -n "$expected" ]; then
			actual="$(shasum -a 256 "$CACHE_DIR/$bin" | awk '{print $1}')"
			[ "$expected" = "$actual" ] \
				|| die "sha256 mismatch for $bin (pinned via $pin_var): expected $expected, got $actual"
			log "  sha256 verified for $bin (pinned via $pin_var)"
		elif curl -fsL --retry 2 -o "$CACHE_DIR/$bin.sha256" "$url.sha256" 2>/dev/null; then
			expected="$(awk '{print $1}' "$CACHE_DIR/$bin.sha256")"
			rm -f "$CACHE_DIR/$bin.sha256"
			if [ -n "$expected" ]; then
				actual="$(shasum -a 256 "$CACHE_DIR/$bin" | awk '{print $1}')"
				[ "$expected" = "$actual" ] \
					|| die "sha256 mismatch for $bin: expected $expected, got $actual"
				log "  sha256 verified for $bin"
			fi
		else
			log "  WARNING: no .sha256 companion published and no pinned $pin_var in env — $bin downloaded UNVERIFIED"
		fi
		chmod +x "$CACHE_DIR/$bin"
	done
else
	[ -n "$SOURCE_URL" ] || die "source-build is not supported for group '$GROUP'; pass a release tag in $VERSION_VAR instead"

	log "$GROUP $REF: building from source"
	mkdir -p "$SRC_ROOT"
	SRC_PATH="$SRC_ROOT/$SOURCE_DIR"
	if [ ! -d "$SRC_PATH/.git" ]; then
		log "  cloning $SOURCE_URL → $SRC_PATH"
		git clone "$SOURCE_URL" "$SRC_PATH" >&2
	fi
	(
		cd "$SRC_PATH"
		# Be permissive: only fetch if the ref isn't present locally yet.
		if ! git cat-file -e "$REF^{commit}" 2>/dev/null; then
			log "  fetching $REF"
			git fetch --quiet origin
		fi
		git -c advice.detachedHead=false checkout --quiet "$REF"
		# macOS: libclang.dylib often isn't found; nudge the linker.
		if [ "$PLATFORM" = darwin-aarch64 ] && command -v brew >/dev/null 2>&1; then
			llvm_prefix="$(brew --prefix llvm 2>/dev/null || true)"
			if [ -n "$llvm_prefix" ] && [ -d "$llvm_prefix/lib" ]; then
				export DYLD_FALLBACK_LIBRARY_PATH="$llvm_prefix/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
			fi
		fi
		# Dedup crates so we don't pass the same `-p` twice (`polkadot-node` group
		# builds polkadot + workers from the same `polkadot` crate).
		targets="$(build_targets_for_group "$GROUP")"
		crates_to_build=""
		while IFS=: read -r crate bin; do
			case " $crates_to_build " in
				*" $crate "*) ;;
				*) crates_to_build="$crates_to_build $crate" ;;
			esac
		done <<< "$targets"
		# Group-specific build flags:
		#   polkadot-node — needs the embedded westend WASM (don't skip it), and
		#       `--features fast-runtime` so zombienet's `westend-development`
		#       preset is available + epochs are short enough for tests.
		#   everything else — no embedded WASM to ship, so SKIP_WASM_BUILD=1 saves
		#       a meaningful chunk of build time.
		case "$GROUP" in
			polkadot-node)
				BUILD_ENV=""
				BUILD_EXTRA_ARGS="--features fast-runtime"
				;;
			*)
				BUILD_ENV="SKIP_WASM_BUILD=1"
				BUILD_EXTRA_ARGS=""
				;;
		esac
		log "  ${BUILD_ENV:+$BUILD_ENV }cargo build --release --locked${crates_to_build// / -p}${BUILD_EXTRA_ARGS:+ $BUILD_EXTRA_ARGS}"
		# shellcheck disable=SC2086
		env $BUILD_ENV cargo build --release --locked $(printf -- '-p %s ' $crates_to_build) $BUILD_EXTRA_ARGS
		while IFS=: read -r crate bin; do
			[ -x "target/release/$bin" ] || die "expected target/release/$bin not produced by cargo"
			cp "target/release/$bin" "$CACHE_DIR/$bin"
			chmod +x "$CACHE_DIR/$bin"
		done <<< "$targets"
	)
fi

for bin in $BINARIES; do
	[ -x "$CACHE_DIR/$bin" ] || die "$bin missing from $CACHE_DIR after fetch"
done

log "$GROUP $REF: ready at $CACHE_DIR"
echo "$CACHE_DIR"
