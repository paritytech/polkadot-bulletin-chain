# Testing

This guide covers testing strategies for applications that use the Bulletin SDK.

## Unit Testing with BulletinClient

`BulletinClient` performs all operations locally without network access, making it ideal for unit tests:

```rust
#[cfg(test)]
mod tests {
    use bulletin_sdk_rust::prelude::*;

    #[test]
    fn test_prepare_store() {
        let client = BulletinClient::new();
        let data = b"Hello, Bulletin!".to_vec();

        let operation = client.prepare_store(data.clone(), StoreOptions::default()).unwrap();
        assert_eq!(operation.data.len(), data.len());
    }

    #[test]
    fn test_prepare_store_empty_data() {
        let client = BulletinClient::new();
        let result = client.prepare_store(vec![], StoreOptions::default());

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::EmptyData => {} // Expected
            e => panic!("Expected EmptyData, got: {:?}", e),
        }
    }

    #[test]
    fn test_cid_calculation() {
        let client = BulletinClient::new();
        let data = b"Test data".to_vec();

        let operation = client.prepare_store(data, StoreOptions::default()).unwrap();
        let cid = operation.calculate_cid().unwrap();

        // CID is deterministic — same data always produces the same CID
        assert!(!cid.to_bytes().unwrap().is_empty());
    }
}
```

## Testing Chunked Operations

Test chunking logic without submitting to a chain:

```rust
#[cfg(test)]
mod tests {
    use bulletin_sdk_rust::prelude::*;

    #[test]
    fn test_chunked_preparation() {
        let client = BulletinClient::new();
        let data = vec![0xABu8; 5 * 1024 * 1024]; // 5 MiB

        let config = ChunkerConfig {
            chunk_size: 1024 * 1024,  // 1 MiB
            max_parallel: 4,
            create_manifest: true,
        };

        let (batch, manifest) = client
            .prepare_store_chunked(&data, Some(config), StoreOptions::default(), None)
            .unwrap();

        assert_eq!(batch.len(), 5);  // 5 chunks
        assert!(manifest.is_some()); // Manifest created
    }

    #[test]
    fn test_chunked_without_manifest() {
        let client = BulletinClient::new();
        let data = vec![0u8; 3 * 1024 * 1024]; // 3 MiB

        let config = ChunkerConfig {
            chunk_size: 1024 * 1024,
            max_parallel: 4,
            create_manifest: false,
        };

        let (batch, manifest) = client
            .prepare_store_chunked(&data, Some(config), StoreOptions::default(), None)
            .unwrap();

        assert_eq!(batch.len(), 3);
        assert!(manifest.is_none());
    }

    #[test]
    fn test_file_too_large() {
        let client = BulletinClient::new();
        let data = vec![0u8; 65 * 1024 * 1024]; // 65 MiB > MAX_FILE_SIZE

        let result = client.prepare_store_chunked(
            &data,
            None,
            StoreOptions::default(),
            None,
        );

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::FileTooLarge(_) => {} // Expected
            e => panic!("Expected FileTooLarge, got: {:?}", e),
        }
    }
}
```

## Testing Authorization Logic

Test authorization estimation and validation:

```rust
#[cfg(test)]
mod tests {
    use bulletin_sdk_rust::prelude::*;

    #[test]
    fn test_estimate_authorization() {
        let client = BulletinClient::new();

        // 10 MiB file with manifest
        let (txs, bytes) = client.estimate_authorization(10 * 1024 * 1024);

        assert!(txs > 0);
        assert!(bytes >= 10 * 1024 * 1024);
    }

    #[test]
    fn test_authorization_check() {
        let manager = AuthorizationManager::new();

        let sufficient_auth = Authorization {
            scope: AuthorizationScope::Account,
            transactions: 10,
            max_size: 10 * 1024 * 1024,
            expires_at: None,
        };

        // Should pass — enough authorization
        assert!(manager.check_authorization(&sufficient_auth, 1024, 1).is_ok());

        // Should fail — not enough bytes
        let result = manager.check_authorization(&sufficient_auth, 100 * 1024 * 1024, 1);
        assert!(matches!(result, Err(Error::InsufficientAuthorization { .. })));
    }
}
```

