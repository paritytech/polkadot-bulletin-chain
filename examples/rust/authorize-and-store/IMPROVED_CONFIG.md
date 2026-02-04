# Improved Subxt Configuration Approach

## Current Issue

Lines 67-92 manually define all signed extensions, which is brittle and requires maintenance whenever Substrate/Polkadot changes:

```rust
impl Config for BulletinConfig {
    // ... manually list all 8+ signed extensions ...
    type ExtrinsicParams = signed_extensions::AnyOf<
        Self,
        (
            signed_extensions::CheckSpecVersion,
            signed_extensions::CheckTxVersion,
            signed_extensions::CheckNonce,
            // ... etc
        ),
    >;
}
```

## Better Approach

Use `PolkadotConfig` as base which auto-discovers extensions from metadata, and only add our custom extension:

```rust
use subxt::{
    config::{
        PolkadotConfig,
        signed_extensions::{self, SignedExtension},
    },
    OnlineClient,
};

// Generate types from metadata
#[subxt::subxt(runtime_metadata_path = "bulletin_metadata.scale")]
pub mod bulletin {}

// Custom signed extension for ProvideCidConfig (Bulletin-specific)
pub struct ProvideCidConfigExt;

impl<T: Config> SignedExtension<T> for ProvideCidConfigExt {
    type Decoded = ();

    fn matches(identifier: &str, _type_id: u32, _types: &PortableRegistry) -> bool {
        identifier == "ProvideCidConfig"
    }
}

impl<T: Config> ExtrinsicParams<T> for ProvideCidConfigExt {
    type Params = ();

    fn new(_client: &ClientState<T>, _params: Self::Params) -> Result<Self, ExtrinsicParamsError> {
        Ok(ProvideCidConfigExt)
    }
}

impl ExtrinsicParamsEncoder for ProvideCidConfigExt {
    fn encode_extra_to(&self, v: &mut Vec<u8>) {
        Option::<()>::None.encode_to(v);
    }

    fn encode_additional_to(&self, _v: &mut Vec<u8>) {}
}

// Use PolkadotConfig directly - it auto-discovers standard extensions from metadata
type BulletinConfig = PolkadotConfig;

// Connect to node
let api = OnlineClient::<BulletinConfig>::from_url(&ws_url).await?;

// For transactions that need custom extension, use custom params
// For regular transactions, use default params (auto-discovered from metadata)
api.tx()
    .sign_and_submit_default(&store_tx, &signer)
    .await?;
```

## Benefits

1. **Auto-discovery**: Standard signed extensions are read from metadata automatically
2. **Less brittle**: No manual maintenance when Substrate updates
3. **Cleaner code**: Only define what's actually custom to Bulletin Chain
4. **Better compatibility**: Works with any Substrate chain that follows standard patterns

## Alternative: Runtime Metadata Params

Even better, use subxt's `ExtrinsicParams::new()` which reads ALL extensions from metadata:

```rust
use subxt::config::DefaultExtrinsicParamsBuilder;

// Let subxt discover ALL extensions from metadata
let params = DefaultExtrinsicParamsBuilder::new().build();

api.tx()
    .sign_and_submit(&tx, &signer, params)
    .await?;
```

## Handling ProvideCidConfig

For the custom `ProvideCidConfig` extension, we have options:

### Option 1: Default to None (current approach)
Always encode `Option::None` for the CID config, which works for most calls.

### Option 2: Make it configurable
Allow setting CID config when needed:

```rust
pub struct ProvideCidConfigExt {
    config: Option<CidConfig>,
}

impl ExtrinsicParamsEncoder for ProvideCidConfigExt {
    fn encode_extra_to(&self, v: &mut Vec<u8>) {
        self.config.encode_to(v);
    }
}
```

### Option 3: Use metadata hints
Subxt can detect when an extension is needed based on the call signature.

## Recommendation

For this example, the simplest improvement is:
1. Use `PolkadotConfig` instead of custom `BulletinConfig`
2. Only implement `ProvideCidConfigExt` for the custom extension
3. Use `.sign_and_submit_default()` for standard calls
4. Use custom params builder only when CID config is actually needed

This makes the example cleaner and more maintainable while still demonstrating how to handle custom signed extensions.
