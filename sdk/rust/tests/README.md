# Bulletin SDK Rust Tests

This directory contains integration tests for the Bulletin SDK Rust implementation.

## Test Types

### Integration Tests (`integration_tests.rs`)

Full end-to-end tests that connect to a running Bulletin Chain node. These tests cover:

- **Store Operations**
  - Simple data storage
  - Chunked storage with progress tracking
  - Custom CID configurations

- **Authorization Operations**
  - Account authorization
  - Preimage authorization
  - Authorization refresh
  - Expired authorization removal

- **Maintenance Operations**
  - Data renewal
  - Authorization cleanup

- **Core Functionality**
  - CID calculation with different algorithms
  - Chunking logic
  - DAG-PB manifest generation
  - Authorization estimation

## Prerequisites

### 1. Running Bulletin Chain Node

Integration tests require a local Bulletin Chain node running at `ws://localhost:9944`.

```bash
# From the project root
cargo build --release
./target/release/polkadot-bulletin-chain --dev --tmp
```

### 2. Chain Metadata

Integration tests use subxt which requires chain metadata. Generate it with:

```bash
# From the project root
./target/release/polkadot-bulletin-chain export-metadata > sdk/rust/artifacts/metadata.scale
```

## Running Tests

### All Tests (Unit + Integration)

```bash
# From sdk/rust/
cargo test --all-features
```

### Integration Tests Only

Integration tests are marked with `#[ignore]` to prevent them from running without a node. Run them explicitly:

```bash
# Run all integration tests
cargo test --test integration_tests --features std -- --ignored --test-threads=1

# Run specific test
cargo test --test integration_tests --features std test_simple_store -- --ignored --test-threads=1
```

**Note:** `--test-threads=1` is recommended to avoid conflicts when multiple tests interact with the same chain.

### Unit Tests Only

```bash
# Exclude integration tests
cargo test --lib --features std
```

## Test Output

Tests include detailed console output:

```
✅ Simple store test passed
   CID: 0x1234...
   Size: 42 bytes

✅ Chunked store test passed
   Chunks: 5
   Manifest CID: 0x5678...

✅ Account authorization test passed
   Block hash: 0xabcd...
```

## Test Coverage

Current test coverage:

- ✅ Simple store operation
- ✅ Chunked store with progress tracking
- ✅ Account authorization workflow
- ✅ Preimage authorization workflow
- ✅ Authorization refresh
- ✅ CID calculation (Blake2b, SHA2)
- ✅ Chunking logic
- ✅ DAG-PB manifest generation
- ✅ Authorization estimation
- ✅ Error handling

## Troubleshooting

### Connection Failed

```
Error: Connection failed: ...
```

**Solution:** Ensure the Bulletin Chain node is running at `ws://localhost:9944`

### Metadata Not Found

```
Error: Could not load metadata from file
```

**Solution:** Generate metadata:
```bash
./target/release/polkadot-bulletin-chain export-metadata > sdk/rust/artifacts/metadata.scale
```

### Authorization Failed

```
Error: InsufficientAuthorization
```

**Solution:** Tests use Alice's account which should have sudo. Ensure the node is running in `--dev` mode.

### Test Timeout

```
Error: Test timed out
```

**Solution:** Increase test timeout or check node performance. Integration tests may take several seconds per operation.

## Writing New Tests

### Adding Integration Tests

```rust
#[tokio::test]
#[ignore] // Mark as integration test
async fn test_my_feature() -> Result<()> {
    let client = create_test_client("//Alice").await?;

    // Your test logic

    Ok(())
}
```

### Adding Unit Tests

```rust
#[test]
fn test_my_utility() {
    // Your test logic
    assert_eq!(expected, actual);
}
```

## CI/CD Integration

For continuous integration:

```yaml
# .github/workflows/test.yml
- name: Start Bulletin Chain
  run: |
    ./target/release/polkadot-bulletin-chain --dev --tmp &
    sleep 10

- name: Run Integration Tests
  run: |
    cd sdk/rust
    cargo test --test integration_tests --features std -- --ignored --test-threads=1
```

## Performance Benchmarks

Some tests include performance metrics:

```rust
let start = std::time::Instant::now();
let result = client.store_chunked(...).await?;
let duration = start.elapsed();

println!("   Upload time: {:?}", duration);
println!("   Throughput: {} MB/s", data.len() as f64 / duration.as_secs_f64() / 1_048_576.0);
```
