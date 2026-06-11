# Polkadot Bulletin Chain

> [!WARNING]                                                                                                                            
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

The Bulletin Chain is a parachain providing distributed data storage and retrieval infrastructure for the Polkadot ecosystem. It stores arbitrary data with proof-of-storage guarantees and makes it accessible via IPFS, with data retention managed over a configurable period (default ~14 days). It is run using Polkadot SDK's `polkadot-omni-node`.

[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)
[![Security: unaudited](https://img.shields.io/badge/security-unaudited-red.svg)](https://github.com/paritytech/polkadot-bulletin-chain/security) 
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)
[![Polkadot SDK](https://img.shields.io/badge/built%20with-Polkadot%20SDK-green.svg)](#)


## Overview

The main purpose of the Bulletin Chain is to provide storage for the People Chain (Proof-of-Personhood). Data is added via authorized extrinsics, indexed with Blake2b-256 hashes, and retrievable from IPFS or directly from the node.

### How it works

1. **Authorization** - Storage access is granted by a privileged origin — Root (sudo), a sibling parachain over XCM, or a registered authorizer — either for a specific account (`authorize_account`) or for data with a specific content hash (`authorize_preimage`).
2. **Storage** - Once authorized, data is submitted via `transactionStorage.store`. Client SDKs automatically chunk large files with DAG-PB manifests for IPFS compatibility.
3. **Retrieval** - Stored data can be retrieved from IPFS via Bitswap, or directly from the node via the transaction index or content hash.
4. **Retention & Renewal** - Data is retained for a configurable period. It can be renewed before expiry to extend retention, with support for automatic renewal.

### People Chain integration

The People Chain root calls `transactionStorage.authorize_preimage` (over XCM) to prime Bulletin to expect data with a given hash. A user account then submits the data via `transactionStorage.store`.

## Quickstart

The shortest path to a running Bulletin chain: launch a single dev node, authorize an account with sudo, then store and retrieve data. For multi-node networks and the full recipe list, see the [development guide](./docs/development.md).

Prerequisites: a Rust toolchain and [`just`](https://github.com/casey/just) (`cargo install just --locked`).

### 1. Fetch binaries and build a chain spec

```bash
just binaries-polkadot      # polkadot-omni-node (+ relay binaries), cached in ./.polkadot-binaries/
just chain-spec westend     # builds the runtime, writes zombienet/bulletin-westend-spec.json
```

### 2. Launch a dev node

A single node with no relay chain, producing and finalizing its own blocks. `//Alice` holds the sudo key.

```bash
OMNI_NODE="$(just binaries-polkadot)/polkadot-omni-node"
"$OMNI_NODE" --chain ./zombienet/bulletin-westend-spec.json --dev --ipfs-server
```

- RPC / WebSocket: `ws://127.0.0.1:9944`
- `--dev` wipes the node's database on exit; `--ipfs-server` lets IPFS peers fetch stored data from the node over Bitswap.

### 3. Authorize an account (sudo)

A fresh chain stores nothing until an account is authorized. On a dev chain `//Alice` is the sudo key, so use it to grant access. Connect [Polkadot.js Apps](https://polkadot.js.org/apps/?rpc=ws%3A%2F%2F127.0.0.1%3A9944) to `ws://127.0.0.1:9944`, then:

**Developer → Sudo → `transactionStorage.authorizeAccount`**
- `who`: the account to authorize (e.g. `//Bob`)
- `transactions`: number of stores allowed (e.g. `100`)
- `bytes`: total byte allowance (e.g. `104857600`, i.e. 100 MiB)

Submit as sudo. The grant lasts for the [authorization period](#authorization--allowances) (~14 days).

> To sponsor a single known blob instead of an account, use **Sudo → `transactionStorage.authorizePreimage(content_hash, max_size)`** — anyone can then store data matching that hash. This is the call the People Chain makes over XCM.

### 4. Store data

As the authorized account, submit **Developer → Extrinsics → `transactionStorage.store(data)`**. The chain content-addresses the data by its Blake2b-256 hash (the CID) and serves it to IPFS peers over Bitswap.

### 5. Retrieve data

The node speaks IPFS **Bitswap** (libp2p), not HTTP — to fetch a CID over HTTP, point an IPFS gateway or light client at the node: a local [Kubo](https://github.com/ipfs/kubo) node peered to it, or Helia/smoldot in the browser (see [Console UI](#console-ui)). With such a gateway listening on port 8283:

```bash
curl "http://127.0.0.1:8283/ipfs/<CID>" -o out.bin
```

### End-to-end in one command

The bundled examples spin up a local network **and** a Kubo gateway, then authorize, store (chunking large files), and read back over IPFS — the quickest way to see the full round trip. This is self-contained and does not use the dev node above; see [examples/README.md](./examples/README.md):

```bash
cd examples
just run-authorize-and-store bulletin-westend-runtime ws kubo-native
```

## Architecture

Repository layout, pallet descriptions, and runtime details are in [docs/architecture.md](./docs/architecture.md).

## Storage Model

All data on the Bulletin Chain has the same retention period (~14 days). Two operations interact with this storage, differing in how they consume allowances:

- **`store`** — writes new data and starts a fresh retention countdown.
- **`renew`** — re-indexes data that is about to expire, resetting its retention countdown. The chain tracks total renewed bytes in a global `PermanentStorageUsed` counter for capacity planning.

When data reaches the end of its retention period without being renewed, it is automatically cleaned up.

### Authorization & Allowances

All storage operations require prior authorization, granted by a privileged origin (Root, a sibling parachain over XCM, or a registered authorizer). Each authorization carries an `AuthorizationExtent` — a set of counters that share a single `bytes_allowance` cap but enforce it differently depending on the operation:

| Counter | Enforcement | Behavior |
|---|---|---|
| `bytes` / `transactions` | **Soft** (store) | Saturate upward on every `store`. Never reject — exceeding the allowance just reduces the transaction's priority boost (via `AllowanceBasedPriority`), letting under-budget accounts land first. |
| `bytes_permanent` | **Hard** (renew) | Increments on every `renew`. Rejects with `PermanentAllowanceExceeded` when `bytes_permanent + size > bytes_allowance`. |
| `bytes_allowance` / `transactions_allowance` | Caps | Set at grant time. `bytes_allowance` is shared between store (soft) and renew (hard). |

This design means `store` is always accepted (authorization just needs to exist and not be expired), but accounts that have exceeded their budget are naturally deprioritized in favor of those still within budget. Renewals, which commit to retaining data longer, are strictly capped.

All counters reset to zero when an expired authorization is re-granted, starting a fresh window.

### Chain-wide Renewal Cap

A global `MaxPermanentStorageSize` limits total renewed bytes across all authorizations. A `renew` is rejected when `PermanentStorageUsed + size > MaxPermanentStorageSize`. When usage crosses 80% of the cap, a `PermanentStorageNearCap` event is emitted as a signal for off-chain governance to raise the cap or coordinate another bulletin chain.

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

- `AsyncBulletinClient` for end-to-end storage workflows
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

Building, fetching pinned `polkadot-sdk` binaries, running zombienet networks, and the full `just` recipe list are documented in the [development guide](./docs/development.md).

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

## Security

Before deploying for real use cases, you are responsible for:

- Reviewing the code yourself — we publish a reference implementation, not a hardened production build
- Checking that the dependencies are up to date and free of known vulnerabilities
- Securing your own fork or deployment environment (keys, secrets, network configuration)
- Tracking the latest tagged releases/commits for security fixes; older releases are not backported (exceptions might apply)

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

[GPL-3.0-only](./LICENSE)
