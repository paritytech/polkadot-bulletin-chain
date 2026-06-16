# Polkadot Bulletin SDK Book

> [!WARNING]
> This is a reference implementation provided for research, experimentation, and developer education. This code has not been fully audited. It is actively under development and may contain bugs, vulnerabilities, or incomplete features. It is not recommended for production use without independent review. Use at your own risk.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../../LICENSE-APACHE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-yellow.svg)](#)

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain).

This directory contains the source for the Polkadot Bulletin SDK documentation book.

## How to Build & View

This documentation is built using [mdBook](https://github.com/rust-lang/mdBook).

### Prerequisites

You need to have `mdbook` installed. If you have Rust installed, you can install it via Cargo:

```bash
cargo install mdbook
```

### Viewing the Book

1.  Navigate to this directory:
    ```bash
    cd docs/book
    ```

2.  Serve the book locally:
    ```bash
    mdbook serve --open
    ```

3.  Build the static HTML:
    ```bash
    mdbook build
    ```
    The output will be in `book/`.

## Security

See the [root README](../../README.md#security) for security notices and responsible deployment guidance.

For Parity's security disclosure process and Bug Bounty program, visit: https://parity.io/bug-bounty

## License

Apache-2.0
