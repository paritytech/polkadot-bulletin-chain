# CLAUDE.md - Polkadot Bulletin Chain

## Agent Rules

**Git commit rules:**
- NEVER add Co-Authored-By lines to commits
- NEVER use git push --force or git push -f

**Automatic formatting:**
- ALWAYS run `/format` after generating or modifying Rust code
- ALWAYS run `/format` before creating any git commit
- This ensures all code follows project formatting standards (Rust, TOML, feature propagation) and passes clippy

**Documentation style:**
- When adding or modifying docs (rustdoc, comments, Markdown), keep them minimal, simple, and correct — no fluff
- Drop field/parameter docs when the name is self-evident in context
- One short line is usually enough; only add more when the *why* is non-obvious (hidden constraints, surprising invariants, workarounds)
- Do not narrate the change or restate what the code does; the reader can see that

## Project Overview

Polkadot Bulletin Chain is a specialized blockchain providing distributed data storage and retrieval infrastructure for the Polkadot ecosystem. It serves as a storage solution primarily for the People/Proof-of-Personhood chain, functioning as a bridge-connected parachain with integrated IPFS support.

**Deployment**: Run as a parachain with Polkadot SDK's `polkadot-omni-node`.

**Key Purpose**: Store arbitrary data with proof-of-storage guarantees and make it accessible via IPFS, with data retention managed over a configurable `RetentionPeriod`.

**Note**: The previous solochain version has been removed and is no longer maintained. Only the parachain runtime is supported.

## Build Commands

```bash
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
cargo test -p pallet-transaction-storage

# Run runtime tests
cargo test -p bulletin-westend-runtime
```

For formatting, linting, and clippy checks, run `/format`.

## Architecture

### Directory Structure

```
polkadot-bulletin-chain/
├── runtimes/
│   └── bulletin-westend/     # Parachain runtime (used for Westend, Paseo, and other deployments)
├── pallets/
│   ├── common/               # Shared pallet utilities
│   └── transaction-storage/  # Core storage pallet
├── examples/                 # JavaScript integration examples
├── scripts/                  # Build and deployment scripts
└── zombienet/                # Network testing configurations
```

### Key Components

**Runtime**: A single WASM parachain runtime (`runtimes/bulletin-westend/`) used across all deployments (Westend, Paseo, etc.). Run with `polkadot-omni-node`.

**Core Pallets**:
- `pallet-transaction-storage` - Stores data, manages retention, provides storage proofs

## Development Workflow

1. **Format and lint**: Run `/format`
2. **Run tests**: `cargo test`
3. **Build**: `cargo build --release`

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
- IPFS idle connection timeout: 1 hour
- Node supports litep2p/Bitswap

## Code Review Guidelines (Parity Standards)

For the full review criteria (Parity Standards), see the `/review` skill. The review bot and all contributors follow those guidelines.

### Rust Code Quality

- **Error Handling**: Use `Result` types with meaningful error enums. Avoid `unwrap()` and `expect()` in production code; they are acceptable in tests.
- **Arithmetic Safety**: Use `checked_*`, `saturating_*`, or `wrapping_*` arithmetic to prevent overflow. Never use raw arithmetic operators on user-provided values.
- **Naming**: Follow Rust naming conventions (snake_case for functions/variables, CamelCase for types).
- **Complexity**: Prefer simple, readable code. Avoid over-engineering and premature abstractions.
- **No useless comments**: Comments should mostly explain **why** things are done, not **how**. The code should be readable enough to explain the how.

### FRAME Pallet Standards

- **Storage**: Use appropriate storage types (`StorageValue`, `StorageMap`, `StorageDoubleMap`, `CountedStorageMap`).
- **Events**: Emit events for all state changes that external observers need to track.
- **Errors**: Define descriptive error types in the pallet's `Error` enum.
- **Weights**: All extrinsics must have accurate weight annotations. Update benchmarks when logic changes.
- **Origins**: Use the principle of least privilege for origin checks.
- **Hooks**: Be cautious with `on_initialize` and `on_finalize`; they affect block production time. Never panic or do unbounded iteration in them. Always benchmark them properly.

### Security Considerations

- **No Panics in Runtime**: Runtime code must never panic. Use defensive programming with `defensive_*` macros.
- **Bounded Collections**: Use `BoundedVec`, `BoundedBTreeMap` etc. to prevent unbounded storage growth.
- **Input Validation**: Validate all user inputs at the entry point.
- **Storage Deposits**: Consider requiring deposits for user-created storage items.
- **Arithmetic**: Always use checked arithmetic for financial calculations.
- **Access Control**: Verify origin permissions before state changes.

### Testing Requirements

- **Unit Tests**: All new functionality requires unit tests.
- **Edge Cases**: Test boundary conditions, error paths, and malicious inputs.
- **Integration Tests**: Complex features should have integration tests.
- **Mock Tests**: Use `mock.rs` and `TestExternalities` for pallet tests.
- **Provider Node Tests**: Test HTTP API endpoints and storage layer.
- **Client SDK Tests**: Test all public SDK methods.

### PR Requirements

- **Single Responsibility**: Each PR should address one concern.
- **Tests Pass**: All CI checks must pass (`cargo test`, `cargo clippy`, `cargo fmt`).
- **No Warnings**: Code should compile without warnings.
- **Documentation**: Public APIs require rustdoc comments.
- **Changelog**: Update changelog for user-facing changes.

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
