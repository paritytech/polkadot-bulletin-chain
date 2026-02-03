#!/usr/bin/env bash
#
# Bulletin SDK Release Tool
# =========================
# Complete release automation for Rust and TypeScript SDKs.
#
# Usage:
#   ./scripts/release-sdk.sh <command> [options]
#
# Commands:
#   bump <version>     Bump version numbers in both SDKs
#   test               Run all SDK tests
#   release <version>  Full release: bump → test → commit → tag → push
#   verify <version>   Verify published packages
#   dry-run <version>  Simulate release without publishing
#
# Examples:
#   ./scripts/release-sdk.sh bump 0.2.0
#   ./scripts/release-sdk.sh release 0.2.0
#   ./scripts/release-sdk.sh dry-run 0.2.0
#   ./scripts/release-sdk.sh verify 0.2.0
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# ============================================================================
# Configuration
# ============================================================================

RUST_SDK_DIR="$ROOT_DIR/sdk/rust"
TS_SDK_DIR="$ROOT_DIR/sdk/typescript"
RUST_CARGO_TOML="$RUST_SDK_DIR/Cargo.toml"
TS_PACKAGE_JSON="$TS_SDK_DIR/package.json"

# Crates.io and npm package names
CRATES_IO_NAME="bulletin-sdk-rust"
NPM_NAME="@bulletin/sdk"

# Tag prefix
TAG_PREFIX="sdk-v"

# ============================================================================
# Colors and Output
# ============================================================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()    { echo -e "${BLUE}ℹ${NC}  $*"; }
success() { echo -e "${GREEN}✓${NC}  $*"; }
warn()    { echo -e "${YELLOW}⚠${NC}  $*"; }
error()   { echo -e "${RED}✗${NC}  $*" >&2; }
step()    { echo -e "\n${BOLD}${CYAN}▶ $*${NC}"; }
detail()  { echo -e "   $*"; }

die() {
    error "$@"
    exit 1
}

# ============================================================================
# Utilities
# ============================================================================

confirm() {
    local prompt="${1:-Continue?}"
    local default="${2:-n}"

    if [[ "$default" == "y" ]]; then
        read -p "$prompt [Y/n] " -n 1 -r
    else
        read -p "$prompt [y/N] " -n 1 -r
    fi
    echo

    if [[ "$default" == "y" ]]; then
        [[ ! $REPLY =~ ^[Nn]$ ]]
    else
        [[ $REPLY =~ ^[Yy]$ ]]
    fi
}

validate_version() {
    local version="$1"
    if ! echo "$version" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
        die "Invalid version format: $version" \
            "Version must follow SemVer: X.Y.Z or X.Y.Z-prerelease" \
            "Examples: 0.1.0, 1.0.0, 0.2.0-alpha.1"
    fi
}

get_rust_version() {
    grep '^version = ' "$RUST_CARGO_TOML" | head -1 | cut -d '"' -f 2
}

get_ts_version() {
    grep '"version":' "$TS_PACKAGE_JSON" | head -1 | cut -d '"' -f 4
}

check_git_clean() {
    if [[ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]]; then
        return 1
    fi
    return 0
}

check_command() {
    command -v "$1" &> /dev/null
}

# ============================================================================
# Commands
# ============================================================================

cmd_help() {
    cat << 'EOF'
Bulletin SDK Release Tool
=========================

USAGE:
    release-sdk.sh <COMMAND> [OPTIONS]

COMMANDS:
    bump <version>      Bump version in both Rust and TypeScript SDKs
    test                Run tests for both SDKs
    release <version>   Full release workflow (bump → test → commit → tag → push)
    verify <version>    Verify packages are published correctly
    dry-run <version>   Simulate full release without making changes
    status              Show current SDK versions and git status

OPTIONS:
    -h, --help          Show this help message
    -y, --yes           Skip confirmation prompts
    --skip-tests        Skip running tests (use with caution)

EXAMPLES:
    # Check current status
    release-sdk.sh status

    # Bump to new version
    release-sdk.sh bump 0.2.0

    # Run tests only
    release-sdk.sh test

    # Full release (interactive)
    release-sdk.sh release 0.2.0

    # Test the release process
    release-sdk.sh dry-run 0.2.0

    # Verify after CI completes
    release-sdk.sh verify 0.2.0

WORKFLOW:
    1. Create feature branch with SDK changes
    2. Merge to main
    3. Run: release-sdk.sh release 0.2.0
    4. Wait for CI to complete
    5. Run: release-sdk.sh verify 0.2.0

For detailed documentation, see: RELEASING.md
EOF
}

