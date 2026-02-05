---
name: release
description: Guide runtime release process for Bulletin Chain networks
---

# Bulletin Chain Release

Automate or guide runtime releases for Bulletin Chain networks.

## Usage

```
/release <network> [options]
```

**Networks**: `testnet`, `westend`, `paseo`, `pop`, `polkadot`

**Options**:
- `--auto` - Full automation (requires `--seed`)
- `--seed "<SEED>"` - Sudo account seed for automated upgrade
- `--to <step>` - Run up to and including this step, then stop
- `--from <step>` - Start from this step (assumes prior steps done)
- `--skip-tests` - Skip pre-checks (tests, clippy, fmt)
- `--dry-run` - Show what would be done without executing

**Steps**: `check`, `bump`, `commit`, `push`, `build`, `download`, `upgrade`, `verify`

## Examples

```bash
/release westend                           # Interactive guided mode
/release westend --auto --seed "//Alice"   # Full automation
/release westend --to push                 # Bump, commit, push, then stop
/release westend --from download           # Download WASM and upgrade
/release westend --skip-tests --to push    # Quick release without tests
```

## Network Parameters

| Parameter | testnet | westend | paseo | pop | polkadot |
|-----------|---------|---------|-------|-----|----------|
| runtime | polkadot-bulletin-chain | bulletin-westend | bulletin-westend | bulletin-westend | bulletin-polkadot |
| runtime_file | `runtime/src/lib.rs` | `runtimes/bulletin-westend/src/lib.rs` | same | same | `runtimes/bulletin-polkadot/src/lib.rs` |
| wasm | manual build | `bulletin_westend_runtime.compact.compressed.wasm` | same | same | `bulletin_polkadot_runtime.compact.compressed.wasm` |
| rpc | `ws://localhost:9944` | `wss://westend-bulletin-rpc.polkadot.io` | TBD | TBD | TBD |
| upgrade_method | sudo | sudo | sudo | authorize | authorize |
| ci_release | no | yes | yes | yes | yes |

## Instructions

When invoked, parse arguments and determine the execution mode:

### 1. Parse Arguments

- Extract `<network>` - if missing, ask using AskUserQuestion
- Parse options: `--auto`, `--seed`, `--to`, `--from`, `--skip-tests`, `--dry-run`
- Validate: `--auto` requires `--seed`

### 2. Determine Steps to Execute

Default steps in order: `check` -> `bump` -> `commit` -> `push` -> `build/download` -> `upgrade` -> `verify`

- If `--to <step>`: execute steps up to and including `<step>`
- If `--from <step>`: start from `<step>`, skip prior steps
- If neither: execute all steps

### 3. Execute Steps

**Step: check** (skipped if `--skip-tests`)
```bash
cargo test && cargo clippy --all-targets --all-features --workspace -- -D warnings
```
If tests fail, stop and report.

**Step: bump**
- Read current `spec_version` from runtime file
- Increment by 1
- In `--auto` mode: edit directly
- In interactive mode: show diff and ask for confirmation

**Step: commit**
```bash
git add runtime/ runtimes/
git commit -m "Bump <runtime> spec_version to <new_version>"
```

**Step: push**
- Determine next tag version (check existing tags with `git tag -l`)
- Create tag and push:
```bash
git tag v<version>
git push origin main --tags
```

**Step: build** (only for testnet)
```bash
cargo build --profile production -p polkadot-bulletin-chain-runtime --features on-chain-release-build
```

**Step: download** (for ci_release networks)
- Wait for CI: `gh run list --workflow=release.yml --limit=1 --json status,conclusion`
- Poll until complete (or timeout after 30 minutes)
- Download: `gh release download <tag> -p "*<runtime>*.wasm" -D /tmp`

**Step: upgrade**

If `upgrade_method` = sudo AND `--seed` provided:
```bash
cd examples && node upgrade_runtime.js "<seed>" <wasm_path> <rpc>
```

If `upgrade_method` = sudo AND no `--seed`:
- Provide polkadot.js link: `https://polkadot.js.org/apps/?rpc=<rpc>#/extrinsics`
- Instructions: `sudo.sudo(system.setCode(code))`

If `upgrade_method` = authorize:
- Show Blake2-256 hash from release notes
- Instructions for governance proposal

**Step: verify**
- Query chain: `system.lastRuntimeUpgrade()`
- Compare with expected `spec_version`
- Report success or failure

### 4. Summary

After execution, show:
- Steps completed
- New spec_version
- Release tag
- Links to release page and chain explorer

## Automation Pipeline (--auto mode)

When `--auto` is specified with `--seed`, execute the full pipeline without prompts:

```
check -> bump -> commit -> push -> download -> upgrade -> verify
         |                          |           |
         v                          v           v
    Edit runtime file         Wait for CI   Run upgrade script
```

If any step fails, stop immediately and report the error with recovery suggestions.
