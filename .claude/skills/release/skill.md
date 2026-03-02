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

**Networks**: `westend`, `paseo`, `pop`, `polkadot`

## Steps

1. **Pre-checks** (optional): `cargo test && cargo clippy --all-targets --all-features --workspace -- -D warnings`

2. **Bump spec_version** in the appropriate runtime file:
   - westend/paseo/pop: `runtimes/bulletin-westend/src/lib.rs`
   - polkadot: `runtimes/bulletin-polkadot/src/lib.rs`

3. **Create a PR** with the version bump:
   ```bash
   git checkout -b bump-<runtime>-spec-version-<VERSION> origin/main
   git add runtimes/
   git commit -m "Bump <runtime> spec_version to <VERSION>"
   git push -u origin bump-<runtime>-spec-version-<VERSION>
   gh pr create --title "Bump <runtime> spec_version to <VERSION>"
   ```

4. **Merge the PR** and **tag the release** on main — `v0.0.X` for testnets, `v1.x.y` for production (see Versioning below):
   ```bash
   gh pr merge <PR_NUMBER> --squash
   git checkout main && git pull
   git tag -a v<VERSION> -m "Release v<VERSION>"
   git push origin --tags
   ```

5. **Download WASM**: The [Release CI](https://github.com/paritytech/polkadot-bulletin-chain/actions/workflows/release.yml) builds the WASM artifact. **Note:** CI takes a long time (15-30+ minutes). Do NOT poll or check CI status repeatedly — tell the user you are waiting and ask them to notify you when the release is ready. Once notified, download with: `gh release download <TAG> -p "*.wasm" -D .`

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
  --network <name>   westend, paseo, pop, polkadot (default: westend)
  --rpc <url>        Custom RPC endpoint
  --verify-only      Only check current version
```

## Versioning Scheme

Two separate version tracks for git tags:

| Track | Networks | Format | Examples |
|-------|----------|--------|----------|
| **Testnet** | westend, paseo | `v0.0.X` | v0.0.4 → v0.0.5 → v0.0.6 |
| **Production** | polkadot, pop | `v1.x.y` | v1.0.0, v1.0.1, v1.1.0 |

- Testnet: increment patch only (`v0.0.X` → `v0.0.X+1`)
- Production: semver — minor for features, patch for fixes
- Never mix tracks: no `v0.0.X` for production, no `v1.x.y` for testnets

When determining the next version, check `git tag --sort=-v:refname` to find the latest tag in the appropriate track.
