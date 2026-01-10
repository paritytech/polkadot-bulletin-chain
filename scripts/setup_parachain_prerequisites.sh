#!/usr/bin/env bash
set -e

BIN_DIR=~/local_bulletin_testing/bin
POLKADOT_SDK_DIR=~/local_bulletin_testing/polkadot-sdk

# Common setup issue on macOS is that libclang.dylib is not found.
if [[ "$OSTYPE" == "darwin"* ]]; then
  export DYLD_FALLBACK_LIBRARY_PATH="$(brew --prefix llvm)/lib"
fi

echo "🔧 Setting up parachain prerequisites..."
echo "   Target directory: $BIN_DIR"

# Create bin directory
mkdir -p $BIN_DIR

# Clone polkadot-sdk if it doesn't exist
if [ ! -d "$POLKADOT_SDK_DIR" ]; then
    echo "   Cloning polkadot-sdk repository..."
    git clone https://github.com/paritytech/polkadot-sdk.git $POLKADOT_SDK_DIR
else
    echo "   polkadot-sdk already exists at $POLKADOT_SDK_DIR"
fi

cd $POLKADOT_SDK_DIR

# Check out latest master
echo "   Fetching latest changes from polkadot-sdk..."
git fetch origin
echo "   Checking out latest master..."
# TODO:
# git reset --hard origin/master
# Let's use the same commit as Cargo.toml to avoid moving Polkadot-SDK
git reset --hard b2bcb74b13f1a1e082f701e3e05ce1be44d16790

# Build polkadot binary
echo "   Building polkadot binary (this may take a while)..."
cargo build -p polkadot -r --features fast-runtime

# Verify and copy polkadot binaries
echo "   Copying polkadot binaries..."
ls -la target/release/polkadot
cp target/release/polkadot $BIN_DIR/
cp target/release/polkadot-prepare-worker $BIN_DIR/
cp target/release/polkadot-execute-worker $BIN_DIR/

# Verify polkadot version (optional check, may fail on macOS due to security/signing)
echo "   Verifying polkadot version..."
$BIN_DIR/polkadot --version || echo "   ⚠ Version check failed (this is OK, binary will still work)"

# Build polkadot-omni-node binary
echo "   Building polkadot-omni-node binary (this may take a while)..."
cargo build -p polkadot-omni-node -r

# Verify and copy polkadot-omni-node binary
echo "   Copying polkadot-omni-node binary..."
ls -la target/release/polkadot-omni-node
cp target/release/polkadot-omni-node $BIN_DIR/

# Verify polkadot-omni-node version (optional check, may fail on macOS due to security/signing)
echo "   Verifying polkadot-omni-node version..."
$BIN_DIR/polkadot-omni-node --version || echo "   ⚠ Version check failed (this is OK, binary will still work)"

# Build and install chain-spec-builder
echo "   Building chain-spec-builder..."
cargo build -p staging-chain-spec-builder -r

# Verify and copy chain-spec-builder binary
echo "   Copying chain-spec-builder binary..."
ls -la target/release/chain-spec-builder
cp target/release/chain-spec-builder $BIN_DIR/

# Verify chain-spec-builder (optional check, may fail on macOS due to security/signing)
echo "   Verifying chain-spec-builder version..."
$BIN_DIR/chain-spec-builder --version || echo "   ⚠ Version check failed (this is OK, binary will still work)"

# Add BIN_DIR to PATH for subsequent scripts
export PATH="$BIN_DIR:$PATH"

echo "✅ Parachain prerequisites setup complete!"
echo "   Binaries installed in: $BIN_DIR"
