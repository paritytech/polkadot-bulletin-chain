# Bulletin Chain Maintenance Playbook

> **Tip:** Use `/release <network>` for guided release assistance.

---

## Network Configuration

| Parameter | Testnet | Westend | Paseo | PoP | Polkadot |
|-----------|---------|---------|-------|-----|----------|
| **Runtime** | polkadot-bulletin-chain | bulletin-westend | bulletin-westend | bulletin-westend | bulletin-polkadot |
| **Runtime File** | `runtime/src/lib.rs` | `runtimes/bulletin-westend/src/lib.rs` | ← same | ← same | `runtimes/bulletin-polkadot/src/lib.rs` |
| **WASM Artifact** | manual build | `bulletin_westend_runtime...wasm` | ← same | ← same | `bulletin_polkadot_runtime...wasm` |
| **Para ID** | N/A (solochain) | 2487 | TBD | TBD | TBD |
| **RPC** | `ws://localhost:9944` | `wss://westend-bulletin-rpc.polkadot.io` | TBD | TBD | TBD |
| **Relay Chain** | N/A (solochain) | westend | paseo | polkadot | polkadot |
| **Upgrade Method** | sudo | sudo | sudo | authorize | authorize |
| **Chain Spec** | `--dev` or `--chain local` | `bulletin-westend-spec.json` | `bulletin-paseo-spec.json` | `bulletin-pop-spec.json` | `bulletin-polkadot-spec.json` |
| **CI Release** | ✗ manual | ✓ | ✓ | ✓ | ✓ |

**Upgrade Methods:**
- `sudo` = `sudo.sudo(system.setCode(code))` - testnets with sudo key
- `authorize` = `system.authorizeUpgrade(hash)` + `system.applyAuthorizedUpgrade(code)` - production chains

**Notes:**
- **Testnet** is the solochain runtime for local development (not a parachain, no relay chain)
- **Westend/Paseo/PoP** share the same `bulletin-westend-runtime`

---

## E2E Release Process

Replace `<NETWORK>` with your target network from the table above.

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

### Step 3: Commit, Tag & Push

```shell
git add runtimes/
git commit -m "Bump <RUNTIME> spec_version to <VERSION>"
git tag v<VERSION>
git push origin main --tags
```

### Step 4: Build / Wait for CI

**For Testnet (manual build):**
```shell
cargo build --profile production -p polkadot-bulletin-chain-runtime --features on-chain-release-build
# Output: target/production/wbuild/polkadot-bulletin-chain-runtime/polkadot_bulletin_chain_runtime.compact.compressed.wasm
```

**For all other networks:**
Monitor [Release CI](https://github.com/paritytech/polkadot-bulletin-chain/actions/workflows/release.yml).

Produces:
- `<WASM_ARTIFACT>`
- Blake2-256 hash in release notes

### Step 5: Apply Runtime Upgrade

**If `<UPGRADE_METHOD>` = sudo:**

1. Download `<WASM_ARTIFACT>` from [Releases](https://github.com/paritytech/polkadot-bulletin-chain/releases)
2. Open `https://polkadot.js.org/apps/?rpc=<RPC>#/extrinsics`
3. Submit: `sudo.sudo(system.setCode(code))` → upload WASM
4. Verify: Chain State → `system.lastRuntimeUpgrade()`

**If `<UPGRADE_METHOD>` = authorize:**

1. Get Blake2-256 hash from [release notes](https://github.com/paritytech/polkadot-bulletin-chain/releases)
2. Submit via governance: `system.authorizeUpgrade(0x<HASH>)`
3. Once authorized, download `<WASM_ARTIFACT>` and submit: `system.applyAuthorizedUpgrade(code)`
4. Verify: Chain State → `system.lastRuntimeUpgrade()`

---

## Node Configuration

**Testnet (solochain):**
```shell
./target/release/polkadot-bulletin-chain --dev --ipfs-server --validator
```

**Parachains (Westend/Paseo/PoP/Polkadot):**
```shell
polkadot-omni-node \
  --chain <CHAIN_SPEC> \
  --collator \
  --ipfs-server \
  -lparachain=info,runtime=debug,xcm=trace,sub-libp2p::bitswap=trace,runtime::transaction-storage=trace \
  -- --chain <RELAY_CHAIN>
```

**Required:** `--ipfs-server` for transaction storage.

---

## Test Against Live Network

```shell
cd examples && npm install
just run-tests-against-<NETWORK> "//Seed"
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

### Zombienet

```shell
# 1. Setup (one-time)
cd examples && just setup-parachain-prerequisites
curl -L -o zombienet https://github.com/paritytech/zombienet/releases/download/v1.3.138/zombienet-$(uname -s | tr '[:upper:]' '[:lower:]')-x64
chmod +x zombienet

# 2. Create chain spec
./scripts/create_<RUNTIME>_spec.sh

# 3. Spawn network
POLKADOT_BINARY_PATH=~/local_bulletin_testing/bin/polkadot \
POLKADOT_PARACHAIN_BINARY_PATH=~/local_bulletin_testing/bin/polkadot-omni-node \
./zombienet -p native spawn ./zombienet/<RUNTIME>-local.toml
```

**Local Endpoints:** Relay Alice `ws://localhost:9942` | Collator `ws://localhost:10000`

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