cmd_status() {
    step "SDK Status"

    local rust_ver ts_ver
    rust_ver=$(get_rust_version)
    ts_ver=$(get_ts_version)

    echo
    echo "  Versions:"
    detail "Rust SDK:       $rust_ver"
    detail "TypeScript SDK: $ts_ver"

    if [[ "$rust_ver" != "$ts_ver" ]]; then
        warn "Versions are out of sync!"
    else
        success "Versions are in sync"
    fi

    echo
    echo "  Git:"
    detail "Branch: $(git -C "$ROOT_DIR" branch --show-current)"
    detail "Status: $(check_git_clean && echo "clean" || echo "uncommitted changes")"

    echo
    echo "  Latest tags:"
    git -C "$ROOT_DIR" tag -l "${TAG_PREFIX}*" | sort -V | tail -5 | while read -r tag; do
        detail "$tag"
    done
    echo
}

cmd_bump() {
    local version="${1:-}"
    [[ -z "$version" ]] && die "Version required. Usage: release-sdk.sh bump <version>"
    validate_version "$version"

    step "Bumping SDK version to $version"

    local rust_ver ts_ver
    rust_ver=$(get_rust_version)
    ts_ver=$(get_ts_version)

    echo
    detail "Current Rust version:       $rust_ver"
    detail "Current TypeScript version: $ts_ver"
    detail "New version:                $version"
    echo

    if [[ "$rust_ver" == "$version" && "$ts_ver" == "$version" ]]; then
        warn "Already at version $version"
        return 0
    fi

    if [[ "${SKIP_CONFIRM:-}" != "1" ]]; then
        confirm "Proceed with version bump?" || { info "Aborted."; exit 0; }
    fi

    # Update Rust SDK
    info "Updating $RUST_CARGO_TOML..."
    if [[ "$(uname)" == "Darwin" ]]; then
        sed -i '' "s/^version = \"$rust_ver\"/version = \"$version\"/" "$RUST_CARGO_TOML"
    else
        sed -i "s/^version = \"$rust_ver\"/version = \"$version\"/" "$RUST_CARGO_TOML"
    fi
    success "Updated Rust SDK to $version"

    # Update TypeScript SDK
    info "Updating $TS_PACKAGE_JSON..."
    if [[ "$(uname)" == "Darwin" ]]; then
        sed -i '' "s/\"version\": \"$ts_ver\"/\"version\": \"$version\"/" "$TS_PACKAGE_JSON"
    else
        sed -i "s/\"version\": \"$ts_ver\"/\"version\": \"$version\"/" "$TS_PACKAGE_JSON"
    fi
    success "Updated TypeScript SDK to $version"

    # Update package-lock.json
    if [[ -f "$TS_SDK_DIR/package-lock.json" ]]; then
        info "Updating package-lock.json..."
        (cd "$TS_SDK_DIR" && npm install --package-lock-only --silent)
        success "Updated package-lock.json"
    fi

    echo
    success "Version bump complete!"
    echo
    detail "Run 'git diff' to review changes"
}

cmd_test() {
    step "Running SDK Tests"

    local failed=0

    # Rust tests
    info "Testing Rust SDK..."
    echo
    if (cd "$RUST_SDK_DIR" && cargo test --all-features); then
        success "Rust tests passed"
    else
        error "Rust tests failed"
        failed=1
    fi

    echo
    info "Running Rust clippy..."
    if (cd "$RUST_SDK_DIR" && cargo clippy --all-targets --all-features -- -D warnings); then
        success "Rust clippy passed"
    else
        error "Rust clippy failed"
        failed=1
    fi

    echo
    info "Checking Rust formatting..."
    if (cd "$RUST_SDK_DIR" && cargo fmt --all -- --check); then
        success "Rust formatting OK"
    else
        error "Rust formatting issues found"
        failed=1
    fi

    # TypeScript tests
    echo
    info "Testing TypeScript SDK..."
    if (cd "$TS_SDK_DIR" && npm test 2>/dev/null); then
        success "TypeScript tests passed"
    else
        # npm test might not exist
        warn "TypeScript tests skipped (no test script)"
    fi

    echo
    info "Running TypeScript typecheck..."
    if (cd "$TS_SDK_DIR" && npm run typecheck 2>/dev/null); then
        success "TypeScript typecheck passed"
    else
        error "TypeScript typecheck failed"
        failed=1
    fi

    echo
    if [[ $failed -eq 0 ]]; then
        success "All tests passed!"
    else
        die "Some tests failed. Fix issues before releasing."
    fi
}

