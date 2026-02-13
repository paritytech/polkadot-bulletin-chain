# Bulletin Chain SDK

Multi-language client SDKs for Polkadot Bulletin Chain.

| Language | Package | Path |
|----------|---------|------|
| Rust | `bulletin-sdk-rust` | [rust/](rust/) |
| TypeScript | `@bulletin/sdk` | [typescript/](typescript/) |

## Build & Test

```bash
# Build all
cd sdk && ./build-all.sh

# Or individually:
cd sdk/rust && cargo build --release --all-features
cd sdk/typescript && npm install && npm run build

# Tests
cd sdk/rust && cargo test --lib --all-features
cd sdk/typescript && npm run test:unit
```

## Documentation

Full SDK documentation: [`docs/sdk-book`](../docs/sdk-book/)

## Release

See [RELEASE_AUTOMATION_SUMMARY.md](RELEASE_AUTOMATION_SUMMARY.md) for publishing to crates.io and npm.

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
