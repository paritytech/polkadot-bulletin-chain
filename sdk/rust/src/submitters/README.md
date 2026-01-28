# Transaction Submitters

This directory contains implementations of the `TransactionSubmitter` trait for different blockchain client libraries.

## Available Submitters

### SubxtSubmitter

Uses the `subxt` library for type-safe Substrate/Polkadot blockchain interaction.

**Status**: Placeholder implementation - requires metadata generation

**Usage**:
```rust
use bulletin_sdk_rust::submitters::SubxtSubmitter;
use bulletin_sdk_rust::async_client::AsyncBulletinClient;

// Get URL from config, env, or CLI args
let ws_url = std::env::var("BULLETIN_WS_URL")
    .unwrap_or_else(|_| "ws://localhost:10000".to_string());

let signer = /* your PairSigner */;

// Option 1: Connect via URL (simplest)
let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
let client = AsyncBulletinClient::new(submitter);

// Option 2: Pass pre-connected client (advanced)
// let api = OnlineClient::<PolkadotConfig>::from_url(&ws_url).await?;
// let submitter = SubxtSubmitter::new(api, signer);
```

**Implementation Guide**:
1. Generate metadata from running node:
   ```bash
   subxt metadata -f bytes > metadata.scale
   ```

2. Add metadata generation to your project:
   ```rust
   #[subxt::subxt(runtime_metadata_path = "metadata.scale")]
   pub mod bulletin {}
   ```

3. Implement transaction submission using generated types

## Creating Custom Submitters

You can create submitters for other client libraries (PAPI, custom RPC clients, etc.) by implementing the `TransactionSubmitter` trait:

### Example: Mock Submitter (for testing)

```rust
use bulletin_sdk_rust::submit::{TransactionSubmitter, TransactionReceipt};
use bulletin_sdk_rust::types::Result;
use async_trait::async_trait;

pub struct MockSubmitter {
    // Test configuration
}

#[async_trait]
impl TransactionSubmitter for MockSubmitter {
    async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt> {
        // Mock implementation
        Ok(TransactionReceipt {
            block_hash: "0xmock".into(),
            extrinsic_hash: "0xmock".into(),
            block_number: Some(1),
        })
    }

    // ... implement other methods
}
```

### Example: PAPI-like Submitter

For JavaScript/TypeScript integration via FFI or WASM:

```rust
pub struct PAPISubmitter {
    // FFI callbacks to JavaScript
    js_submit_fn: extern "C" fn(*const u8, usize) -> *const u8,
}

#[async_trait]
impl TransactionSubmitter for PAPISubmitter {
    async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt> {
        // Call JavaScript via FFI
        let result_ptr = (self.js_submit_fn)(data.as_ptr(), data.len());
        // Parse result and return receipt
    }
}
```

## Transaction Submitter Requirements

All submitters must:

1. Implement `TransactionSubmitter` trait
2. Be `Send + Sync` for async usage
3. Handle all TransactionStorage pallet calls:
   - `store` - Store data
   - `authorize_account` - Authorize account (sudo)
   - `authorize_preimage` - Authorize preimage (sudo)
   - `renew` - Renew storage
   - `refresh_account_authorization` - Refresh auth (sudo)
   - `refresh_preimage_authorization` - Refresh auth (sudo)
   - `remove_expired_account_authorization` - Cleanup
   - `remove_expired_preimage_authorization` - Cleanup

4. Return `TransactionReceipt` with:
   - Block hash
   - Extrinsic hash
   - Block number (optional)

## Testing Submitters

Create a mock submitter for unit tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct TestSubmitter;

    #[async_trait]
    impl TransactionSubmitter for TestSubmitter {
        // ... minimal mock implementation
    }

    #[tokio::test]
    async fn test_client_with_submitter() {
        let submitter = TestSubmitter;
        let client = AsyncBulletinClient::new(submitter);
        // Test client methods
    }
}
```

## Contributing

To add a new submitter implementation:

1. Create a new file in this directory: `your_submitter.rs`
2. Implement `TransactionSubmitter` trait
3. Add module declaration to `mod.rs`
4. Add documentation and examples
5. Submit a PR with tests
