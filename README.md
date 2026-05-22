# Polkadot Bulletin Chain

The Bulletin Chain is a parachain providing distributed data storage and retrieval infrastructure for the Polkadot ecosystem. It stores arbitrary data with proof-of-storage guarantees and makes it accessible via IPFS, with data retention managed over a configurable period (default ~14 days). It is run using Polkadot SDK's `polkadot-omni-node`.

## Overview

The main purpose of the Bulletin Chain is to provide storage for the People Chain (Proof-of-Personhood). Data is added via authorized extrinsics, indexed with Blake2b-256 hashes, and retrievable from IPFS or directly from the node.

### How it works

1. **Authorization** - Storage access is controlled via root-origin calls. Authorization is granted either for a specific account (`authorize_account`) or for data with a specific content hash (`authorize_preimage`).
2. **Storage** - Once authorized, data is submitted via `transactionStorage.store`. Large files are automatically chunked with DAG-PB manifests for IPFS compatibility.
3. **Retrieval** - Stored data can be retrieved from IPFS via Bitswap, or directly from the node via the transaction index or content hash.
4. **Retention & Renewal** - Data is retained for a configurable period. It can be renewed before expiry to extend retention, with support for automatic renewal.

### People Chain integration

The People Chain root calls `transactionStorage.authorize_preimage` (over XCM) to prime Bulletin to expect data with a given hash. A user account then submits the data via `transactionStorage.store`.

## Architecture

```
polkadot-bulletin-chain/
├── runtimes/
│   ├── bulletin-westend/              # Parachain runtime (Westend testnet)
│   │   └── integration-tests/         # XCM emulator integration tests
│   └── bulletin-paseo/                # Parachain runtime (Paseo testnet)
├── pallets/
│   ├── transaction-storage/           # Core storage pallet
│   │   └── primitives/                # Shared types (ContentHash, CID utilities)
│   ├── hop-promotion/                 # HOP pool data promotion to chain storage
│   └── common/                        # Shared pallet utilities (NoCurrency, call inspection)
├── sdk/
│   ├── rust/                          # Rust SDK (no_std compatible)
│   └── typescript/                    # TypeScript SDK (@parity/bulletin-sdk)
├── console-ui/                        # React web interface
├── examples/                          # JavaScript/TypeScript/Rust integration examples
├── stress-test/                       # Write throughput & Bitswap read benchmarks
├── docs/                              # SDK book, authorization docs, operational playbook
├── scripts/                           # Build, benchmarking, and deployment scripts
└── zombienet/                         # Local parachain network configurations
```

## Storage Model

All data on the Bulletin Chain has the same retention period (~14 days). Two operations interact with this storage, differing in how they consume allowances:

- **`store`** — writes new data and starts a fresh retention countdown.
- **`renew`** — re-indexes data that is about to expire, resetting its retention countdown. The chain tracks total renewed bytes in a global `PermanentStorageUsed` counter for capacity planning.

When data reaches the end of its retention period without being renewed, it is automatically cleaned up.

### Authorization & Allowances

All storage operations require prior authorization, granted via root-origin calls. Each authorization carries an `AuthorizationExtent` — a set of counters that share a single `bytes_allowance` cap but enforce it differently depending on the operation:

| Counter | Enforcement | Behavior |
|---|---|---|
| `bytes` / `transactions` | **Soft** (store) | Saturate upward on every `store`. Never reject — exceeding the allowance just reduces the transaction's priority boost (via `AllowanceBasedPriority`), letting under-budget accounts land first. |
| `bytes_permanent` | **Hard** (renew) | Increments on every `renew`. Rejects with `PermanentAllowanceExceeded` when `bytes_permanent + size > bytes_allowance`. |
| `bytes_allowance` / `transactions_allowance` | Caps | Set at grant time. `bytes_allowance` is shared between store (soft) and renew (hard). |

This design means `store` is always accepted (authorization just needs to exist and not be expired), but accounts that have exceeded their budget are naturally deprioritized in favor of those still within budget. Renewals, which commit to retaining data longer, are strictly capped.

All counters reset to zero when an expired authorization is re-granted, starting a fresh window.

### Chain-wide Renewal Cap

A global `MaxPermanentStorageSize` limits total renewed bytes across all authorizations. A `renew` is rejected when `PermanentStorageUsed + size > MaxPermanentStorageSize`. When usage crosses 80% of the cap, a `PermanentStorageNearCap` event is emitted as a signal for off-chain governance to raise the cap or coordinate another bulletin chain.

