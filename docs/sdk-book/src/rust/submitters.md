# Transaction Submitters

The Rust SDK uses the **Transaction Submitter** pattern to separate blockchain interaction from data preparation. This allows you to choose how to submit transactions (subxt, custom RPC, mock for testing) without changing your application logic.

## Overview

A `TransactionSubmitter` is a trait that defines methods for submitting all TransactionStorage pallet operations:

**Submission Methods:**
- `submit_store` - Store data
- `submit_authorize_account` - Authorize account (sudo)
- `submit_authorize_preimage` - Authorize preimage (sudo)
- `submit_renew` - Renew storage retention
- `submit_refresh_account_authorization` - Refresh authorization (sudo)
- `submit_refresh_preimage_authorization` - Refresh authorization (sudo)
- `submit_remove_expired_account_authorization` - Remove expired auth
- `submit_remove_expired_preimage_authorization` - Remove expired auth

**Query Methods (for authorization checking):**
- `query_account_authorization` - Query account authorization state
- `query_preimage_authorization` - Query preimage authorization state

These query methods enable the SDK to check authorization **before** uploading to fail fast and avoid wasted transaction fees.

## Built-in Submitters

### SubxtSubmitter

Uses the `subxt` library for type-safe blockchain interaction.

**Status**: Template implementation - requires metadata generation

**Usage**:

```rust
use bulletin_sdk_rust::prelude::*;

// Get WebSocket URL from config, environment, or CLI
let ws_url = std::env::var("BULLETIN_WS_URL")
    .unwrap_or_else(|_| "ws://localhost:10000".to_string());

let signer = /* your PairSigner */;

// Connect via URL - simplest approach
let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
let client = AsyncBulletinClient::new(submitter);

// Now use the client
let result = client.store(data, None).await?;
```

**Advanced: Pre-connected Client**

```rust
use subxt::{OnlineClient, PolkadotConfig};

// Connect manually first (for custom config, etc.)
let api = OnlineClient::<PolkadotConfig>::from_url(&ws_url).await?;

// Pass pre-connected client
let submitter = SubxtSubmitter::new(api, signer);
```

**Implementation Note**: The built-in `SubxtSubmitter` is a placeholder that returns errors. For a working implementation, see the example at `examples/rust-authorize-and-store/src/main.rs` which shows:
- **Metadata codegen**: Uses `#[subxt::subxt]` macro to generate types from runtime metadata
- **Custom signed extensions**: Implements ProvideCidConfig for CID configuration
- **Type-safe transactions**: Generated transaction builders from actual runtime
- **Sudo call wrapping**: For authorization operations requiring root
- **Error handling**: Comprehensive error mapping

**Setup**: The example requires generating metadata from your running Bulletin Chain node first:
```bash
cd examples/rust-authorize-and-store
./fetch_metadata.sh <WS_URL>  # e.g., ws://localhost:10000
cargo run --release -- --ws <WS_URL>
```

### MockSubmitter

Mock implementation for testing without a blockchain node.

**Usage**:

```rust
use bulletin_sdk_rust::prelude::*;

// Create mock submitter
let submitter = MockSubmitter::new();
let client = AsyncBulletinClient::new(submitter);

// Test your code without connecting to a node
let result = client.store(data, None).await?;

// Mock generates fake receipts
println!("Mock block: {}", result.block_number.unwrap());
```

**Simulate Failures**:

```rust
let submitter = MockSubmitter::failing();
let client = AsyncBulletinClient::new(submitter);

// All operations will fail with mock errors
let result = client.store(data, options).await; // Returns Err
```

**Mock Authorization Support**:

```rust
use bulletin_sdk_rust::prelude::*;

// Create mock submitter with authorization
let submitter = MockSubmitter::new();
let account = AccountId32::from([1u8; 32]);

// Set mock authorization for testing
submitter.set_account_authorization(
    account.clone(),
    Authorization {
        scope: AuthorizationScope::Account,
        transactions: 100,
        max_size: 10_000_000,
        expires_at: None,
    },
);

// Create client with account for authorization checking
let client = AsyncBulletinClient::new(submitter)
    .with_account(account);

// Upload - authorization will be checked automatically
let result = client.store(data, None).await?;
```