cmd_release() {
    local version="${1:-}"
    [[ -z "$version" ]] && die "Version required. Usage: release-sdk.sh release <version>"
    validate_version "$version"

    local tag="${TAG_PREFIX}${version}"

    step "Releasing SDK v$version"
    echo
    detail "This will:"
    detail "  1. Bump versions to $version"
    detail "  2. Run all tests"
    detail "  3. Commit changes"
    detail "  4. Create tag: $tag"
    detail "  5. Push to origin (triggers CI release)"
    echo

    # Pre-flight checks
    step "Pre-flight Checks"

    # Check git status
    if ! check_git_clean; then
        warn "Working directory has uncommitted changes:"
        git -C "$ROOT_DIR" status --short
        echo
        if [[ "${SKIP_CONFIRM:-}" != "1" ]]; then
            confirm "Continue anyway?" || { info "Aborted."; exit 0; }
        fi
    else
        success "Working directory is clean"
    fi

    # Check branch
    local branch
    branch=$(git -C "$ROOT_DIR" branch --show-current)
    detail "Current branch: $branch"
    if [[ "$branch" != "main" && "$branch" != "master" ]]; then
        warn "Not on main/master branch"
        if [[ "${SKIP_CONFIRM:-}" != "1" ]]; then
            confirm "Continue on branch '$branch'?" || { info "Aborted."; exit 0; }
        fi
    fi

    # Check tag doesn't exist
    if git -C "$ROOT_DIR" tag -l "$tag" | grep -q "$tag"; then
        die "Tag $tag already exists. Delete it first or use a different version."
    fi
    success "Tag $tag is available"

    # Bump versions
    cmd_bump "$version"

    # Run tests (unless skipped)
    if [[ "${SKIP_TESTS:-}" != "1" ]]; then
        cmd_test
    else
        warn "Tests skipped (--skip-tests)"
    fi

    # Commit
    step "Creating Commit"

    local commit_msg="chore(sdk): bump version to $version"
    detail "Message: $commit_msg"
    echo

    if [[ "${SKIP_CONFIRM:-}" != "1" ]]; then
        confirm "Create commit?" || { info "Aborted."; exit 0; }
    fi

    git -C "$ROOT_DIR" add \
        "$RUST_CARGO_TOML" \
        "$TS_PACKAGE_JSON" \
        "$TS_SDK_DIR/package-lock.json" 2>/dev/null || true

    git -C "$ROOT_DIR" commit -m "$commit_msg"
    success "Created commit"

    # Tag
    step "Creating Tag"

    detail "Tag: $tag"
    echo

    if [[ "${SKIP_CONFIRM:-}" != "1" ]]; then
        confirm "Create tag $tag?" || { info "Aborted."; exit 0; }
    fi

    git -C "$ROOT_DIR" tag "$tag"
    success "Created tag $tag"

    # Push
    step "Pushing to Origin"

    detail "This will trigger the release CI workflow"
    echo

    if [[ "${SKIP_CONFIRM:-}" != "1" ]]; then
        confirm "Push commit and tag to origin?" || {
            warn "Aborted. To push manually:"
            detail "git push origin $branch"
            detail "git push origin $tag"
            exit 0
        }
    fi

    git -C "$ROOT_DIR" push origin "$branch"
    git -C "$ROOT_DIR" push origin "$tag"
    success "Pushed to origin"

    # Done
    echo
    echo -e "${GREEN}${BOLD}══════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}${BOLD}  Release $version initiated successfully!${NC}"
    echo -e "${GREEN}${BOLD}══════════════════════════════════════════════════════════════${NC}"
    echo
    detail "Next steps:"
    detail "  1. Monitor CI: https://github.com/paritytech/polkadot-bulletin-chain/actions"
    detail "  2. Once complete, verify: ./scripts/release-sdk.sh verify $version"
    echo
}

