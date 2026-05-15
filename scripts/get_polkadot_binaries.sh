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
#   relay-runtime        → westend_runtime.compact.compressed.wasm (source-build only)
#
# Each group reads `<GROUP>_VERSION` env (uppercase, dashes → underscores):
#   POLKADOT_NODE_VERSION, FRAME_OMNI_BENCHER_VERSION, CHAIN_SPEC_BUILDER_VERSION,
#   TRY_RUNTIME_VERSION, ZOMBIENET_VERSION, RELAY_RUNTIME_VERSION
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
# Cache layout (rooted at $POLKADOT_BINARIES_DIR, default `./.polkadot-binaries`):
#   <root>/<group>/<ref>/<platform>/<binary>
#   <root>/_src/<repo>.git/                   bare clone, objects shared across worktrees
#   <root>/_src/worktrees/<ref>/              per-SHA worktree checkout
#   <root>/_src/target/<ref>/                 per-SHA cargo target dir (avoids
#                                             invalidation when SHAs alternate)

set -euo pipefail

log() { printf '[get-binaries] %s\n' "$*" >&2; }
die() { log "error: $*"; exit 1; }

GROUP="${1:-}"
[ -n "$GROUP" ] || die "usage: $0 <group>"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Target directory for downloaded / built binaries. Default lives in `.github/env`;
# override per-shell to share across worktrees, e.g.
#   export POLKADOT_BINARIES_DIR=$HOME/.cache/polkadot-bulletin-binaries
CACHE_ROOT="${POLKADOT_BINARIES_DIR:-$REPO_ROOT/.polkadot-binaries}"
# Resolve to absolute: the source-build path `cd`s into the cloned repo before `cp`-ing
# binaries to $CACHE_ROOT/<group>/<ref>/<platform>, so a relative path breaks.
mkdir -p "$CACHE_ROOT"
CACHE_ROOT="$(cd "$CACHE_ROOT" && pwd)"
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
		relay-runtime)
			VERSION_VAR="RELAY_RUNTIME_VERSION"
			BINARIES="westend_runtime.compact.compressed.wasm"
			RELEASE_REPO=""
			SOURCE_URL="https://github.com/paritytech/polkadot-sdk.git"
			SOURCE_DIR="polkadot-sdk"
			;;
		*)
			die "unknown group: $1 (expected one of: polkadot-node, frame-omni-bencher, chain-spec-builder, try-runtime, zombienet, relay-runtime)"
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

# WASM artifacts (relay-runtime) aren't executable — check existence only.
case "$GROUP" in
	relay-runtime) cache_check_op="-f" ;;
	*)             cache_check_op="-x" ;;