## Authorization Queries

Submitters can implement query methods to enable automatic authorization checking before uploads. This allows the SDK to fail fast if authorization is insufficient.

### Query Methods

```rust
pub trait TransactionSubmitter {
    // ... submission methods ...

    /// Query authorization state for an account.
    /// Returns `None` if no authorization exists or queries are not supported.
    async fn query_account_authorization(
        &self,
        who: AccountId32,
    ) -> Result<Option<Authorization>>;

    /// Query authorization state for a preimage.
    /// Returns `None` if no authorization exists or queries are not supported.
    async fn query_preimage_authorization(
        &self,
        content_hash: ContentHash,
    ) -> Result<Option<Authorization>>;
}
```

### MockSubmitter Implementation

`MockSubmitter` fully implements authorization queries for testing:

```rust
let submitter = MockSubmitter::new();
let account = AccountId32::from([1u8; 32]);

// Set authorization
submitter.set_account_authorization(
    account.clone(),
    Authorization {
        scope: AuthorizationScope::Account,
        transactions: 50,
        max_size: 5_000_000,
        expires_at: Some(1000),
    },
);

// Query it back
let auth = submitter.query_account_authorization(account).await?;
assert!(auth.is_some());
println!("Available: {} txs, {} bytes", auth.unwrap().transactions, auth.unwrap().max_size);
```

### SubxtSubmitter Implementation

`SubxtSubmitter` provides documentation on how to implement queries but requires metadata generation:

```rust
// Example implementation (requires generated metadata):
async fn query_account_authorization(&self, who: AccountId32) -> Result<Option<Authorization>> {
    let address = bulletin_metadata::storage()
        .transaction_storage()
        .account_authorizations(&who);

    let result = self.api.storage().at_latest().await?.fetch(&address).await?;

    Ok(result.map(|auth_data| Authorization {
        scope: AuthorizationScope::Account,
        transactions: auth_data.transactions,
        max_size: auth_data.max_size,
        expires_at: Some(auth_data.expires_at),
    }))
}
```

### How Authorization Checking Works

When you set an account on the client:

```rust
let client = AsyncBulletinClient::new(submitter)
    .with_account(account);  // Enable automatic authorization checking
```

The SDK will:
1. Call `query_account_authorization()` before each upload
2. Validate sufficient transactions and bytes are authorized
3. Fail immediately with `Error::InsufficientAuthorization` if not enough
4. Proceed with upload only if authorization is sufficient

This saves transaction fees and time by catching authorization issues early!

## Creating Custom Submitters

You can implement your own submitter for any blockchain client library:

### Example: Custom RPC Submitter

```rust
use bulletin_sdk_rust::submit::{TransactionSubmitter, TransactionReceipt};
use bulletin_sdk_rust::types::Result;
use async_trait::async_trait;

pub struct CustomRpcSubmitter {
    rpc_url: String,
    api_key: String,
}

impl CustomRpcSubmitter {
    pub fn new(rpc_url: String, api_key: String) -> Self {
        Self { rpc_url, api_key }
    }
}

#[async_trait]
impl TransactionSubmitter for CustomRpcSubmitter {
    async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt> {
        // Build and submit transaction via custom RPC
        let response = reqwest::Client::new()
            .post(&self.rpc_url)
            .header("X-API-Key", &self.api_key)
            .json(&json!({
                "method": "transactionStorage_store",
                "params": [hex::encode(data)],
            }))
            .send()
            .await?;

        // Parse response and return receipt
        // ...
    }

    // ... implement other methods
}
```

### Using Custom Submitters

```rust
let submitter = CustomRpcSubmitter::new(
    "https://my-rpc-endpoint.com".to_string(),
    "my-api-key".to_string(),
);

let client = AsyncBulletinClient::new(submitter);
```

## Connection Configuration Patterns

### From Environment Variable

```rust
let ws_url = std::env::var("BULLETIN_WS_URL")
    .expect("BULLETIN_WS_URL not set");

let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
```

### From Config File

```rust
use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    bulletin_ws_url: String,
}

let config: Config = toml::from_str(&std::fs::read_to_string("config.toml")?)?;
let submitter = SubxtSubmitter::from_url(&config.bulletin_ws_url, signer).await?;
```

