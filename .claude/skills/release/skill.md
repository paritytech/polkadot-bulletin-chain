---
name: release
description: Guide runtime release process for Bulletin Chain networks
---

# Bulletin Chain Release

Guide the user through releasing to a specific network.

## Usage

`/release <network>` where network is: `testnet`, `westend`, `paseo`, `pop`, `polkadot`

## Network Parameters

| Parameter | testnet | westend | paseo | pop | polkadot |
|-----------|---------|---------|-------|-----|----------|
| runtime | polkadot-bulletin-chain | bulletin-westend | bulletin-westend | bulletin-westend | bulletin-polkadot |
| runtime_file | `runtime/src/lib.rs` | `runtimes/bulletin-westend/src/lib.rs` | ← same | ← same | `runtimes/bulletin-polkadot/src/lib.rs` |
| wasm | manual build | `bulletin_westend_runtime.compact.compressed.wasm` | ← same | ← same | `bulletin_polkadot_runtime.compact.compressed.wasm` |
| rpc | `ws://localhost:9944` | `wss://westend-bulletin-rpc.polkadot.io` | TBD | TBD | TBD |
| upgrade_method | sudo | sudo | sudo | authorize | authorize |
| ci_release | no | yes | yes | yes | yes |
| is_parachain | no | yes | yes | yes | yes |

## Instructions

When invoked:

1. **Parse argument** - If no network provided, ask using AskUserQuestion with options: testnet, westend, paseo, pop, polkadot

2. **Check current state** - Read the runtime file and show current `spec_version`

3. **Guide through steps:**

   **Step 1: Pre-checks**
   ```
   cargo test && cargo clippy --all-targets --all-features --workspace -- -D warnings
   ```

   **Step 2: Bump spec_version**
   - Offer to edit the runtime file
   - Increment spec_version by 1

   **Step 3: Commit & Tag**
   ```
   git add runtime/ runtimes/
   git commit -m "Bump <runtime> spec_version to <new_version>"
   git tag v<version>
   git push origin main --tags
   ```

   **Step 4: Build WASM**

   If `ci_release` = no (testnet):
   ```
   cargo build --profile production -p polkadot-bulletin-chain-runtime --features on-chain-release-build
   ```
   Output: `target/production/wbuild/polkadot-bulletin-chain-runtime/polkadot_bulletin_chain_runtime.compact.compressed.wasm`

   If `ci_release` = yes:
   - Link: https://github.com/paritytech/polkadot-bulletin-chain/actions/workflows/release.yml
   - Wait for CI to complete
   - Download from https://github.com/paritytech/polkadot-bulletin-chain/releases

   **Step 5: Apply upgrade**

   If `upgrade_method` = sudo:
   - Link to polkadot.js: `https://polkadot.js.org/apps/?rpc=<rpc>#/extrinsics`
   - Submit: `sudo.sudo(system.setCode(code))`

   If `upgrade_method` = authorize:
   - Get Blake2-256 hash from release notes
   - Submit: `system.authorizeUpgrade(0x<hash>)`
   - Then: `system.applyAuthorizedUpgrade(code)`

   **Step 6: Verify**
   - Check `system.lastRuntimeUpgrade()` matches new version

4. **Offer help** at each step - bump version, create commit, check CI status, provide links
