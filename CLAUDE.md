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

**Deployment Mode**: Parachain, run with the Polkadot SDK's `polkadot-omni-node` binary against the `bulletin-westend-runtime` WASM.

**Key Purpose**: Store arbitrary data with proof-of-storage guarantees and make it accessible via IPFS, with data retention managed over a configurable `RetentionPeriod`.

## Build Commands

```bash
# Build the runtime (debug)
cargo build

# Build the runtime (release)
cargo build --release

# Build production runtime (with optimizations, strips logs)
cargo build --profile production -p bulletin-westend-runtime --features on-chain-release-build

# Build with runtime benchmarks enabled
cargo build --release --features runtime-benchmarks
```

## Test Commands

```bash
# Run all tests
cargo test

# Run pallet tests
cargo test -p pallet-bulletin-transaction-storage
cargo test -p pallet-validator-set
cargo test -p pallet-relayer-set

# Run runtime tests
cargo test -p bulletin-westend-runtime

# Run XCM integration tests
cargo test -p bulletin-westend-integration-tests
```

For formatting, linting, and clippy checks, run `/format`.

## Architecture

### Directory Structure

```
polkadot-bulletin-chain/
├── runtimes/
│   └── bulletin-westend/     # Westend parachain runtime
├── pallets/
│   ├── common/               # Shared pallet utilities
│   ├── transaction-storage/  # Core storage pallet
│   ├── validator-set/        # PoA validator management
│   └── relayer-set/          # Bridge relayer management
├── sdk/                      # Rust and TypeScript SDKs
├── examples/                 # JavaScript/Rust integration examples
├── scripts/                  # Build and deployment scripts
└── zombienet/                # Network testing configurations
```

### Key Components

**Runtime**: A WASM runtime targeting the Westend testnet parachain slot.
- `runtimes/bulletin-westend/` - Westend testnet runtime (also used for Paseo)

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
zombienet spawn zombienet/bulletin-westend-local.toml
```

### Benchmarking

```bash
# Run benchmarks using the Python script
python3 scripts/cmd/cmd.py bench --runtime bulletin-westend
```

## Polkadot SDK (Upstream)

This project is built on the **Polkadot SDK** (formerly Substrate/Polkadot/Cumulus). For deeper understanding of the underlying framework, pallets, and patterns used here, refer to:

- **Repository**: https://github.com/paritytech/polkadot-sdk
- **SDK CLAUDE.md**: https://github.com/paritytech/polkadot-sdk/blob/master/CLAUDE.md

The Polkadot SDK provides:
- FRAME pallet system and runtime macros
- Consensus engines (Aura for parachain block authoring; relay-chain GRANDPA finality)
- Networking (libp2p, litep2p)
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
- IPFS idle connection timeout: 1 hour (configured on the collator/full-node via `polkadot-omni-node`)
- IPFS retrieval supports litep2p/Bitswap

## Code Review Guidelines

For the full review criteria (Parity Standards), see the `/review` skill. The review bot and all contributors follow those guidelines.

### Using the Claude Review Bot

- **@claude** - Mention in any comment to ask questions or request help
- **Assign to claude[bot]** - Assign an issue to have Claude analyze and propose solutions
- **Label with `claude`** - Add the `claude` label to an issue for Claude to investigate

## SDK Development Guidelines

When developing or modifying the Bulletin SDK (Rust or TypeScript):

- **Scope**: Only implement what is directly needed for core functionality (storage, authorization, CID/chunking)
- **No kitchen sink**: Don't add generic utilities (retry, sleep, batch, etc.) - users have their own libraries
- **No placeholders**: Either implement correctly or don't include - no hardcoded placeholder values
- **No reimplementing**: If functionality exists in standard libraries or common packages (@polkadot/util-crypto, etc.), don't reimplement it
- **Minimal API surface**: Smaller, focused APIs are easier to maintain and less likely to have bugs
