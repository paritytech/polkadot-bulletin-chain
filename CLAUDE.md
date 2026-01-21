# CLAUDE.md - Polkadot Bulletin Chain

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
cargo test -p polkadot-bulletin-chain-runtime
cargo test -p bulletin-polkadot-runtime
cargo test -p bulletin-westend-runtime

# Run clippy linting
cargo clippy --all-targets --all-features --workspace -- -D warnings

# Format check
cargo +nightly fmt --all -- --check
```

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
├── runtime/                  # Rococo testnet runtime (WASM)
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

**Node (`node/`)**: Off-chain validator/full-node binary with BaBE + GRANDPA consensus and integrated IPFS (Bitswap/Kademlia).

**Runtimes**: Three WASM runtimes targeting different networks:
- `runtime/` - Rococo testnet (bridges via BridgeHub)
- `runtimes/bulletin-polkadot/` - Production Polkadot (bridges to People Chain)
- `runtimes/bulletin-westend/` - Westend testnet

**Core Pallets**:
- `pallet-transaction-storage` - Stores data, manages retention, provides storage proofs
- `pallet-validator-set` - Dynamic validator set management (PoA)
- `pallet-relayer-set` - Manages bridge relayers between Bulletin and PoP chain

## Development Workflow

1. **Format code**: `cargo +nightly fmt --all -- --check`
2. **Run clippy**: `cargo clippy --all-targets --all-features --workspace`
3. **Run tests**: `cargo test`
4. **Build**: `cargo build --release`

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

## Notes

- Configurable Storage Retention Period
- Maximum storage requirement: 1.5-2TB
- IPFS idle connection timeout: 1 hour
- Node supports litep2p/Bitswap
- Solochain validators need BABE and GRANDPA session keys

## Code Review Guidelines (Parity Standards)

These guidelines are used by the Claude Code review bot and should be followed by all contributors.

### Rust Code Quality

- **Error Handling**: Use `Result` types with meaningful error enums. Avoid `unwrap()` and `expect()` in production code; they are acceptable in tests.
- **Arithmetic Safety**: Use `checked_*`, `saturating_*`, or `wrapping_*` arithmetic to prevent overflow. Never use raw arithmetic operators on user-provided values.
- **Naming**: Follow Rust naming conventions (snake_case for functions/variables, CamelCase for types).
- **Complexity**: Prefer simple, readable code. Avoid over-engineering and premature abstractions.

### FRAME Pallet Standards

- **Storage**: Use appropriate storage types (`StorageValue`, `StorageMap`, `StorageDoubleMap`, `CountedStorageMap`).
- **Events**: Emit events for all state changes that external observers need to track.
- **Errors**: Define descriptive error types in the pallet's `Error` enum.
- **Weights**: All extrinsics must have accurate weight annotations. Update benchmarks when logic changes.
- **Origins**: Use the principle of least privilege for origin checks.
- **Hooks**: Be cautious with `on_initialize` and `on_finalize`; they affect block production time.

### Security Considerations

- **No Panics in Runtime**: Runtime code must never panic. Use defensive programming.
- **Bounded Collections**: Use `BoundedVec`, `BoundedBTreeMap` etc. to prevent unbounded storage growth.
- **Input Validation**: Validate all user inputs at the entry point.
- **Storage Deposits**: Consider requiring deposits for user-created storage items that are returned once the item is cleared.

### Testing Requirements

- **Unit Tests**: All new functionality requires unit tests.
- **Edge Cases**: Test boundary conditions, error paths, and malicious inputs.
- **Integration Tests**: Complex features should have integration tests using `sp-io::TestExternalities`.
- **Benchmark Tests**: Features affecting weights should have benchmark tests.

### PR Requirements

- **Single Responsibility**: Each PR should address one concern.
- **Tests Pass**: All CI checks must pass (`cargo test`, `cargo clippy`, `cargo fmt`).
- **No Warnings**: Code should compile without warnings.
- **Documentation**: Public APIs require rustdoc comments.

### Using the Claude Review Bot

The repository has a Claude Code review bot that automatically reviews PRs. You can also interact with it:

- **@claude** - Mention in any comment to ask questions or request help
- **Assign to claude[bot]** - Assign an issue to have Claude analyze and propose solutions
- **Label with `claude`** - Add the `claude` label to an issue for Claude to investigate

The bot enforces these guidelines and provides actionable feedback with fix suggestions.
