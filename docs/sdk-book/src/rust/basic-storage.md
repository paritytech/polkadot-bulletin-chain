# Basic Storage

Store data using `AsyncBulletinClient`.

## Example

```rust
use bulletin_sdk_rust::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ws_url = std::env::var("BULLETIN_WS_URL")
        .unwrap_or_else(|_| "ws://localhost:10000".to_string());
    let signer = /* your PairSigner */;

    let submitter = SubxtSubmitter::from_url(&ws_url, signer).await?;
    let client = AsyncBulletinClient::new(submitter);

    let result = client
        .store(b"Hello, Bulletin!".to_vec())
        .send()
        .await?;

    println!("CID: {:?}, Size: {}", result.cid, result.size);
    Ok(())
}
```

## Builder Options

```rust
// Custom codec and hash
client.store(data)
    .with_codec(CidCodec::DagPb)
    .with_hash_algorithm(HashAlgorithm::Sha256)
    .with_finalization(true)
    .send()
    .await?;

// With progress callback (for chunked uploads)
client.store(large_data)
    .with_callback(|event| {
        tracing::debug!(?event, "Upload progress");
    })
    .send()
    .await?;
```

## Authorization Checking

Set an account on the client to enable automatic pre-flight authorization checking. The SDK queries the blockchain before submitting and fails fast if authorization is insufficient.

```rust
let client = AsyncBulletinClient::new(submitter)
    .with_account(account);

// Fails immediately with Error::InsufficientAuthorization if not authorized
let result = client.store(data).send().await?;
```

To disable (e.g. for offline signing or high-frequency uploads):

```rust
let mut config = AsyncClientConfig::default();
config.check_authorization_before_upload = false;
let client = AsyncBulletinClient::with_config(submitter, config);
```

## Two-Step Approach

For more control, prepare the operation first:

```rust
let client = BulletinClient::new();
let operation = client.prepare_store(data, options)?;
// operation.cid_bytes contains the CID
// operation.data is ready to submit via your own method
```

## Testing

Use `MockSubmitter` for unit tests:

```rust
#[tokio::test]
async fn test_store() {
    let client = AsyncBulletinClient::new(MockSubmitter::new());
    let result = client.store(b"test".to_vec(), None).await;
    assert!(result.is_ok());
}
```
