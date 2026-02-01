---
name: release-runtime
description: Release Bulletin Chain runtime to testnets (Westend/Paseo/PoP)
argument-hint: "<version> [--dry-run]"
disable-model-invocation: true
user-invocable: true
allowed-tools: Bash, Read, Edit, Glob, Grep
---

# Release Bulletin Runtime v$ARGUMENTS

Release the Bulletin Chain runtime. All deployments use `bulletin-westend-runtime`.

## Arguments

- `$0` - Version tag (e.g., `1.2.0`)
- `--dry-run` - Simulate only, no changes

## Step 1: Pre-flight Checks

```bash
git status --porcelain
git branch --show-current
```

Check current spec_version in `runtimes/bulletin-westend/src/lib.rs`.

## Step 2: Prepare Release

### Run Tests

```bash
cargo test
cargo clippy --all-targets --all-features --workspace -- -D warnings
cargo +nightly fmt --all -- --check
```

### Update Benchmarks (if weights changed)

```bash
cargo build --release -p bulletin-westend-runtime --features runtime-benchmarks
python3 scripts/cmd/cmd.py bench bulletin-westend
```

### Build Production Runtime

```bash
cargo build --profile production -p bulletin-westend-runtime --features on-chain-release-build
```

Verify output:
```bash
ls -la target/production/wbuild/bulletin-westend-runtime/bulletin_westend_runtime.compact.compressed.wasm
```

## Step 3: Commit and Tag (skip if --dry-run)

```bash
git add -A
git commit -m "chore: release v$0"
git tag v$0
git push origin HEAD --tags
```

CI will build artifacts for: bulletin-polkadot, bulletin-westend, bulletin-paseo, bulletin-pop

## Step 4: Runtime Upgrade

### Testnets (Westend/Paseo) - Method 1: Immediate

1. Download `bulletin_westend_runtime.compact.compressed.wasm` from GitHub release
2. Open polkadot.js:
   - Westend: https://polkadot.js.org/apps/?rpc=wss://westend-bulletin-rpc.polkadot.io
   - Paseo: https://polkadot.js.org/apps/?rpc=wss://paseo-bulletin-rpc.polkadot.io
3. Developer → Extrinsics → `sudo.sudo(system.setCode(code))`
4. Verify: Developer → Chain State → `system.lastRuntimeUpgrade()`

### Production (Polkadot) - Method 2: Authorized

1. Download WASM, calculate hash:
   ```bash
   cat bulletin_westend_runtime.compact.compressed.wasm | sha256sum
   ```
2. Submit: `system.authorizeUpgrade(0x<hash>)`
3. Submit: `system.applyAuthorizedUpgrade(code)`

## Step 5: Verify and Test

After upgrade confirmed on-chain:

```bash
cd examples && npm install
just run-tests-against-westend "//Seed"
```

### If Paseo RPC Issues

- Escalate to infra team
- Or send script to operator to run directly (keep simple, no extra setup)

## Step 6: Report Success

```
✅ Runtime v$0 released!

CI artifacts: bulletin-polkadot, bulletin-westend, bulletin-paseo, bulletin-pop

Upgraded:
- [ ] Westend (Para ID: 2487)
- [ ] Paseo
- [ ] PoP (if applicable)

Live tests:
- [ ] just run-tests-against-westend "//Seed"
```

## Dry Run Mode

If `--dry-run`:
- Run tests and build only
- Show what WOULD be committed/tagged
- Do NOT commit, tag, push, or upgrade

## Network Info

| Network | Para ID | RPC |
|---------|---------|-----|
| Westend | 2487 | `wss://westend-bulletin-rpc.polkadot.io` |
| Paseo | TBD | `wss://paseo-bulletin-rpc.polkadot.io` |
| Polkadot | TBD | TBD |

## Links

- [Westend PJS](https://polkadot.js.org/apps/?rpc=wss://westend-bulletin-rpc.polkadot.io)
- [bulletin-westend-runtime](/runtimes/bulletin-westend/)
- [Maintenance Playbook](docs/playbook.md)