## Pallets

### pallet-transaction-storage

Core storage pallet providing distributed data storage and retrieval with authorization-based access control.

**Extrinsics:**
- `store` / `store_with_cid_config` - Store data (with optional CID codec/hash configuration)
- `renew` / `renew_content_hash` - Extend retention of stored data
- `authorize_account` - Grant an account permission to store (with transaction/byte limits)
- `authorize_preimage` - Authorize storage of data with a specific content hash
- `refresh_account_authorization` / `refresh_preimage_authorization` - Extend authorization expiration

**Key features:**
- Authorization-based access control (account-scoped or content-addressed)
- Configurable retention period with automatic cleanup
- Auto-renewal tracking for important data
- Merkle-based storage proofs with chunk validation
- Soft-cap (priority signal) and hard-cap (per-window renewal quota) for storage capacity
- Feeless transaction support via `pallet-skip-feeless-payment`

### pallet-hop-promotion

Promotes near-expiry HOP (Human-Operated Peer) pool data to permanent chain storage. Uses general (unsigned authorized) transactions to fill unused blockspace without charging users. Validates sr25519 signatures and checks that the promoting account has an active Bulletin authorization.

### pallet-common

Shared utilities including `NoCurrency` (a no-op fungible currency for pallets that require one) and call inspection helpers for unwrapping utility/sudo/proxy wrappers during authorization tracking.

## Runtimes

Two parachain runtimes (`bulletin-westend`, `bulletin-paseo`) share the same pallet composition with network-specific constants. Both use 24-second slots (4 relay chain slots), 10 MiB max block length, and a ~14 day retention period.

## SDK

Multi-language client SDKs for submitting data, managing authorizations, and generating IPFS-compatible DAG-PB manifests.

### Rust SDK (`sdk/rust/`)

`no_std` compatible core with optional `std` features for direct transaction submission via subxt.

- Automatic chunking with configurable chunk size (default 1 MiB)
- DAG-PB manifest generation for chunked data
- `BulletinClient` for offline prepare operations
- Progress tracking via callbacks

### TypeScript SDK (`sdk/typescript/`)

Published as `@parity/bulletin-sdk` on npm. Browser and Node.js compatible (requires Node >= 22).

- `BulletinClient` for end-to-end storage workflows
- `FixedSizeChunker` and `UnixFsDagBuilder` for large file handling
- Built on `polkadot-api` (PAPI)

**Quick start:** See [sdk/README.md](./sdk/README.md)

**Full documentation:** See [docs/book/](./docs/book/) (viewable locally with `mdbook serve --open`)

## Console UI

A React 19 + Vite web application for interacting with the Bulletin Chain in the browser. Built with Polkadot API, Smoldot light client, Helia (IPFS), and Tailwind CSS. Includes Playwright E2E tests.

## Build

```bash
# Build production runtime
cargo build --profile production -p bulletin-westend-runtime --features on-chain-release-build

# Build with runtime benchmarks enabled
cargo build --release --features runtime-benchmarks

# Run all tests
cargo test

# Run pallet tests
cargo test -p pallet-bulletin-transaction-storage

# Run runtime tests
cargo test -p bulletin-westend-runtime
```

## Benchmarking

```bash
# Run benchmarks for a specific runtime
python3 scripts/cmd/cmd.py bench --runtime bulletin-westend

# Run all benchmarks
python3 scripts/cmd/cmd.py bench
```

## Stress Testing

The `stress-test/` directory contains a benchmarking tool for measuring write throughput and Bitswap read performance:

```bash
# Throughput benchmark across payload sizes (1KB - 2MB)
bulletin-stress-test throughput

# Bitswap read benchmark across concurrency levels (1-64 clients)
bulletin-stress-test bitswap
```

## Local Development

### One-time setup

