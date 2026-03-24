# CLAUDE.md - Polkadot Bulletin Chain

## Agent Rules

**Git commit rules:**
- NEVER add Co-Authored-By lines to commits
- NEVER use git push --force or git push -f

**Automatic formatting:**
- ALWAYS run `/format` after generating or modifying Rust code
- ALWAYS run `/format` before creating any git commit
- This ensures all code follows project formatting standards (Rust, TOML, feature propagation) and passes clippy

## Project Overview

Polkadot Bulletin Chain is a specialized blockchain providing distributed data storage and retrieval infrastructure for the Polkadot ecosystem. It serves as a storage solution primarily for the People/Proof-of-Personhood chain, functioning as a bridge-connected parachain with integrated IPFS support.

**Deployment Modes**:
- **Solochain**: Run with the custom `node/` binary which includes BABE + GRANDPA consensus and integrated IPFS support
- **Parachain**: Run with Polkadot SDK's `polkadot-omni-node` for parachain deployments

**Key Purpose**: Store arbitrary data with proof-of-storage guarantees and make it accessible via IPFS, with data retention managed over a configurable `RetentionPeriod`.

## Build Commands

```bash
# Build the node (debug)
cargo build

# Build the node (release)
cargo build --release

# Build production runtime (with optimizations, strips logs)
cargo build --profile production -p bulletin-polkadot-runtime --features on-chain-release-build
cargo build --profile production -p bulletin-westend-runtime --features on-chain-release-build

# Build with runtime benchmarks enabled
cargo build --release --features runtime-benchmarks
```

## Test Commands

```bash
# Run all tests
cargo test

# Run pallet tests
cargo test -p pallet-transaction-storage
cargo test -p pallet-validator-set
cargo test -p pallet-relayer-set

# Run runtime tests
cargo test -p bulletin-polkadot-runtime
cargo test -p bulletin-westend-runtime
```

For formatting, linting, and clippy checks, run `/format`.

## Run Commands

```bash
# Run local dev node
./target/release/polkadot-bulletin-chain --dev

# Run with IPFS server enabled
./target/release/polkadot-bulletin-chain --ipfs-server --validator --chain bulletin-polkadot

# Generate chain spec
./target/release/polkadot-bulletin-chain build-spec --chain bulletin-polkadot > spec.json
```

## Architecture

### Directory Structure

```
polkadot-bulletin-chain/
├── node/                     # Off-chain solochain node implementation (CLI, service, RPC)
├── runtimes/
│   ├── bulletin-polkadot/    # Production Polkadot runtime
│   └── bulletin-westend/     # Westend testnet runtime
├── pallets/
│   ├── common/               # Shared pallet utilities
│   ├── transaction-storage/  # Core storage pallet
│   ├── validator-set/        # PoA validator management
│   └── relayer-set/          # Bridge relayer management
├── examples/                 # JavaScript integration examples
├── scripts/                  # Build and deployment scripts
└── zombienet/                # Network testing configurations
```

### Key Components

**Node (`node/`)**: Off-chain validator/full-node binary with BABE + GRANDPA consensus and integrated IPFS (Bitswap/Kademlia).

**Runtimes**: Two WASM runtimes targeting different networks:
- `runtimes/bulletin-polkadot/` - Production Polkadot (bridges to People Chain)
- `runtimes/bulletin-westend/` - Westend testnet

**Core Pallets**:
- `pallet-transaction-storage` - Stores data, manages retention, provides storage proofs
- `pallet-validator-set` - Dynamic validator set management (PoA)
- `pallet-relayer-set` - Manages bridge relayers between Bulletin and PoP chain

## Development Workflow

1. **Format and lint**: Run `/format`
2. **Run tests**: `cargo test`
3. **Build**: `cargo build --release`

### Zombienet Testing

Local network spawning for integration tests:
```bash
# Requires zombienet binary and polkadot binaries
zombienet spawn zombienet/bulletin-polkadot-local.toml
```

### Benchmarking

```bash
# Run benchmarks using the Python script
python3 scripts/cmd/cmd.py bench
```

## Polkadot SDK (Upstream)

This project is built on the **Polkadot SDK** (formerly Substrate/Polkadot/Cumulus). For deeper understanding of the underlying framework, pallets, and patterns used here, refer to:

- **Repository**: https://github.com/paritytech/polkadot-sdk
- **SDK CLAUDE.md**: https://github.com/paritytech/polkadot-sdk/blob/master/CLAUDE.md

The Polkadot SDK provides:
- FRAME pallet system and runtime macros
- Consensus engines (BABE, GRANDPA)
- Networking (libp2p, litep2p)
- Bridge pallets for cross-chain messaging
- XCM (Cross-Consensus Messaging) infrastructure

## Dependencies

- **Polkadot SDK**: See `Cargo.toml` for the pinned revision
- **Rust**: Nightly toolchain for WASM compilation

## Feature Flags

- `runtime-benchmarks` - Enable weight generation
- `try-runtime` - Runtime migration testing
- `std` - Standard library features (default)
- `on-chain-release-build` - Production build that strips logs for smaller wasm size

## Operational Limits & Requirements

- Configurable Storage Retention Period
- Maximum storage requirement: 1.5-2TB
- IPFS idle connection timeout: 1 hour
- Node supports litep2p/Bitswap
- Solochain validators need BABE and GRANDPA session keys

## Code Review Guidelines

For the full review criteria (Parity Standards), see the `/review` skill. The review bot and all contributors follow those guidelines.

### Using the Claude Review Bot

- **@claude** - Mention in any comment to ask questions or request help
- **Assign to claude[bot]** - Assign an issue to have Claude analyze and propose solutions
- **Label with `claude`** - Add the `claude` label to an issue for Claude to investigate
