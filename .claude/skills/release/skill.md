---
name: release
description: Guide runtime release process for Bulletin Chain networks
---

# Bulletin Chain Release

Guide the user through runtime releases. Reference `docs/playbook.md` for full details.

## Usage

```
/release <network>
```

**Networks**: `testnet`, `westend`, `paseo`, `pop`, `polkadot`

## Steps

1. **Pre-checks** (optional): `cargo test && cargo clippy --all-targets --all-features --workspace -- -D warnings`

2. **Bump spec_version** in the appropriate runtime file:
   - testnet: `runtime/src/lib.rs`
   - westend/paseo/pop: `runtimes/bulletin-westend/src/lib.rs`
   - polkadot: `runtimes/bulletin-polkadot/src/lib.rs`

3. **Create a PR** with the version bump:
   ```bash
   git checkout -b bump-<runtime>-spec-version-<VERSION> origin/main
   git add runtimes/ runtime/
   git commit -m "Bump <runtime> spec_version to <VERSION>"
   git push -u origin bump-<runtime>-spec-version-<VERSION>
   gh pr create --title "Bump <runtime> spec_version to <VERSION>"
   ```

4. **Merge the PR** and **tag the release** on main:
   ```bash
   gh pr merge <PR_NUMBER> --merge
   git checkout main && git pull
   git tag v<VERSION>
   git push origin --tags
   ```

5. **Build/Download WASM**:
   - testnet: `cargo build --profile production -p polkadot-bulletin-chain-runtime --features on-chain-release-build`
   - others: wait for the [Release CI](https://github.com/paritytech/polkadot-bulletin-chain/actions/workflows/release.yml), then `gh release download <TAG> -p "*.wasm" -D .`

6. **Upgrade**: Use the upgrade script in `examples/`:
   ```bash
   node upgrade_runtime.js "<SEED>" ./path/to/runtime.wasm --network <network>
   ```

7. **Verify**: Confirm `spec_version` matches the new version:
   ```bash
   node upgrade_runtime.js --verify-only --network <network>
   ```
   Expected output should show the bumped `spec_version`. If it doesn't match, the upgrade failed.

## Upgrade Script Reference

```bash
node upgrade_runtime.js <seed> <wasm_path> [options]

Options:
  --network <name>   testnet, westend, paseo, pop, polkadot (default: westend)
  --rpc <url>        Custom RPC endpoint
  --verify-only      Only check current version
  --dry-run          Show what would happen
```
