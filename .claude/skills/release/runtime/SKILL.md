---
name: release-runtime
description: Release Bulletin Chain runtime (bump spec_version, build WASM, create release)
argument-hint: "<spec_version> [--network polkadot|westend] [--dry-run]"
disable-model-invocation: true
user-invocable: true
allowed-tools: Bash, Read, Edit, Glob, Grep
---

# Release Bulletin Runtime v$ARGUMENTS

You are releasing a Bulletin Chain runtime. Follow these steps precisely.

## Step 1: Parse Arguments

Extract from `$ARGUMENTS`:
- `spec_version`: Required integer (e.g., `1001`)
- `--network`: `polkadot` (default) or `westend`
- `--dry-run`: If present, simulate only

## Step 2: Identify Target Runtime

Based on `--network`:

| Network | Runtime Path | Package |
|---------|--------------|---------|
| polkadot | `runtimes/bulletin-polkadot/` | `bulletin-polkadot-runtime` |
| westend | `runtimes/bulletin-westend/` | `bulletin-westend-runtime` |

## Step 3: Pre-flight Checks

```bash
# Check git status
git status --porcelain

# Check current branch
git branch --show-current

# Check if tag exists
git tag -l "runtime-v<spec_version>"
```

Read current spec_version from runtime's `src/lib.rs`:
```rust
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_version: XXXX,  // Find this value
    ...
};
```

**Report to user:**
- Target runtime: bulletin-<network>-runtime
- Current spec_version: XXXX
- New spec_version: <spec_version>
- Git branch: <branch>
- Tag runtime-v<spec_version>: available/exists

**Stop if:**
- Tag already exists
- New spec_version <= current spec_version

## Step 4: Bump spec_version

Edit `runtimes/bulletin-<network>/src/lib.rs`:

Find the `RuntimeVersion` block and update `spec_version`:
```rust
spec_version: <spec_version>,
```

## Step 5: Build Production Runtime

```bash
cargo build --profile production \
  -p bulletin-<network>-runtime \
  --features on-chain-release-build
```

Verify WASM output exists:
```bash
ls -la target/production/wbuild/bulletin-<network>-runtime/*.wasm
```

## Step 6: Run Tests

```bash
cargo test -p bulletin-<network>-runtime
cargo clippy -p bulletin-<network>-runtime -- -D warnings
```

**If any test fails:** Stop and report. Do not continue.

## Step 7: Commit (skip if --dry-run)

Ask user for confirmation, then:

```bash
git add runtimes/bulletin-<network>/src/lib.rs
git commit -m "chore(runtime): bump bulletin-<network> spec_version to <spec_version>"
```

## Step 8: Tag (skip if --dry-run)

```bash
git tag runtime-<network>-v<spec_version>
```

## Step 9: Push (skip if --dry-run)

Ask user for confirmation:

```bash
git push origin HEAD
git push origin runtime-<network>-v<spec_version>
```

## Step 10: Create GitHub Release (skip if --dry-run)

```bash
gh release create runtime-<network>-v<spec_version> \
  --title "Bulletin <Network> Runtime v<spec_version>" \
  --notes "Runtime upgrade for bulletin-<network>-runtime

spec_version: <spec_version>

## WASM

The production WASM blob is attached below." \
  target/production/wbuild/bulletin-<network>-runtime/*.compact.compressed.wasm
```

## Step 11: Report Success

```
✅ Runtime v<spec_version> released!

Artifacts:
- WASM: target/production/wbuild/bulletin-<network>-runtime/
- Tag: runtime-<network>-v<spec_version>
- Release: https://github.com/paritytech/polkadot-bulletin-chain/releases/tag/runtime-<network>-v<spec_version>

Next steps for on-chain upgrade:
1. Download WASM from GitHub release
2. Submit runtime upgrade proposal via governance
3. Or for solochain: use sudo.setCode()
```

## Dry Run Mode

If `--dry-run`:
- Complete steps 1-6 (checks, bump, build, test)
- Show what WOULD be committed and tagged
- Do NOT commit, tag, push, or create release

## Error Recovery

```bash
# Undo changes
git checkout -- runtimes/bulletin-<network>/src/lib.rs

# Delete local tag
git tag -d runtime-<network>-v<spec_version>

# Delete remote tag (if pushed)
git push origin :refs/tags/runtime-<network>-v<spec_version>

# Delete GitHub release (if created)
gh release delete runtime-<network>-v<spec_version> --yes
```
