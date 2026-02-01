---
name: release-runtime
description: Release Bulletin Chain runtime to Westend and Paseo testnets
argument-hint: "<spec_version> [--dry-run]"
disable-model-invocation: true
user-invocable: true
allowed-tools: Bash, Read, Edit, Glob, Grep
---

# Release Bulletin Runtime v$ARGUMENTS

Release and upgrade the Bulletin Chain runtime to Westend and Paseo testnets.

## Arguments

- `$0` - New spec_version (e.g., `1000002`)
- `--dry-run` - Simulate only, no changes

## Step 1: Pre-flight Checks

```bash
git status --porcelain
git branch --show-current
```

Read current spec_version from `runtimes/bulletin-westend/src/lib.rs`:
```rust
spec_version: 1_000_001,  // Find current value
```

**Report:**
- Current spec_version: X
- New spec_version: $0

**Stop if:** New spec_version <= current

## Step 2: Bump spec_version

Edit `runtimes/bulletin-westend/src/lib.rs`:

Change:
```rust
spec_version: 1_000_001,
```
To:
```rust
spec_version: $0,
```

## Step 3: Build Production Runtime

```bash
cargo build --profile production \
  -p bulletin-westend-runtime \
  --features on-chain-release-build
```

Verify WASM:
```bash
ls -la target/production/wbuild/bulletin-westend-runtime/*.compact.compressed.wasm
```

## Step 4: Run Tests

```bash
cargo test -p bulletin-westend-runtime
cargo clippy -p bulletin-westend-runtime -- -D warnings
```

**If tests fail:** Stop and report.

## Step 5: Commit and Tag (skip if --dry-run)

```bash
git add runtimes/bulletin-westend/src/lib.rs
git commit -m "chore(runtime): bump bulletin-westend spec_version to $0"
git tag runtime-westend-v$0
git push origin HEAD --tags
```

## Step 6: Upgrade with Sudo

### Westend Bulletin

1. Open: https://polkadot.js.org/apps/?rpc=wss://westend-bulletin-rpc.polkadot.io
2. Developer → Sudo → system.setCode
3. Upload WASM blob
4. Submit and verify spec_version shows $0

### Paseo Bulletin

1. Open Polkadot.js for Paseo Bulletin
2. Developer → Sudo → system.setCode
3. Upload same WASM blob
4. Submit and verify spec_version shows $0

## Step 7: Run Live Tests (after upgrade confirmed)

**Wait for nodes to be running the new version, then:**

```bash
cd examples

# Westend
just run-live-tests-westend "<seed>"

# Paseo
just run-live-tests-paseo "<seed>"
```

### Paseo RPC Issues

If Paseo RPC has problems:
1. Escalate to infra team
2. Or send script to Nikola to run directly
   - Keep it simple - just the script, no Kubo setup needed

## Step 8: Report Success

```
✅ Runtime v$0 released!

Upgraded:
- [x] Westend Bulletin (spec_version: $0)
- [x] Paseo Bulletin (spec_version: $0)

Live tests:
- [ ] just run-live-tests-westend
- [ ] just run-live-tests-paseo
```

## Dry Run Mode

If `--dry-run`:
- Complete steps 1-4 only
- Show what WOULD be committed
- Do NOT commit, tag, push, or upgrade

## Links

- [Westend Bulletin PJS](https://polkadot.js.org/apps/?rpc=wss://westend-bulletin-rpc.polkadot.io)
- [Bulletin Westend Runtime](/runtimes/bulletin-westend/)
- Playbook/Runbook (ask Andrii)
