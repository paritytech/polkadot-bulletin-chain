# Rust SDK

The `bulletin-sdk-rust` crate provides a robust client for interacting with the Bulletin Chain. It is designed to be:

- **Type-Safe**: Leverages Rust's type system to prevent common errors.
- **Flexible**: Works with `std` (standard library) and `no_std` (WASM/embedded) environments.
- **Modular**: Use only what you need (chunking, CID calculation, or full client).

## Modules

- `client`: High-level entry point (`BulletinClient`).
- `chunker`: Splits data into chunks (`FixedSizeChunker`).
- `cid`: CID calculation utilities.
- `storage`: Transaction preparation.
- `authorization`: Authorization helpers.

Proceed to [Installation](./installation.md) to get started.
