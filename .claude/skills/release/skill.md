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

**Networks**: `westend`, `paseo`

Each runtime is released independently — one tag triggers one runtime build.

## Steps

1. **Pre-checks** (optional): `cargo test && cargo clippy --all-targets --all-features --workspace -- -D warnings`

2. **Bump spec_version** in the appropriate runtime file:
   - westend: `runtimes/bulletin-westend/src/lib.rs`
   - paseo: `runtimes/bulletin-paseo/src/lib.rs`

3. **Create a PR** with the version bump:
   ```bash
   git checkout -b bump-<network>-spec-version-<VERSION> origin/main
   git add runtimes/
   git commit -m "Bump <network> spec_version to <VERSION>"
   git push -u origin bump-<network>-spec-version-<VERSION>
   gh pr create --title "Bump <network> spec_version to <VERSION>"
   ```

4. **Merge the PR** and **tag the release** on main. Tag must end in `-westend` or `-paseo` — the suffix tells CI which runtime to build:
   ```bash
   gh pr merge <PR_NUMBER> --squash
   git checkout main && git pull
   git tag -a v<VERSION>-<network> -m "Release v<VERSION>-<network>"
   git push origin v<VERSION>-<network>
   ```

5. **Download WASM**: The [Release CI](https://github.com/paritytech/polkadot-bulletin-chain/actions/workflows/release.yml) builds the WASM artifact for the runtime matching the tag suffix. **Note:** CI takes a long time (15-30+ minutes). Do NOT poll or check CI status repeatedly — tell the user you are waiting and ask them to notify you when the release is ready. Once notified, download with: `gh release download <TAG> -p "*.wasm" -D .`

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
  --network <name>   westend, paseo (default: westend)
  --rpc <url>        Custom RPC endpoint
  --verify-only      Only check current version
```

## Versioning Scheme

Each runtime has its own independent tag track, distinguished by suffix:

| Network  | Format             | Examples                                  |
|----------|--------------------|-------------------------------------------|
| westend  | `v0.0.X-westend`   | v0.0.10-westend → v0.0.11-westend         |
| paseo    | `v0.0.X-paseo`     | v0.0.1-paseo → v0.0.2-paseo               |

- Westend and paseo bump independently — bumping westend does not bump paseo.
- Bare `vX.Y.Z` tags (no suffix) are not picked up by Release CI and will not produce artifacts.

When determining the next version for a network, filter tags by suffix:

```bash
# next westend version
git tag --list 'v*-westend' --sort=-v:refname | head -1

# next paseo version
git tag --list 'v*-paseo' --sort=-v:refname | head -1
```
