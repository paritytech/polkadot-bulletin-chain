# Polkadot Bulletin Chain - Just Commands
# Install just: brew install just
# Run `just --list` to see all available commands

# Default recipe - run the complete PAPI workflow test
default: test-papi-flow

# Build the bulletin chain node in release mode
build:
    cargo build --release -p polkadot-bulletin-chain

# Install JavaScript dependencies
npm-install:
    npm install

# Clean build artifacts
clean:
    cargo clean

# Test the complete PAPI workflow (builds, starts services, runs example)
test-papi-flow: build npm-install
    #!/usr/bin/env bash
    set -e
    
    echo "🚀 Starting complete PAPI workflow test..."
    echo ""
    
    # Check if IPFS is available
    if ! command -v ipfs &> /dev/null; then
        echo "❌ IPFS not found. Using local kubo binary..."
        IPFS_CMD="./kubo/ipfs"
        if [ ! -f "$IPFS_CMD" ]; then
            echo "❌ Error: Neither system IPFS nor ./kubo/ipfs found."
            echo "Please install IPFS or download kubo to ./kubo/"
            exit 1
        fi
    else
        IPFS_CMD="ipfs"
    fi
    
    # Initialize IPFS if needed
    if [ ! -d ~/.ipfs ]; then
        echo "📦 Initializing IPFS..."
        $IPFS_CMD init
    fi
    
    # Start IPFS daemon in background
    echo "📡 Starting IPFS daemon..."
    $IPFS_CMD daemon > /tmp/ipfs-daemon.log 2>&1 &
    IPFS_PID=$!
    echo "   IPFS PID: $IPFS_PID"
    sleep 3
    
    # Start zombienet in background
    echo "⚡ Starting Bulletin chain with zombienet..."
    POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain ./zombienet-macos-arm64 -p native spawn ./zombienet/bulletin-polkadot-local.toml > /tmp/zombienet.log 2>&1 &
    ZOMBIENET_PID=$!
    echo "   Zombienet PID: $ZOMBIENET_PID"
    echo "   Waiting for chain to start (15 seconds)..."
    sleep 15
    
    # Start IPFS reconnect script in background
    echo "🔄 Starting IPFS reconnect script..."
    ./scripts/ipfs-reconnect-solo.sh > /tmp/ipfs-reconnect.log 2>&1 &
    RECONNECT_PID=$!
    echo "   Reconnect PID: $RECONNECT_PID"
    sleep 2
    
    # Generate PAPI descriptors
    echo "🔧 Generating PAPI descriptors..."
    npm run papi:generate
    
    # Run the example
    echo ""
    echo "🎯 Running authorize_and_store_papi.js example..."
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    cd examples && node authorize_and_store_papi.js
    EXAMPLE_EXIT=$?
    
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    if [ $EXAMPLE_EXIT -eq 0 ]; then
        echo "✅ Example completed successfully!"
    else
        echo "❌ Example failed with exit code $EXAMPLE_EXIT"
    fi
    
    echo ""
    echo "📋 Background processes:"
    echo "   IPFS daemon:     PID $IPFS_PID"
    echo "   Zombienet:       PID $ZOMBIENET_PID"
    echo "   IPFS reconnect:  PID $RECONNECT_PID"
    echo ""
    echo "🧹 To cleanup, run:"
    echo "   kill $IPFS_PID $ZOMBIENET_PID $RECONNECT_PID"
    echo "   or: just cleanup"
    echo ""
    echo "📝 Logs available at:"
    echo "   /tmp/ipfs-daemon.log"
    echo "   /tmp/zombienet.log"
    echo "   /tmp/ipfs-reconnect.log"
    
    # Save PIDs to file for cleanup
    echo "$IPFS_PID $ZOMBIENET_PID $RECONNECT_PID" > /tmp/bulletin-test-pids.txt
    
    exit $EXAMPLE_EXIT

# Cleanup background processes from test-papi-flow
cleanup:
    #!/usr/bin/env bash
    if [ -f /tmp/bulletin-test-pids.txt ]; then
        PIDS=$(cat /tmp/bulletin-test-pids.txt)
        echo "🧹 Stopping background processes: $PIDS"
        kill $PIDS 2>/dev/null || true
        rm /tmp/bulletin-test-pids.txt
        echo "✅ Cleanup complete!"
    else
        echo "⚠️  No saved PIDs found. Processes may still be running."
        echo "Check with: ps aux | grep -E 'ipfs|zombienet|ipfs-reconnect'"
    fi
