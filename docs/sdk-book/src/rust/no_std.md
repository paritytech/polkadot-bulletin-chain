# no_std & ink! Support

The Rust SDK is designed to be compatible with `no_std` environments, making it ideal for use within **ink! smart contracts** or other Substrate runtimes.

## Configuration

Disable default features in your `Cargo.toml`:

```toml
[dependencies]
bulletin-sdk-rust = { version = "0.1", default-features = false }
```

If using in an ink! contract, enable the `ink` feature to ensure proper type encoding:

```toml
bulletin-sdk-rust = { version = "0.1", default-features = false, features = ["ink"] }
```

## Limitations in `no_std`

- **Networking**: The SDK does not handle networking in `no_std`. You cannot use `subxt` or `tokio`.
- **Async**: Async/await is generally not available or requires a specific runtime.
- **Functionality**: Core logic (chunking, CID calculation, DAG generation) works perfectly.

## Example: Verifying a CID in a Contract

You can use the SDK to verify that a piece of data matches a claimed CID inside a smart contract.

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
