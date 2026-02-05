# How to Release

## Quick Start

```
/release westend
```

Claude will guide you through the entire process.

## Manual Steps

1. **Bump version** in the runtime file
2. **Tag & push** → CI builds the WASM
3. **Apply upgrade** via polkadot.js

## Networks

| Network | Runtime file | How to upgrade |
|---------|--------------|----------------|
| westend | `runtimes/bulletin-westend/src/lib.rs` | `sudo.sudo(system.setCode)` |
| paseo | same as westend | `sudo.sudo(system.setCode)` |
| pop | same as westend | `system.authorizeUpgrade` |
| polkadot | `runtimes/bulletin-polkadot/src/lib.rs` | `system.authorizeUpgrade` |
| testnet | `runtime/src/lib.rs` | manual build, `sudo.sudo` |

## Example: Westend Release

```bash
# 1. Edit runtimes/bulletin-westend/src/lib.rs
#    Change: spec_version: 1000002 → 1000003

# 2. Commit and tag
git add runtimes/
git commit -m "Bump bulletin-westend spec_version to 1000003"
git tag v0.0.6
git push origin main --tags

# 3. Wait for CI: https://github.com/paritytech/polkadot-bulletin-chain/actions

# 4. Download WASM from: https://github.com/paritytech/polkadot-bulletin-chain/releases

# 5. Open: https://polkadot.js.org/apps/?rpc=wss://westend-bulletin-rpc.polkadot.io#/extrinsics
#    Submit: sudo → sudo(call) → system → setCode → upload WASM

# 6. Verify: Chain State → system → lastRuntimeUpgrade()
```

## Links

- [Releases](https://github.com/paritytech/polkadot-bulletin-chain/releases)
- [CI](https://github.com/paritytech/polkadot-bulletin-chain/actions/workflows/release.yml)
- [Full Playbook](playbook.md)
