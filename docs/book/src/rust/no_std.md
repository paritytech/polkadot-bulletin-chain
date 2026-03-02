# no_std Support

The Rust SDK is designed to be compatible with `no_std` environments, making it ideal for use in constrained environments like embedded systems or WebAssembly runtimes.

> **Important**: The `no_std` feature is for **data preparation and verification** only. You cannot call `store()` or `retrieve()` methods in no_std contexts. These high-level APIs require networking and async support which are only available with the `std` feature.

## Configuration

Disable default features in your `Cargo.toml`:

```toml
[dependencies]
bulletin-sdk-rust = { version = "0.1", default-features = false }
```

## Limitations in `no_std`

When using the SDK without the `std` feature:

- **No `store()` or `retrieve()`**: High-level client APIs require std for networking
- **No Networking**: Cannot use `subxt` or connect to nodes
- **No Async**: Async/await requires std library support
- **Data Preparation Only**: Use for CID calculation, chunking, and verification

## What Works in `no_std`

The following SDK features are fully functional in no_std environments:

- ✅ **CID Calculation**: Compute CIDs for any data
- ✅ **Chunking**: Split data into optimal chunks
- ✅ **DAG-PB Generation**: Create manifests for chunked data
- ✅ **Authorization Helpers**: Calculate required authorization
- ✅ **Type Definitions**: All core types are no_std compatible

## Example: Verifying a CID

You can use the SDK to verify that data matches a claimed CID in constrained environments:

```rust
#![no_std]
use bulletin_sdk_rust::prelude::*;

fn verify_upload(data: &[u8], claimed_cid: &[u8]) -> bool {
    let calculated = calculate_cid_default(data).expect("Failed to calc CID");
    let cid_bytes = cid_to_bytes(&calculated).expect("Failed to convert");

    // Compare bytes
    cid_bytes.to_bytes() == claimed_cid
}
```

## Example: Preparing Chunked Data

```rust
#![no_std]
use bulletin_sdk_rust::{chunker::{Chunker, FixedSizeChunker}, types::ChunkerConfig};
extern crate alloc;
use alloc::vec::Vec;

fn prepare_chunks(data: &[u8]) -> Vec<Vec<u8>> {
    let config = ChunkerConfig {
        chunk_size: 1024 * 1024, // 1 MiB chunks
        max_parallel: 1, // Serial processing in no_std
        create_manifest: false,
    };

    let chunker = FixedSizeChunker::new(config);
    let chunks = chunker.chunk(data).expect("Failed to chunk");

    chunks.into_iter().map(|c| c.data).collect()
}
```

## Use Cases

**Embedded Systems**: Calculate CIDs on IoT devices before uploading via a gateway.

**WASM Modules**: Use in WebAssembly modules for client-side data preparation and verification.

**Substrate Pallets**: Use CID calculation and chunking logic for **on-chain verification** (e.g., verify that submitted data matches a claimed CID). Note: You cannot use the `store()` API in pallets - that's for off-chain clients only.

**Resource-Constrained Environments**: Run CID calculations on systems without std library support.

## Memory Considerations

In no_std environments, be mindful of memory constraints:

- Use streaming chunking for large files
- Process one chunk at a time instead of loading all data into memory
- Consider chunk size based on available RAM
- Use iterators where possible to avoid allocations
