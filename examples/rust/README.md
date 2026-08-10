# Rust Examples

> [!WARNING]
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../../LICENSE-APACHE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain).

Rust examples for interacting with Polkadot Bulletin Chain via the [`bulletin-sdk-rust`](../../sdk/rust/) crate.

## Examples

### authorize-and-store

Uses the SDK's `TransactionClient` and `BulletinClient` to:

- Authorize an account to store data (requires sudo)
- Store small data with pre-calculated CID verification
- Store large data via chunked upload with a DAG-PB manifest
- Track transaction progress through callbacks

No metadata generation is required — the SDK handles all chain interaction.

See [`authorize-and-store/README.md`](authorize-and-store/README.md) for details, example output, and code walkthroughs.

## Prerequisites

- A running Bulletin Chain node with a WebSocket endpoint (see the [root README](../../README.md) quickstart)
- A seed account with sudo privileges (for authorization)

## Usage

```bash
cd authorize-and-store
cargo run --release -- --ws ws://localhost:10000 --seed "//Eve"
```

Options:

- `--ws <URL>` — WebSocket URL of the node (default: `ws://localhost:10000`)
- `--seed <SEED>` — seed phrase or dev seed such as `//Eve` (default: `//Eve`)

Control log verbosity with `RUST_LOG`, e.g. `RUST_LOG=debug`.

## Security

See the [root README](../../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
