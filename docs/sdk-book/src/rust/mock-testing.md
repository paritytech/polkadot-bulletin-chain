# Mock Testing

`MockBulletinClient` lets you test without a running blockchain node. It calculates real CIDs but doesn't submit transactions.

## Usage

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::test]
async fn test_store_data() {
    let client = MockBulletinClient::new();

    let data = b"Hello, Mock Bulletin!".to_vec();
    let result = client.store(data.clone()).send().await.unwrap();

    assert_eq!(result.size, data.len() as u64);

    // Verify operations
    let ops = client.operations();
    assert_eq!(ops.len(), 1);

    // Clear between tests
    client.clear_operations();
}
```

## Error Simulation

```rust
#[tokio::test]
async fn test_auth_failure() {
    let mut config = MockClientConfig::default();
    config.simulate_auth_failure = true;

    let client = MockBulletinClient::with_config(config);
    let result = client.store(b"test".to_vec()).send().await;

    assert!(matches!(result, Err(Error::InsufficientAuthorization { .. })));
}
```

## Configuration

```rust
let config = MockClientConfig {
    default_chunk_size: 512 * 1024,
    chunking_threshold: 1024 * 1024,
    simulate_auth_failure: false,
    simulate_storage_failure: false,
    ..Default::default()
};

let client = MockBulletinClient::with_config(config);
```
