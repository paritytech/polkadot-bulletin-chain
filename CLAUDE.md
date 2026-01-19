# CLAUDE.md - Polkadot Bulletin Chain

## Project Overview

Polkadot Bulletin Chain is a specialized blockchain providing distributed data storage and retrieval infrastructure for the Polkadot ecosystem. It serves as a storage solution primarily for the People/Proof-of-Personhood chain, functioning as a bridge-connected parachain with integrated IPFS support.

**Deployment Modes**:
- **Solochain**: Run with the custom `node/` binary which includes BABE + GRANDPA consensus and integrated IPFS support
- **Parachain**: Run with Polkadot SDK's `polkadot-omni-node` for parachain deployments

**Key Purpose**: Store arbitrary data with proof-of-storage guarantees and make it accessible via IPFS, with data retention managed over a configurable period (currently 2 weeks).

## Build Commands

```bash
# Build the node (debug)
cargo build

# Build the node (release)
cargo build --release

# Build production runtime (with optimizations)
cargo build --profile production -p bulletin-polkadot-runtime
cargo build --profile production -p bulletin-westend-runtime

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
cargo clippy --all-targets --all-features -- -D warnings

# Format check
cargo fmt --check
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
- `pallet-transaction-storage` - Stores data, manages 2-week retention, provides storage proofs
- `pallet-validator-set` - Dynamic validator set management (PoA)
- `pallet-relayer-set` - Manages bridge relayers between Bulletin and PoP chain

## Development Workflow

1. **Format code**: `cargo fmt`
2. **Run clippy**: `cargo clippy --all-targets`
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

## Notes

- Storage retention is 2 weeks (~201,600 blocks)
- Maximum storage requirement: 1.5-2TB
- IPFS idle connection timeout: 1 hour
- Node supports litep2p/Bitswap
- Solochain validators need BABE and GRANDPA session keys
