# Bulletin Chain SDK

Multi-language client SDKs for Polkadot Bulletin Chain with complete transaction submission support.

## Overview

This directory contains off-chain client SDKs that simplify data storage on the Bulletin Chain with automatic chunking, authorization management, and DAG-PB manifest generation.

**Available SDKs:**
- **Rust** (`rust/`) - no_std compatible, works in native apps and ink! smart contracts
- **TypeScript** (`typescript/`) - Browser and Node.js compatible

Both SDKs provide:
- ✅ Complete transaction submission (not just data preparation)
- ✅ All 8 pallet operations
- ✅ Automatic chunking with progress tracking
- ✅ Authorization management
- ✅ DAG-PB manifest generation (IPFS-compatible)

## Quick Start

### Prerequisites

**For Rust SDK:**
- Rust toolchain (stable)
- `cargo` package manager

**For TypeScript SDK:**
- Node.js 18+ or 20+
- `npm`, `yarn`, or `pnpm`

**For Integration Tests (both):**
- Running Bulletin Chain node at `ws://localhost:9944`

```bash
# Start local dev node
cargo build --release
./target/release/polkadot-bulletin-chain --dev --tmp
```

## Build All SDKs

### From Repository Root

```bash
# Build Rust SDK
cd sdk/rust
cargo build --release --all-features

# Build TypeScript SDK
cd ../typescript
npm install
npm run build
```

### Using Workspace (Rust SDK only)

The Rust SDK is part of the workspace, so from the repository root:

```bash
# Build entire workspace (includes SDK)
cargo build --release

# Build just the SDK
cargo build -p bulletin-sdk-rust --release
```

## Test All SDKs

### Rust SDK Tests

```bash
cd sdk/rust

# Run all unit tests
cargo test --lib --all-features

# Run all tests including integration tests (requires running node)
cargo test --all-features -- --ignored --test-threads=1

# Run specific test
cargo test test_simple_store --all-features -- --ignored

# Run tests in no_std mode
cargo test --no-default-features
```

### TypeScript SDK Tests

```bash
cd sdk/typescript

# Install dependencies first
npm install

# Build the SDK
npm run build

# Run all unit tests (no node required)
npm run test:unit

# Run integration tests (requires running node)
npm run test:integration

# Run all tests
npm test

# Run tests with coverage
npm run test:coverage

# Run tests in watch mode
npm run test:watch
```

## Complete Build & Test Script

Here's a complete script to build and test everything:

```bash
#!/bin/bash
set -e

echo "🔨 Building and Testing Bulletin SDK"
echo ""

# Build Rust SDK
echo "📦 Building Rust SDK..."
cd sdk/rust
cargo build --release --all-features
echo "✅ Rust SDK built"
echo ""

# Test Rust SDK (unit tests only, no node required)
echo "🧪 Testing Rust SDK (unit tests)..."
cargo test --lib --all-features
echo "✅ Rust SDK unit tests passed"
echo ""

# Build TypeScript SDK
echo "📦 Building TypeScript SDK..."
cd ../typescript
npm install
npm run build
echo "✅ TypeScript SDK built"
echo ""

# Test TypeScript SDK (unit tests only, no node required)
echo "🧪 Testing TypeScript SDK (unit tests)..."
npm run test:unit
echo "✅ TypeScript SDK unit tests passed"
echo ""

echo "🎉 All SDKs built and unit tests passed!"
echo ""
echo "💡 To run integration tests (requires local node):"
echo "   1. Start node: ./target/release/polkadot-bulletin-chain --dev --tmp"
echo "   2. Run Rust integration tests: cd sdk/rust && cargo test --all-features -- --ignored --test-threads=1"
echo "   3. Run TypeScript integration tests: cd sdk/typescript && npm run test:integration"
```

Save this as `sdk/build-and-test.sh` and run:

```bash
chmod +x sdk/build-and-test.sh
./sdk/build-and-test.sh
```

## Run Examples

### Rust Examples

```bash
cd sdk/rust

# Generate metadata first
cd ../..
./target/release/polkadot-bulletin-chain export-metadata > sdk/rust/artifacts/metadata.scale
cd sdk/rust

# Simple store example
cargo run --example simple_store --features std

# Chunked store example
cargo run --example chunked_store --features std -- large_file.bin

# Authorization management example
cargo run --example authorization_management --features std
```