Install [`just`](https://github.com/casey/just) — every recipe in this repo is wired through it, and the CI workflows call the same recipes:

```bash
cargo install just --locked
just --list   # see all recipes
```

### polkadot-sdk binaries

All external binaries (`polkadot`, `polkadot-omni-node`, `polkadot-prepare-worker`, `polkadot-execute-worker`, `chain-spec-builder`, `frame-omni-bencher`, `try-runtime`, `zombienet`) are fetched on demand by `scripts/get_polkadot_binaries.sh` and cached under `./.polkadot-binaries/` (gitignored, repo-local — no `$HOME` writes).

Five env vars in `.github/env` pin the version of each group:

| Variable | Drives |
|---|---|
| `POLKADOT_NODE_VERSION` | `polkadot`, 2 workers, `polkadot-omni-node` |
| `FRAME_OMNI_BENCHER_VERSION` | `frame-omni-bencher` |
| `CHAIN_SPEC_BUILDER_VERSION` | `chain-spec-builder` |
| `TRY_RUNTIME_VERSION` | `try-runtime` |
| `ZOMBIENET_VERSION` | `zombienet` (release-tag only) |

Each value is either:
- **a release tag** (e.g. `polkadot-stable2603`) — script downloads the prebuilt asset for your platform (Linux x86_64 or macOS arm64) and verifies its `.sha256` companion file, OR
- **a 40-char commit hash** — script clones `polkadot-sdk` / `try-runtime-cli` once into `./.polkadot-binaries/_src/`, checks out the commit, and builds with `SKIP_WASM_BUILD=1`.

Override at the shell to pin a different version for one session:

```bash
POLKADOT_NODE_VERSION=polkadot-stable2603 just binaries-polkadot
POLKADOT_NODE_VERSION=d6a4f5977b39bf5e5152e2f2bb6719ea92b992ea just binaries-polkadot
```

Useful recipes:

```bash
just binaries-polkadot              # fetch / build polkadot + workers + omni-node
just binaries-chain-spec-builder    # fetch / build chain-spec-builder
just binaries-bencher               # frame-omni-bencher
just binaries-try-runtime           # try-runtime CLI
just binaries-zombienet             # zombienet (release-only)
just binaries-all                   # cold-cache convenience: every group

just chain-spec westend             # build runtime + emit zombienet/bulletin-westend-spec.json
just chain-spec paseo               # same for paseo

just test-pallets                   # pallet unit tests
just test-zombienet-auto-renew      # auto-renew e2e suite (matrix: westend|paseo)
just test-zombienet-sync            # sync e2e suite
just bench <args>                   # frame-omni-bencher with extra args
just try-runtime <args>             # try-runtime CLI with extra args
```

### Zombienet

Local parachain networks can be spun up using the configurations in `zombienet/`:

- `bulletin-westend-local.toml` - Local Westend relay + Bulletin parachain
- `bulletin-paseo-local.toml` - Local Paseo relay + Bulletin parachain

### Examples

The `examples/` directory contains JavaScript, TypeScript, and Rust scripts demonstrating chain interaction:

- Authorization and storage workflows (WebSocket RPC and Smoldot light client)
- Content-addressed (preimage) authorization
- Chunked data storage with DAG-PB manifests
- Large file handling with parallel uploads
- Auto-renewal monitoring
- Runtime upgrades

See [examples/README.md](./examples/README.md) for setup and usage.

## CI/CD

GitHub Actions workflows in `.github/workflows/` cover checks (Rust, SDK, console UI), integration and stress tests, runtime migration testing, crate publishing, releases, and UI deployment.

## Troubleshooting

### macOS build issues

#### `algorithm` file not found error

This means C++ standard library headers can't be found. Fix:

```bash
xcode-select --install
```

If already installed, reinstall:

```bash
sudo rm -rf /Library/Developer/CommandLineTools
xcode-select --install
```

Verify the active developer path: `xcode-select -p` (should be `/Applications/Xcode.app/Contents/Developer` or `/Library/Developer/CommandLineTools`).

If incorrect, set manually: `sudo xcode-select --switch /Library/Developer/CommandLineTools`

See the official [Polkadot SDK macOS guide](https://docs.polkadot.com/develop/parachains/install-polkadot-sdk/#macos) for more.

#### `dyld: Library not loaded: @rpath/libclang.dylib`

```bash
brew install llvm
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
export LD_LIBRARY_PATH="$LIBCLANG_PATH:$LD_LIBRARY_PATH"
export DYLD_LIBRARY_PATH="$LIBCLANG_PATH:$DYLD_LIBRARY_PATH"
export PATH="$(brew --prefix llvm)/bin:$PATH"
```

Verify `libclang.dylib` exists: `ls "$(brew --prefix llvm)/lib/libclang.dylib"`, then rebuild:

```bash
cargo clean
cargo build --release
```

## License

[GPL-3.0-only](./LICENSE)