### From CLI Arguments

```rust
use clap::Parser;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "ws://localhost:10000")]
    ws: String,
}

let args = Args::parse();
let submitter = SubxtSubmitter::from_url(&args.ws, signer).await?;
```

## Complete Example

See `examples/rust-authorize-and-store/` for a complete working example that demonstrates:
- **Metadata-driven codegen**: Uses `#[subxt::subxt]` to generate types from runtime
- **Custom signed extensions**: Implements `ProvideCidConfig` for Bulletin Chain
- **SubxtSubmitter implementation**: All 8 pallet operations with proper error handling
- **Authorization and storage workflow**: Complete end-to-end example
- **CLI argument handling**: Clean argument parsing with clap

**Usage**:
```bash
# First, generate metadata from your running Bulletin Chain node
cd examples/rust-authorize-and-store
./fetch_metadata.sh <WS_URL>

# Then build and run
cargo run --release -- --ws <WS_URL> --seed "<SEED>"
```

Where:
- `<WS_URL>`: Your node's WebSocket URL (e.g., `ws://localhost:10000`)
- `<SEED>`: Account seed like `//Alice` for dev or your mnemonic

**Controlling Log Output**:

The example uses `tracing` for structured logging. Control log levels with the `RUST_LOG` environment variable:

```bash
# Default (INFO level)
cargo run --release -- --ws <WS_URL> --seed "<SEED>"

# Debug output (more verbose)
RUST_LOG=debug cargo run --release -- --ws <WS_URL> --seed "<SEED>"

# Only show warnings and errors
RUST_LOG=warn cargo run --release -- --ws <WS_URL> --seed "<SEED>"

# Filter specific modules
RUST_LOG=authorize_and_store=debug,subxt=info cargo run --release -- --ws <WS_URL>

# Save logs to file
RUST_LOG=debug cargo run --release -- --ws <WS_URL> 2>&1 | tee output.log

# Filter output in real-time
RUST_LOG=info cargo run --release -- --ws <WS_URL> 2>&1 | grep -i "cid\|error"
```

Log levels: `error`, `warn`, `info` (default), `debug`, `trace`

**Note**: The example requires `bulletin_metadata.scale` to be generated before compilation. See the example's README for details.

## Testing with Submitters

### Unit Tests with MockSubmitter

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bulletin_sdk_rust::prelude::*;

    #[tokio::test]
    async fn test_store_workflow() {
        let submitter = MockSubmitter::new();
        let client = AsyncBulletinClient::new(submitter);

        let data = b"test data".to_vec();
        let result = client.store(data, None).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_error_handling() {
        let submitter = MockSubmitter::failing();
        let client = AsyncBulletinClient::new(submitter);

        let result = client.store(vec![1, 2, 3], None).await;
        assert!(result.is_err());
    }
}
```

### Integration Tests with Real Node

```rust
#[tokio::test]
#[ignore] // Run with: cargo test -- --ignored
async fn test_real_node() {
    let submitter = SubxtSubmitter::from_url(
        "ws://localhost:10000",
        my_test_signer(),
    ).await.unwrap();

    let client = AsyncBulletinClient::new(submitter);

    let data = b"integration test".to_vec();
    let result = client.store(data, None).await;

    assert!(result.is_ok());
}
```

## Best Practices

1. **Use `from_url()` for simplicity** - Let the submitter handle connection setup
2. **Store URL in config** - Don't hardcode localhost, use env vars or config files
3. **Use `MockSubmitter` for tests** - Fast, no node required
4. **Handle errors gracefully** - All submitter methods return `Result`
5. **Implement all 8 methods** - Custom submitters must implement the full trait
6. **Consider retries** - Network errors are common, implement retry logic
7. **Log transactions** - Keep track of submitted transactions for debugging

## Further Reading

- [AsyncBulletinClient API](./basic-storage.md)
- [Authorization Management](./authorization.md)
- [Example Implementation](../../examples/rust-authorize-and-store/)
- [Creating Custom Submitters](../../../sdk/rust/src/submitters/README.md)