esac
all_present=true
for bin in $BINARIES; do
	if [ ! $cache_check_op "$CACHE_DIR/$bin" ]; then
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
		# sha256 verification: env-pinned hash > companion `.sha256` > skip+warn. The
		# env-pinned form wins because it flows through PR review.
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
	mkdir -p "$SRC_ROOT/worktrees" "$SRC_ROOT/target"
	BARE_PATH="$SRC_ROOT/$SOURCE_DIR.git"
	if [ ! -d "$BARE_PATH" ]; then
		log "  cloning $SOURCE_URL → $BARE_PATH (bare)"
		git clone --bare "$SOURCE_URL" "$BARE_PATH" >&2
	fi
	# Permissive fetch: only when ref isn't reachable in the bare repo.
	if ! git --git-dir="$BARE_PATH" cat-file -e "$REF^{commit}" 2>/dev/null; then
		log "  fetching $REF into $BARE_PATH"
		git --git-dir="$BARE_PATH" fetch --quiet origin
	fi
	# GC any worktrees the user deleted by hand (e.g. via `rm -rf`).
	git --git-dir="$BARE_PATH" worktree prune
	WORKTREE_PATH="$SRC_ROOT/worktrees/$REF"
	if [ ! -d "$WORKTREE_PATH" ]; then
		log "  adding worktree at $WORKTREE_PATH"
		git --git-dir="$BARE_PATH" -c advice.detachedHead=false \
			worktree add --detach --quiet "$WORKTREE_PATH" "$REF"
	fi
	TARGET_DIR="$SRC_ROOT/target/$REF"
	mkdir -p "$TARGET_DIR"
	(
		cd "$WORKTREE_PATH"
		# Per-SHA target dir so divergent groups (e.g. relay-runtime at a newer SHA
		# than polkadot-node) don't trash each other's incremental cache.
		export CARGO_TARGET_DIR="$TARGET_DIR"
		# macOS: libclang.dylib often isn't found; nudge the linker.
		if [ "$PLATFORM" = darwin-aarch64 ] && command -v brew >/dev/null 2>&1; then
			llvm_prefix="$(brew --prefix llvm 2>/dev/null || true)"
			if [ -n "$llvm_prefix" ] && [ -d "$llvm_prefix/lib" ]; then
				export DYLD_FALLBACK_LIBRARY_PATH="$llvm_prefix/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
			fi
		fi
		if [ "$GROUP" = "polkadot-node" ]; then
			# The relay runtime is supplied via the `relay-runtime` group + chain-spec,
			# so the node doesn't need its native runtime — skip the wasm-builder.
			log "  SKIP_WASM_BUILD=1 cargo build --release --locked -p polkadot"
			SKIP_WASM_BUILD=1 cargo build --release --locked -p polkadot
			log "  SKIP_WASM_BUILD=1 cargo build --release --locked -p polkadot-omni-node"
			SKIP_WASM_BUILD=1 cargo build --release --locked -p polkadot-omni-node
			while IFS=: read -r crate bin; do
				[ -x "$CARGO_TARGET_DIR/release/$bin" ] || die "expected $CARGO_TARGET_DIR/release/$bin not produced by cargo"
				cp "$CARGO_TARGET_DIR/release/$bin" "$CACHE_DIR/$bin"
				chmod +x "$CACHE_DIR/$bin"
			done <<< "$(build_targets_for_group "$GROUP")"
		elif [ "$GROUP" = "relay-runtime" ]; then
			# Drives the substrate-wasm-builder via cargo build of the runtime crate.
			# `--features fast-runtime` cuts epoch / session durations to test-friendly
			# values, which is the whole reason we build it ourselves.
			log "  cargo build --release --locked -p westend-runtime --features fast-runtime"
			cargo build --release --locked -p westend-runtime --features fast-runtime
			wasm_src="$CARGO_TARGET_DIR/release/wbuild/westend-runtime/westend_runtime.compact.compressed.wasm"
			[ -f "$wasm_src" ] || die "expected $wasm_src not produced by cargo"
			cp "$wasm_src" "$CACHE_DIR/westend_runtime.compact.compressed.wasm"
		else
			targets="$(build_targets_for_group "$GROUP")"
			crates_to_build=""
			while IFS=: read -r crate bin; do
				case " $crates_to_build " in
					*" $crate "*) ;;
					*) crates_to_build="$crates_to_build $crate" ;;
				esac
			done <<< "$targets"
			log "  SKIP_WASM_BUILD=1 cargo build --release --locked${crates_to_build// / -p}"
			# shellcheck disable=SC2086
			SKIP_WASM_BUILD=1 cargo build --release --locked $(printf -- '-p %s ' $crates_to_build)
			while IFS=: read -r crate bin; do
				[ -x "$CARGO_TARGET_DIR/release/$bin" ] || die "expected $CARGO_TARGET_DIR/release/$bin not produced by cargo"
				cp "$CARGO_TARGET_DIR/release/$bin" "$CACHE_DIR/$bin"
				chmod +x "$CACHE_DIR/$bin"
			done <<< "$targets"
		fi
	)
fi

for bin in $BINARIES; do
	[ $cache_check_op "$CACHE_DIR/$bin" ] || die "$bin missing from $CACHE_DIR after fetch"
done

log "$GROUP $REF: ready at $CACHE_DIR"
echo "$CACHE_DIR"