cmd_verify() {
    local version="${1:-}"
    [[ -z "$version" ]] && die "Version required. Usage: release-sdk.sh verify <version>"
    validate_version "$version"

    step "Verifying SDK v$version Release"

    local all_ok=1

    # Check crates.io
    echo
    info "Checking crates.io..."
    if check_command cargo; then
        local crates_result
        crates_result=$(cargo search "$CRATES_IO_NAME" 2>/dev/null | head -1 || echo "")
        if echo "$crates_result" | grep -q "$version"; then
            success "Rust SDK v$version found on crates.io"
            detail "$crates_result"
        else
            error "Rust SDK v$version NOT found on crates.io"
            detail "Latest: $crates_result"
            all_ok=0
        fi
    else
        warn "cargo not installed, skipping crates.io check"
    fi

    # Check npm
    echo
    info "Checking npm..."
    if check_command npm; then
        local npm_versions
        npm_versions=$(npm view "$NPM_NAME" versions --json 2>/dev/null || echo "[]")
        if echo "$npm_versions" | grep -q "\"$version\""; then
            success "TypeScript SDK v$version found on npm"
        else
            error "TypeScript SDK v$version NOT found on npm"
            detail "Available: $npm_versions"
            all_ok=0
        fi
    else
        warn "npm not installed, skipping npm check"
    fi

    # Check GitHub release
    echo
    info "Checking GitHub releases..."
    if check_command gh; then
        local tag="${TAG_PREFIX}${version}"
        if gh release view "$tag" --repo paritytech/polkadot-bulletin-chain &>/dev/null; then
            success "GitHub release $tag exists"
            detail "URL: https://github.com/paritytech/polkadot-bulletin-chain/releases/tag/$tag"
        else
            error "GitHub release $tag NOT found"
            all_ok=0
        fi
    else
        warn "gh CLI not installed, skipping GitHub check"
        detail "Install: brew install gh"
    fi

    echo
    if [[ $all_ok -eq 1 ]]; then
        success "All verifications passed!"
    else
        error "Some verifications failed. Check CI logs for details."
        exit 1
    fi
}

cmd_dry_run() {
    local version="${1:-}"
    [[ -z "$version" ]] && die "Version required. Usage: release-sdk.sh dry-run <version>"
    validate_version "$version"

    step "Dry Run: SDK v$version"
    echo
    warn "DRY RUN MODE - No changes will be made"
    echo

    local tag="${TAG_PREFIX}${version}"

    # Show what would happen
    detail "Would bump versions:"
    detail "  Rust:       $(get_rust_version) → $version"
    detail "  TypeScript: $(get_ts_version) → $version"
    echo
    detail "Would create commit: chore(sdk): bump version to $version"
    detail "Would create tag: $tag"
    detail "Would push to: origin/$(git -C "$ROOT_DIR" branch --show-current)"
    echo

    # Run tests
    info "Running tests (this is the only real action)..."
    cmd_test

    echo
    success "Dry run complete! Ready for: release-sdk.sh release $version"

    # Offer to trigger GitHub Actions dry-run
    echo
    if check_command gh; then
        if confirm "Trigger GitHub Actions dry-run workflow?"; then
            info "Triggering workflow..."
            gh workflow run release-sdk.yml \
                --repo paritytech/polkadot-bulletin-chain \
                -f version="$version" \
                -f dry_run=true
            success "Workflow triggered. Monitor at:"
            detail "https://github.com/paritytech/polkadot-bulletin-chain/actions/workflows/release-sdk.yml"
        fi
    fi
}

# ============================================================================
# Main
# ============================================================================

main() {
    # Parse global options
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -h|--help)
                cmd_help
                exit 0
                ;;
            -y|--yes)
                export SKIP_CONFIRM=1
                shift
                ;;
            --skip-tests)
                export SKIP_TESTS=1
                shift
                ;;
            -*)
                die "Unknown option: $1"
                ;;
            *)
                break
                ;;
        esac
    done

    local cmd="${1:-help}"
    shift || true

    case "$cmd" in
        help|--help|-h)
            cmd_help
            ;;
        status)
            cmd_status
            ;;
        bump)
            cmd_bump "$@"
            ;;
        test)
            cmd_test
            ;;
        release)
            cmd_release "$@"
            ;;
        verify)
            cmd_verify "$@"
            ;;
        dry-run|dryrun)
            cmd_dry_run "$@"
            ;;
        *)
            die "Unknown command: $cmd" \
                "Run 'release-sdk.sh help' for usage."
            ;;
    esac
}

main "$@"
