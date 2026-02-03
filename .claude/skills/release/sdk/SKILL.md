---
name: release-sdk
description: Release Bulletin SDK (Rust + TypeScript) to crates.io and npm
argument-hint: "<version> [--dry-run]"
disable-model-invocation: true
user-invocable: true
allowed-tools: Bash, Read, Edit, Glob, Grep
---

# Release Bulletin SDK v$ARGUMENTS

You are releasing the Bulletin SDK. Follow these steps precisely.

## Step 1: Parse Arguments

Extract from `$ARGUMENTS`:
- `version`: Required semver (e.g., `0.2.0`)
- `--dry-run`: If present, simulate only - do NOT commit, tag, or push

Validate version matches pattern: `X.Y.Z` or `X.Y.Z-prerelease`

## Step 2: Pre-flight Checks

Run these checks and report status:

```bash
# Check git status
git status --porcelain

# Check current branch
git branch --show-current

# Check if tag already exists
git tag -l "sdk-v<version>"
```

Read current versions:
- `sdk/rust/Cargo.toml` - find `version = "X.Y.Z"`
- `sdk/typescript/package.json` - find `"version": "X.Y.Z"`

**Report to user:**
- Current Rust version: X.Y.Z
- Current TypeScript version: X.Y.Z
- Target version: <version>
- Git branch: <branch>
- Working directory: clean/dirty
- Tag sdk-v<version>: available/exists

**Stop if:**
- Tag already exists
- Versions don't match each other (warn user, ask to continue)

## Step 3: Bump Versions

Edit both files to set new version:

**sdk/rust/Cargo.toml:**
```
version = "<version>"
```

**sdk/typescript/package.json:**
```
"version": "<version>"
```

Then update lockfile:
```bash
cd sdk/typescript && npm install --package-lock-only
```

## Step 4: Run Tests

```bash
# Rust SDK
cd sdk/rust && cargo test --all-features
cd sdk/rust && cargo clippy --all-targets --all-features -- -D warnings
cd sdk/rust && cargo fmt --all -- --check

# TypeScript SDK
cd sdk/typescript && npm run typecheck
cd sdk/typescript && npm test
```

**If any test fails:** Stop and report the failure. Do not continue.

## Step 5: Commit (skip if --dry-run)

Ask user for confirmation, then:

```bash
git add sdk/rust/Cargo.toml sdk/typescript/package.json sdk/typescript/package-lock.json
git commit -m "chore(sdk): bump version to <version>"
```

## Step 6: Tag (skip if --dry-run)

```bash
git tag sdk-v<version>
```

## Step 7: Push (skip if --dry-run)

Ask user for confirmation before pushing:

```bash
git push origin HEAD
git push origin sdk-v<version>
```

## Step 8: Report Success

```
✅ SDK v<version> release initiated!

Next steps:
1. Monitor CI: https://github.com/paritytech/polkadot-bulletin-chain/actions
2. After CI completes, verify:
   - crates.io: https://crates.io/crates/bulletin-sdk-rust
   - npm: https://www.npmjs.com/package/@bulletin/sdk
   - GitHub: https://github.com/paritytech/polkadot-bulletin-chain/releases/tag/sdk-v<version>
```

## Dry Run Mode

If `--dry-run` was specified:
- Complete steps 1-4 (checks, bump, test)
- Show what WOULD be committed and tagged
- Do NOT actually commit, tag, or push
- Offer to trigger GitHub Actions dry-run:
  ```bash
  gh workflow run release-sdk.yml -f version=<version> -f dry_run=true
  ```

## Error Recovery

If something fails mid-release:

```bash
# Undo uncommitted changes
git checkout -- sdk/rust/Cargo.toml sdk/typescript/package.json sdk/typescript/package-lock.json

# Delete local tag (if created)
git tag -d sdk-v<version>

# Delete remote tag (if pushed)
git push origin :refs/tags/sdk-v<version>
```
