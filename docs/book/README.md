# Polkadot Bulletin SDK Book

> Part of the [Polkadot Bulletin Chain](https://github.com/paritytech/polkadot-bulletin-chain). See the root [README](../../README.md) for project status, disclaimers, and security notices.

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

## License

Apache-2.0