### TypeScript Examples

```bash
cd sdk/typescript

# Build first
npm run build

# Simple store example
node examples/simple-store.js

# Large file example
node examples/large-file.js large_file.bin

# Complete workflow example
node examples/complete-workflow.js
```

## Integration Tests with Local Node

### 1. Start Local Node

```bash
# Terminal 1: Start the node
cargo build --release
./target/release/polkadot-bulletin-chain --dev --tmp
```

### 2. Run Rust Integration Tests

```bash
# Terminal 2
cd sdk/rust

# Generate metadata
../../target/release/polkadot-bulletin-chain export-metadata > artifacts/metadata.scale

# Run integration tests
cargo test --test integration_tests --features std -- --ignored --test-threads=1
```

### 3. Run TypeScript Integration Tests

```bash
# Terminal 2 (or Terminal 3)
cd sdk/typescript

# Make sure it's built
npm run build

# Run integration tests
npm run test:integration
```

## CI/CD Testing

For continuous integration, use this sequence:

```bash
# 1. Start node in background
./target/release/polkadot-bulletin-chain --dev --tmp &
NODE_PID=$!
sleep 10

# 2. Run all tests
cd sdk/rust
cargo test --all-features -- --ignored --test-threads=1

cd ../typescript
npm install
npm run build
npm run test:integration

# 3. Cleanup
kill $NODE_PID
```

## Documentation

Each SDK has comprehensive documentation:

- **Rust SDK**: [`sdk/rust/README.md`](rust/README.md)
- **TypeScript SDK**: [`sdk/typescript/README.md`](typescript/README.md)
- **Utilities Reference**: [`sdk/UTILITIES.md`](UTILITIES.md)
- **Implementation Details**: [`sdk/IMPLEMENTATION_COMPLETE.md`](IMPLEMENTATION_COMPLETE.md)

## Troubleshooting

### Rust Build Issues

**Problem:** Compilation errors

```bash
# Clean and rebuild
cargo clean
cargo build --release --all-features
```

**Problem:** Test failures

```bash
# Ensure node is running
ps aux | grep polkadot-bulletin-chain

# Check node endpoint
curl -H "Content-Type: application/json" -d '{"id":1, "jsonrpc":"2.0", "method": "system_health"}' http://localhost:9944
```

### TypeScript Build Issues

**Problem:** Module not found

```bash
# Reinstall dependencies
rm -rf node_modules package-lock.json
npm install
npm run build
```

**Problem:** Test timeouts

```bash
# Increase timeout in vitest.config.ts
testTimeout: 60000 // 60 seconds
```

### Integration Test Issues

**Problem:** Connection refused

**Solution:** Ensure the node is running at `ws://localhost:9944`:

```bash
./target/release/polkadot-bulletin-chain --dev --tmp
```

**Problem:** Authorization errors

**Solution:** Integration tests use Alice's account which has sudo. Ensure node is in `--dev` mode.

**Problem:** Metadata not found (Rust tests)

**Solution:** Generate metadata:

```bash
./target/release/polkadot-bulletin-chain export-metadata > sdk/rust/artifacts/metadata.scale
```

## Performance Testing

### Rust

```bash
cd sdk/rust

# Build with optimizations
cargo build --release --all-features

# Run benchmarks (if available)
cargo bench --all-features
```

### TypeScript

```bash
cd sdk/typescript

# Build optimized
npm run build

# Run performance tests (if available)
npm run test:performance
```

## Development Workflow

### Making Changes to Rust SDK

```bash
cd sdk/rust

# Make changes to src/

# Format code
cargo fmt

# Check lints
cargo clippy --all-features -- -D warnings

# Run tests
cargo test --all-features

# Build
cargo build --all-features
```

### Making Changes to TypeScript SDK

```bash
cd sdk/typescript

# Make changes to src/

# Type check
npm run typecheck

# Lint
npm run lint

# Run tests
npm test

# Build
npm run build
```

## Publishing (Future)

When ready to publish:

### Rust SDK

```bash
cd sdk/rust
cargo publish --dry-run
cargo publish
```

### TypeScript SDK

```bash
cd sdk/typescript
npm publish --dry-run
npm publish
```

## License

GPL-3.0-or-later WITH Classpath-exception-2.0
