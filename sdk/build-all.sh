#!/bin/bash
set -e

echo "🔨 Building Bulletin SDK Libraries"
echo ""

# Build Rust SDK
echo "📦 Building Rust SDK..."
cd rust
cargo build --release --all-features
echo "✅ Rust SDK built successfully"
echo "   Location: target/release/libbulletin_sdk_rust.rlib"
echo ""

# Build TypeScript SDK
echo "📦 Building TypeScript SDK..."
cd ../typescript
npm install
npm run build
echo "✅ TypeScript SDK built successfully"
echo "   Location: dist/"
echo ""

echo "🎉 All SDK libraries built successfully!"
