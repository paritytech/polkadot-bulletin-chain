# Mock Testing

The Rust SDK provides `MockBulletinClient` for testing your application logic without requiring a running blockchain node.

## Overview

`MockBulletinClient` simulates blockchain operations without actually submitting transactions. It:
- Calculates real CIDs using the same logic as the real client
- Tracks all operations performed for verification
- Supports error simulation for testing failure paths
- Provides the same builder pattern API as `AsyncBulletinClient`

## Basic Usage

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::test]
async fn test_store_data() {
    // Create mock client
    let client = MockBulletinClient::new();

    // Store data (no blockchain required)
    let data = b"Hello, Mock Bulletin!".to_vec();
    let result = client.store(data.clone()).send().await.unwrap();

    // Verify the CID was calculated
    assert_eq!(result.size, data.len() as u64);

    // Check operations performed
    let ops = client.operations();
    assert_eq!(ops.len(), 1);
    match &ops[0] {
        MockOperation::Store { data_size, .. } => {
            assert_eq!(*data_size, data.len());
        },
        _ => panic!("Expected Store operation"),
    }
}
```

## Builder Pattern

The mock client supports the same builder pattern as the real client:

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::test]
async fn test_builder_pattern() {
    let client = MockBulletinClient::new();
    let data = b"Test data".to_vec();

    let result = client
        .store(data)
        .with_codec(CidCodec::Raw)
        .with_hash_algorithm(HashAlgorithm::Blake2b256)
        .with_finalization(true)
        .send()
        .await
        .unwrap();

    assert!(result.cid.len() > 0);
}
```

## Error Simulation

Test error handling by simulating failures:

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::test]
async fn test_authorization_failure() {
    let mut config = MockClientConfig::default();
    config.simulate_auth_failure = true;

    let client = MockBulletinClient::with_config(config);
    let data = b"Test data".to_vec();

    let result = client.store(data).send().await;

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::InsufficientAuthorization { .. } => {
            // Expected error
        },
        _ => panic!("Expected InsufficientAuthorization error"),
    }
}

#[tokio::test]
async fn test_storage_failure() {
    let mut config = MockClientConfig::default();
    config.simulate_storage_failure = true;

    let client = MockBulletinClient::with_config(config);
    let data = b"Test data".to_vec();

    let result = client.store(data).send().await;

    assert!(result.is_err());
    match result.unwrap_err() {
        Error::SubmissionFailed(msg) => {
            assert_eq!(msg, "Simulated storage failure");
        },
        _ => panic!("Expected SubmissionFailed error"),
    }
}
```

## Verifying Operations

Track all operations performed during testing:

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::test]
async fn test_multiple_operations() {
    let client = MockBulletinClient::new();

    // Perform multiple stores
    client.store(b"Data 1".to_vec()).send().await.unwrap();
    client.store(b"Data 2".to_vec()).send().await.unwrap();
    client.store(b"Data 3".to_vec()).send().await.unwrap();

    // Verify all operations
    let ops = client.operations();
    assert_eq!(ops.len(), 3);

    // Clear for next test
    client.clear_operations();
    assert_eq!(client.operations().len(), 0);
}
```

## Configuration

Customize the mock client behavior:

```rust
use bulletin_sdk_rust::prelude::*;

let config = MockClientConfig {
    default_chunk_size: 512 * 1024, // 512 KiB
    max_parallel: 4,
    create_manifest: true,
    check_authorization_before_upload: true,
    chunking_threshold: 1 * 1024 * 1024, // 1 MiB
    simulate_auth_failure: false,
    simulate_storage_failure: false,
};

let client = MockBulletinClient::with_config(config);
```

## Testing Authorization

The mock client supports authorization operations:

```rust
use bulletin_sdk_rust::prelude::*;
use sp_runtime::AccountId32;

#[tokio::test]
async fn test_authorization() {
    let client = MockBulletinClient::new();

    // Test authorization estimation
    let estimate = client.estimate_authorization(10_000_000); // 10 MB
    assert!(estimate.0 > 0); // transactions
    assert!(estimate.1 > 0); // bytes
}
```

## Best Practices

1. **Use for Unit Tests**: Mock client is perfect for testing application logic without blockchain overhead
2. **Verify CIDs**: The mock calculates real CIDs, so you can verify correctness
3. **Test Error Paths**: Use error simulation to ensure your error handling works
4. **Track Operations**: Use `operations()` to verify the right operations were performed
5. **Clean State**: Call `clear_operations()` between tests for isolation

## Limitations

- Does not actually submit transactions to a blockchain
- Does not validate on-chain state or permissions
- Block numbers are always mock values (1)
- No actual data retention or storage limits

For integration tests that require real blockchain interaction, use `AsyncBulletinClient` with a local test node.
