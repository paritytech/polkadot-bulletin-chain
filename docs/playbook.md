# Bulletin Chain Maintenance Playbook

> **Tip:** Use `/release <network>` for guided release assistance.

---

## E2E Release Process

Replace `<NETWORK>` with your target: `westend`, `paseo`, `pop`, or `polkadot`.

### Step 1: Pre-release Checks

```shell
cargo test
cargo clippy --all-targets --all-features --workspace -- -D warnings
cargo +nightly fmt --all -- --check
```

### Step 2: Bump spec_version

Edit `<RUNTIME_FILE>` and increment `spec_version`:

```rust
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_version: 1000003,  // ← increment this
    // ...
};
```

### Step 3: Create a PR with the Version Bump

```shell
git checkout -b bump-<RUNTIME>-spec-version-<VERSION> origin/main
git add runtimes/
git commit -m "Bump <RUNTIME> spec_version to <VERSION>"
git push -u origin bump-<RUNTIME>-spec-version-<VERSION>
gh pr create --title "Bump <RUNTIME> spec_version to <VERSION>"
```

### Step 4: Merge PR & Tag the Release

Tag using the correct version track — `v0.0.X` for testnets, `v1.x.y` for production (see [Versioning Scheme](#versioning-scheme)).

```shell
gh pr merge <PR_NUMBER> --merge
git checkout main && git pull
git tag v<VERSION>
git push origin --tags
```

### Step 5: Download WASM from CI

Monitor [Release CI](https://github.com/paritytech/polkadot-bulletin-chain/actions/workflows/release.yml). Once the tag build completes, download the artifact:

```shell
gh release download <TAG> -p "*.wasm" -D .
```

The release includes:
- `<WASM_ARTIFACT>`
- Blake2-256 hash in release notes

### Step 6: Apply Runtime Upgrade

**If `<UPGRADE_METHOD>` = sudo:**

**Option A - Automated (Recommended):**
```shell
cd examples
gh release download <TAG> -p "*westend*.wasm" -D .
node upgrade_runtime.js "<SUDO_SEED>" ./bulletin_westend_runtime.compact.compressed.wasm <RPC>
```

**Option B - Manual via polkadot.js:**
1. Download `<WASM_ARTIFACT>` from [Releases](https://github.com/paritytech/polkadot-bulletin-chain/releases)
2. Open `https://polkadot.js.org/apps/?rpc=<RPC>#/extrinsics`
3. Submit: `sudo.sudo(system.setCode(code))` → upload WASM
4. Verify: Chain State → `system.lastRuntimeUpgrade()` shows new `spec_version`

**If `<UPGRADE_METHOD>` = authorize:**

1. Get Blake2-256 hash from [release notes](https://github.com/paritytech/polkadot-bulletin-chain/releases)
2. Submit via governance: `system.authorizeUpgrade(0x<HASH>)`
3. Once authorized, download `<WASM_ARTIFACT>` and submit: `system.applyAuthorizedUpgrade(code)`
4. Verify: Chain State → `system.lastRuntimeUpgrade()` shows new `spec_version`

---

## Test Against Live Network

```shell
cd examples && npm install
just run-live-tests-<NETWORK> "//Seed"
```

Currently supported: `westend`

---

## Local Testing

### Prerequisites

```shell
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown
rustup component add rust-src

# System deps (choose one)
sudo apt-get install -y protobuf-compiler libclang-dev  # Ubuntu
brew install protobuf llvm                               # macOS

# Tools
cargo install just staging-chain-spec-builder --locked
```

### Build & Test

```shell
cargo build --release -p <RUNTIME>-runtime
cargo test
```

### Benchmarking

```shell
cargo build --release -p <RUNTIME>-runtime --features runtime-benchmarks
python3 scripts/cmd/cmd.py bench <RUNTIME>
```

### Integration Tests

```shell
cd examples
just run-authorize-and-store <RUNTIME>-runtime ws
just run-store-chunked-data <RUNTIME>-runtime
just run-store-big-data <RUNTIME>-runtime
```

---

## Troubleshooting

| Issue | Solution |
|-------|----------|
| macOS libclang | `ln -s "$(brew --prefix llvm)/lib/libclang.dylib" "$(brew --prefix)/lib/libclang.dylib"` |
| Zombienet stuck | `pkill -f zombienet; pkill -f polkadot; rm -rf /tmp/zombie-*` |
| Runtime upgrade fails | Verify Blake2-256 hash matches release notes |
| IPFS not working | Ensure `--ipfs-server` flag, check: `grep bitswap collator.log` |
| Verify hash locally | `b2sum -l 256 <wasm_file>` |

---

## Versioning Scheme

Two separate version tracks for git tags:

| Track | Networks | Format | Examples |
|-------|----------|--------|----------|
| **Testnet** | westend, paseo | `v0.0.X` | v0.0.4, v0.0.5, v0.0.6 |
| **Production** | polkadot, pop | `v1.x.y` | v1.0.0, v1.0.1, v1.1.0 |

**Rules:**
- Testnet releases increment the **patch** component only: `v0.0.4` → `v0.0.5` → `v0.0.6`.
- Production releases follow semver: bump **minor** for new features, **patch** for fixes.
- A single git tag triggers CI for **all** runtimes in the matrix (westend, paseo, polkadot). The tag version determines which track the release belongs to.
- Never use `v0.0.X` tags for production or `v1.x.y` tags for testnet-only releases.
