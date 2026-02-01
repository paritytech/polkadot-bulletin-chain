---
name: release-runtime
description: Release Bulletin Chain runtime to Westend/Paseo testnets
argument-hint: "<spec_version> [--network westend|paseo] [--dry-run]"
disable-model-invocation: true
user-invocable: true
allowed-tools: Bash, Read, Edit, Glob, Grep
---

# Release Bulletin Runtime v$ARGUMENTS

You are releasing a Bulletin Chain runtime to testnets. Follow these steps precisely.

## Arguments

- `$0` - New spec_version (e.g., `1000002`)
- `--network` - Target: `westend` (default), `paseo`, or `both`
- `--dry-run` - Simulate only, no changes

## Pre-Release Coordination

**Before starting, confirm:**
1. Westend and Paseo Bulletin collators are using `unstable-bulletin-support-v1`
2. Coordinate with Nikola or ping Naren about collator readiness

## Step 1: Pre-flight Checks

```bash
git status --porcelain
git branch --show-current
```

Read current spec_version from `runtimes/bulletin-westend/src/lib.rs`:
```rust
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_version: 1_000_001,  // Find this
    ...
};
```

**Report:**
- Current spec_version: X
- New spec_version: $0
- Git branch: <branch>

**Stop if:**
- New spec_version <= current (must increment)
- Working directory not clean

## Step 2: Bump spec_version

Edit `runtimes/bulletin-westend/src/lib.rs`:

Find the `RuntimeVersion` block and update:
```rust
spec_version: $0,
```

## Step 3: Build Production Runtime

```bash
cargo build --profile production \
  -p bulletin-westend-runtime \
  --features on-chain-release-build
```

Verify WASM exists:
```bash
ls -la target/production/wbuild/bulletin-westend-runtime/*.compact.compressed.wasm
```

## Step 4: Run Tests

```bash
cargo test -p bulletin-westend-runtime
cargo clippy -p bulletin-westend-runtime -- -D warnings
```

**If tests fail:** Stop and report. Do not continue.

## Step 5: Commit and Tag (skip if --dry-run)

```bash
git add runtimes/bulletin-westend/src/lib.rs
git commit -m "chore(runtime): bump bulletin-westend spec_version to $0"
git tag runtime-westend-v$0
git push origin HEAD --tags
```

## Step 6: Upgrade Runtimes via Sudo

**For Westend:**
1. Open Polkadot.js Apps: https://polkadot.js.org/apps/?rpc=wss://westend-bulletin-rpc.polkadot.io
2. Go to Developer → Sudo
3. Submit `system.setCode` with the WASM blob
4. Verify: Runtime version should show $0

**For Paseo:**
1. Open Polkadot.js Apps for Paseo Bulletin
2. Same process: Sudo → `system.setCode`
3. Verify: Runtime version should show $0

## Step 7: Verify and Run Live Tests

**After upgrade is confirmed on-chain:**

```bash
cd examples

# For Westend
just run-live-tests-westend "<seed>"

# For Paseo
just run-live-tests-paseo "<seed>"
```

**If Paseo RPC issues:**
- Escalate to infra team
- Or send script to Nikola to run (keep it simple, no Kubo setup needed)

## Step 8: Report Success

```
✅ Runtime v$0 released!

Upgraded:
- [ ] Westend Bulletin (spec_version: $0)
- [ ] Paseo Bulletin (spec_version: $0)

Live tests:
- [ ] Westend: just run-live-tests-westend
- [ ] Paseo: just run-live-tests-paseo

Collator version: unstable-bulletin-support-v1
```

## Dry Run Mode

If `--dry-run`:
- Complete steps 1-4 (checks, bump, build, test)
- Show what WOULD be committed
- Do NOT commit, tag, push, or upgrade

## Rollback

If upgrade fails:
```bash
# Revert to previous runtime via sudo
# Use the previous WASM blob from git history or releases
```

## Links

- [Westend Bulletin](https://polkadot.js.org/apps/?rpc=wss://westend-bulletin-rpc.polkadot.io)
- [Bulletin Westend Runtime](/runtimes/bulletin-westend/)
- [Playbook/Runbook](link-to-runbook-if-exists)