## Testing Renewal Operations

```rust
#[cfg(test)]
mod tests {
    use bulletin_sdk_rust::prelude::*;

    #[test]
    fn test_prepare_renewal() {
        let client = BulletinClient::new();
        let storage_ref = StorageRef::new(100, 5);

        let renewal = client.prepare_renew(storage_ref).unwrap();
        assert_eq!(renewal.block(), 100);
        assert_eq!(renewal.index(), 5);
    }

    #[test]
    fn test_renewal_tracker() {
        let mut tracker = RenewalTracker::new();

        tracker.track(
            StorageRef::new(100, 0),
            vec![1, 2, 3],
            1024,
            1000, // retention period
        );

        // Entry expires at block 1100
        let expiring = tracker.expiring_before(1050);
        assert_eq!(expiring.len(), 0); // Not yet expiring

        let expiring = tracker.expiring_before(1200);
        assert_eq!(expiring.len(), 1); // Now expiring
    }
}
```

## Testing Error Metadata

Verify error codes, retryability, and recovery hints:

```rust
#[cfg(test)]
mod tests {
    use bulletin_sdk_rust::prelude::*;

    #[test]
    fn test_error_codes() {
        let err = Error::EmptyData;
        assert_eq!(err.code(), "EMPTY_DATA");
        assert!(!err.is_retryable());
        assert_eq!(err.recovery_hint(), "Provide non-empty data");
    }

    #[test]
    fn test_retryable_errors() {
        let network_err = Error::NetworkError("timeout".into());
        assert!(network_err.is_retryable());
        assert_eq!(network_err.code(), "NETWORK_ERROR");

        let empty_err = Error::EmptyData;
        assert!(!empty_err.is_retryable());
    }

    #[test]
    fn test_insufficient_auth_error() {
        let err = Error::InsufficientAuthorization {
            need: 10_000_000,
            available: 1_000_000,
        };
        assert_eq!(err.code(), "INSUFFICIENT_AUTHORIZATION");
        assert!(!err.is_retryable());
    }
}
```

## Integration Testing

For integration tests that require actual blockchain interaction, use `TransactionClient` with a local test node:

```rust
#[cfg(test)]
mod integration_tests {
    use bulletin_sdk_rust::prelude::*;
    use subxt_signer::sr25519::Keypair;
    use std::str::FromStr;

    // Run with: cargo test --features std -- --ignored
    #[tokio::test]
    #[ignore] // Requires a running local node
    async fn test_store_and_retrieve() {
        let client = TransactionClient::new("ws://localhost:10000")
            .await
            .expect("Failed to connect to local node");

        let uri = subxt_signer::SecretUri::from_str("//Alice").unwrap();
        let signer = Keypair::from_uri(&uri).unwrap();
        let account = subxt::utils::AccountId32::from(signer.public_key().0);

        // Authorize
        client.authorize_account(account, 10, 10 * 1024 * 1024, &signer)
            .await
            .expect("Authorization failed");

        // Store
        let data = b"Integration test data".to_vec();
        let receipt = client.store(data, &signer)
            .await
            .expect("Store failed");

        assert!(!receipt.block_hash.is_empty());
    }
}
```

## Best Practices

1. **Use `BulletinClient` for unit tests** — no network needed, fast, deterministic
2. **Test error paths** — verify your code handles `EmptyData`, `FileTooLarge`, `InsufficientAuthorization`, etc.
3. **Test CID determinism** — same data with same options should always produce the same CID
4. **Mark integration tests with `#[ignore]`** — they require a running node and should not run in CI by default
5. **Test chunking edge cases** — files that don't divide evenly, single-chunk files, maximum file size
