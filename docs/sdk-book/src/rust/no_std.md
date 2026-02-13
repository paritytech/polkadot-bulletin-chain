# no_std Support

The SDK works in `no_std` environments for data preparation and verification. High-level APIs (`store()`, networking) require `std`.

## Setup

```toml
[dependencies]
bulletin-sdk-rust = { version = "0.1", default-features = false }
```

## Available in no_std

- CID calculation
- Chunking
- DAG-PB manifest generation
- Authorization estimation
- All core types

## Examples

```rust
#![no_std]
use bulletin_sdk_rust::prelude::*;

fn verify_upload(data: &[u8], claimed_cid: &[u8]) -> bool {
    let calculated = calculate_cid_default(data).expect("Failed to calc CID");
    let cid_bytes = cid_to_bytes(&calculated).expect("Failed to convert");
    cid_bytes.to_bytes() == claimed_cid
}
```

```rust
#![no_std]
use bulletin_sdk_rust::{chunker::{Chunker, FixedSizeChunker}, types::ChunkerConfig};
extern crate alloc;
use alloc::vec::Vec;

fn prepare_chunks(data: &[u8]) -> Vec<Vec<u8>> {
    let config = ChunkerConfig {
        chunk_size: 1024 * 1024,
        max_parallel: 1,
        create_manifest: false,
    };
    let chunker = FixedSizeChunker::new(config);
    chunker.chunk(data).expect("Failed to chunk")
        .into_iter().map(|c| c.data).collect()
}
```

Use cases: embedded/IoT devices, WASM modules, Substrate pallets (on-chain CID verification).
