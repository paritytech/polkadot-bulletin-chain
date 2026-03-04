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

# Check out a known-working revision for the relay chain / omni-node binaries.
# NOTE: this does NOT have to match the Cargo.toml SDK revision — the runtime is
# compiled separately. This only affects the host binaries (polkadot, omni-node).
POLKADOT_SDK_REV="81a3af9830ea8b6ff64b066b73b04bb3b675add5"

# Skip rebuild if already at the correct revision with all binaries present
CURRENT_REV=$(git rev-parse HEAD 2>/dev/null || echo "")
if [ "$CURRENT_REV" = "$POLKADOT_SDK_REV" ] \
    && [ -x "$BIN_DIR/polkadot" ] \
    && [ -x "$BIN_DIR/polkadot-prepare-worker" ] \
    && [ -x "$BIN_DIR/polkadot-execute-worker" ] \
    && [ -x "$BIN_DIR/polkadot-omni-node" ] \
    && [ -x "$BIN_DIR/chain-spec-builder" ]; then
    echo "   ✅ Already at correct revision ($POLKADOT_SDK_REV) with binaries present. Skipping build."
    exit 0
fi

echo "   Fetching latest changes from polkadot-sdk..."
git fetch origin
echo "   Checking out polkadot-sdk revision: $POLKADOT_SDK_REV..."
git checkout "$POLKADOT_SDK_REV"

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
