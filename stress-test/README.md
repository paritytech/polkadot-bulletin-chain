# Bulletin Chain Stress Test

CLI tool and integration test suite for benchmarking write throughput and Bitswap read performance of the Bulletin Chain.

## Prerequisites

### Rust Toolchain

The project requires a nightly Rust toolchain for WASM compilation and formatting:

```bash
# Install Rust via rustup (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add nightly toolchain (needed for cargo fmt and WASM builds)
rustup toolchain install nightly
rustup component add rustfmt --toolchain nightly

# Add WASM target (needed for runtime builds)
rustup target add wasm32-unknown-unknown --toolchain nightly
```

### Build the Stress Test Binary

```bash
cargo build --release -p bulletin-stress-test
```

The binary is at `./target/release/bulletin-stress-test`.

### Parachain Binaries (for zombienet tests)

Parachain tests require two external binaries from the Polkadot SDK. These must be compatible with the SDK revision pinned in this repo's `Cargo.toml` (`f9df3236...`).

**Option A: Download from Polkadot SDK releases**

Download `polkadot` and `polkadot-parachain` from the [polkadot-sdk releases](https://github.com/paritytech/polkadot-sdk/releases) page. Pick a release whose tag matches (or is close to) the pinned revision.

```bash
# Example: download and make executable
chmod +x polkadot polkadot-parachain
```

**Option B: Build from source**

```bash
git clone https://github.com/paritytech/polkadot-sdk.git
cd polkadot-sdk
git checkout f9df32360c6c22747f60b3ff58243890eecafc8e

# Build the relay chain validator
cargo build --release -p polkadot

# Build the parachain omni-node (collator)
cargo build --release -p polkadot-parachain-bin
```

The binaries will be at `target/release/polkadot` and `target/release/polkadot-parachain`.

**Automated setup**: A script is provided that clones polkadot-sdk, builds the required binaries, and installs them to `~/local_bulletin_testing/bin/`:

```bash
./scripts/setup_parachain_prerequisites.sh
```

This builds `polkadot`, `polkadot-omni-node`, `polkadot-prepare-worker`, `polkadot-execute-worker`, and `chain-spec-builder`. It skips the build if the binaries are already present at the correct revision. On macOS it automatically sets `DYLD_FALLBACK_LIBRARY_PATH` for libclang.

Then set the env vars:

```bash
export POLKADOT_RELAY_BINARY_PATH=~/local_bulletin_testing/bin/polkadot
export POLKADOT_PARACHAIN_BINARY_PATH=~/local_bulletin_testing/bin/polkadot-omni-node
```

### Parachain Chain Spec

The chain spec at `zombienet/bulletin-westend-spec.json` (Para ID 2487, westend-local relay) embeds the runtime WASM blob. **You must regenerate it after any runtime changes**, otherwise zombienet tests will run with a stale runtime:

```bash
./scripts/create_bulletin_westend_spec.sh
```

This builds the `bulletin-westend-runtime` and embeds the fresh WASM into the spec. Requires `chain-spec-builder` on PATH (built by `./scripts/setup_parachain_prerequisites.sh`).

### zombienet-sdk

The zombienet tests use `zombienet-sdk` as a Rust dependency (not the standalone zombienet CLI binary). No separate installation is needed — `cargo test` fetches it automatically.

The only requirement is setting `ZOMBIE_PROVIDER=native` to use native process spawning instead of Docker.

## CLI Usage

```bash
bulletin-stress-test [OPTIONS] <COMMAND>
```

### Global Options

| Option | Default | Description |
|--------|---------|-------------|
| `--ws-url <URL>` | `ws://127.0.0.1:9944` | WebSocket URL(s), comma-separated for multi-node submission (e.g. `ws://rpc1:9944,ws://rpc2:9955`). First URL is used for control (authorization, monitoring); all are used for submission. |
| `--p2p-multiaddr <ADDR>` | auto-discovered | Node's P2P multiaddr for Bitswap (discovered via RPC if omitted) |
| `--authorizer-seed <SEED>` | `//Alice` | Seed for the authorizer account (must be in runtime's Authorizer origin) |
| `--iterations <N>` | `512` | Number of unique items for Bitswap read tests |
| `--submitters <N>` | `4` | WebSocket RPC connections for store submission (one worker per connection). Actual count is `max(N, 8)`. Increase for remote RPCs. |
| `--target-blocks <N>` | `5` | Number of measured blocks with transactions per variant. The monitor counts best blocks and stops when the target is reached. |
| `--iteration-blocks <N>` | `20` | Measured blocks worth of transactions per pipeline iteration (chunk size) |
| `--mix-seed <N>` | OS entropy | Seed for random payload-size draws in MIXED mode (reproducible runs) |
| `--output <FORMAT>` | `text` | Output format: `text` or `json` |
| `--output-file <PATH>` | none | JSON output file, flushed after every variant so partial results survive crashes |

### Commands

#### `throughput [block-capacity]`

Measures write throughput by filling blocks with storage transactions across payload sizes.

```bash
# Run all fixed payload sizes against a local dev node
./target/release/bulletin-stress-test --ws-url ws://127.0.0.1:9944 throughput

# Run specific variants only
./target/release/bulletin-stress-test throughput --variants "1KB,128KB,1MB"

# Run weighted mixed-size workload (real-world distribution)
./target/release/bulletin-stress-test throughput --variants MIXED

# Reproducible mixed run
./target/release/bulletin-stress-test throughput --variants MIXED --mix-seed 42
```

**Fixed variants**: 1KB, 4KB, 32KB, 128KB, 512KB, 1MB, 2MB.

**MIXED mode**: weighted distribution simulating real-world traffic (1KB 23%, 4KB 15%, 32KB 12%, 128KB 17.5%, 512KB 15.5%, 1MB 9%, 2MB 8%). Each account draws a random payload size from this distribution.

For each variant, the tool:
1. Queries chain limits (block weight, length, max transactions) to calculate capacity
2. Creates unique one-shot accounts (fresh derivation per run to avoid nonce collisions)
3. Signs store extrinsics with look-ahead (batch N+1 signed concurrently while batch N is dispatched)
4. Authorizes accounts on-chain, then submits via fire-and-forget RPC (`author_submitExtrinsic`)
5. Monitors blocks via events (lightweight, no block body fetch) and tracks per-block bytes via content hash mapping
6. Stops after `--target-blocks` measured blocks with transactions
7. Waits for finalization of measured blocks before computing results
8. Drains the transaction pool before proceeding to the next variant

The `--variants` flag accepts a comma-separated list of size labels (e.g. `"1KB,128KB,1MB"`) or `MIXED`.

#### `bitswap [b2]`

Measures Bitswap (IPFS) read performance by sweeping concurrency levels (1, 2, 4, 8, 16, 32, 64 simultaneous clients).

```bash
# Run with 128 items (128 x 128KB = 16MB stored)
./target/release/bulletin-stress-test --iterations 128 bitswap

# Smaller run for quick smoke test
./target/release/bulletin-stress-test --iterations 32 bitswap
```

Stores `--iterations` unique 128KB items on-chain, then creates N independent Bitswap clients at each concurrency level. Each client reads all items sequentially while all N clients run in parallel. Reports aggregate reads/s, latency percentiles, and data integrity verification per level.

#### `full`

Runs all test suites sequentially (throughput + bitswap).

```bash
./target/release/bulletin-stress-test --output json full > results.json

# With crash-safe incremental output
./target/release/bulletin-stress-test --output-file results.json full
```

## Running Against Remote Endpoints

The stress test can target any live Bulletin Chain node via its public WebSocket endpoint. The authorizer seed must correspond to an account in the runtime's Authorizer origin on that chain.

### Single-Node (e.g. Westend Bulletin)

```bash
./target/release/bulletin-stress-test \
  --ws-url 'wss://westend-bulletin-rpc.polkadot.io' \
  --authorizer-seed '//Alice' \
  --submitters 16 \
  --target-blocks 100 \
  --output-file westend-results.json \
  throughput --variants "32KB"
```

Key considerations for remote endpoints:
- **Increase `--submitters`**: Remote RPCs have higher latency; use 8-32 submitters to keep the pool saturated. Workers use fire-and-forget RPC for maximum throughput.
- **Increase `--target-blocks`**: More blocks give more statistically significant results. The tool automatically calculates accounts needed.
- **Use `--output-file`**: Results are flushed after each variant, so partial results survive crashes or network interruptions.
- **Ctrl+C**: First signal stops gracefully (finishes current variant, prints results). Second signal force-exits.
- **Bitswap tests require P2P access**: The `bitswap` command needs a direct P2P connection to the node, which is usually not available through public RPC endpoints.

### Multi-Node Submission

For parachains or multi-validator networks, submit to multiple RPC endpoints to increase throughput:

```bash
./target/release/bulletin-stress-test \
  --ws-url "wss://rpc1.example.com,wss://rpc2.example.com" \
  --submitters 16 \
  --target-blocks 200 \
  --output-file results.json \
  throughput
```

The first URL is used for control operations (authorization, block monitoring); all URLs are used for transaction submission. Submitters are distributed round-robin across URLs.

## Zombienet Integration Tests

Automated tests that spawn ephemeral local networks, run the stress test CLI as a subprocess, and validate results against expectations tables.

### Prerequisites Checklist

Before running zombienet tests, ensure you have:

1. **Stress test binary** — built automatically by `cargo test`
2. **Parachain binaries**: `polkadot` + `polkadot-parachain` (see [Parachain Binaries](#parachain-binaries-for-zombienet-tests)) and the chain spec at `zombienet/bulletin-westend-spec.json`
3. **Environment variable**: `ZOMBIE_PROVIDER=native` (required for all tests)

### Environment Variables

**Required for all tests:**

| Variable | Description |
|----------|-------------|
| `ZOMBIE_PROVIDER=native` | Use native process spawning (not Docker). **Required.** |
| `RUST_LOG=info` | Recommended. Required to see CLI subprocess progress logs during test execution. |

**Parachain tests:**

| Variable | Default | Description |
|----------|---------|-------------|
| `POLKADOT_RELAY_BINARY_PATH` | `polkadot` (on PATH) | Path to the relay chain validator binary |
| `POLKADOT_PARACHAIN_BINARY_PATH` | `polkadot-parachain` (on PATH) | Path to the parachain omni-node binary |
| `PARACHAIN_CHAIN_SPEC_PATH` | `./zombienet/bulletin-westend-spec.json` | Path to the parachain chain spec |
| `RELAY_CHAIN` | `westend-local` | Relay chain identifier |
| `PARACHAIN_ID` | `2487` | Parachain ID |

### Running Tests

Always use `--test-threads=1` — tests share a single zombienet network and must run sequentially.

```bash
# Set common env vars (adjust paths to your binaries)
export ZOMBIE_PROVIDER=native
export RUST_LOG=info
export POLKADOT_RELAY_BINARY_PATH=/path/to/polkadot
export POLKADOT_PARACHAIN_BINARY_PATH=/path/to/polkadot-parachain

# Run ALL tests (throughput variants + bitswap)
cargo test -p bulletin-stress-test --test zombienet \
  -- --nocapture --test-threads=1

# Run all throughput variants
cargo test -p bulletin-stress-test --test zombienet test_parachain_throughput \
  -- --nocapture --test-threads=1

# Run a single throughput variant (e.g. 32KB)
cargo test -p bulletin-stress-test --test zombienet test_parachain_throughput_32kb \
  -- --nocapture --test-threads=1

# Bitswap concurrent read
cargo test -p bulletin-stress-test --test zombienet test_parachain_bitswap_read \
  -- --nocapture --test-threads=1
```

All throughput variant tests share a single zombienet network (spawned on first use). The network stays alive across tests within the same `cargo test` invocation.

### Test Matrix

#### Throughput (one test per payload size)

| Test | Variant | Expected |
|------|---------|----------|
| `test_parachain_throughput_1kb` | 1KB | success |
| `test_parachain_throughput_4kb` | 4KB | success |
| `test_parachain_throughput_32kb` | 32KB | success |
| `test_parachain_throughput_128kb` | 128KB | success |
| `test_parachain_throughput_512kb` | 512KB | success |
| `test_parachain_throughput_1mb` | 1MB | success |
| `test_parachain_throughput_2mb` | 2MB | success |
| `test_parachain_throughput_2050kb` | 2050KB | success |
| `test_parachain_throughput_4mb` | 4MB | rejection (WASM OOM) |
| `test_parachain_throughput_5mb` | 5MB | rejection |
| `test_parachain_throughput_7mb` | 7MB | rejection |
| `test_parachain_throughput_7_5mb` | 7.5MB | rejection |
| `test_parachain_throughput_8mb` | 8MB | rejection |
| `test_parachain_throughput_10mb` | 10MB | rejection |

#### Bitswap

| Test | What it validates |
|------|-------------------|
| `test_parachain_bitswap_read` | All 7 concurrency levels (1..64): reads > 0, failures = 0, data verified |

### Expected Results

The zombienet tests validate results against per-variant expectations tables:

Payload sizes up to 2050KB expected to succeed. 4MB+ may OOM during WASM block import on non-authoring nodes (the WASM freeing-bump allocator has a 16MB heap limit; chunking in `do_store` requires ~2x the payload size in concurrent allocations).

### Network Topology

Parachain (6 nodes):
```
  2 relay validators (alice, bob)  -- westend-local
         |                |
   collator-1      collator-2     -- 2 collators, both author blocks
         | gossip        | gossip
      rpc-1           rpc-2       -- 2 full nodes (non-collating)
         | ws://         | ws://
            stress-test           -- splits submitters across RPC nodes
```
- 2 relay validators run `westend-local`
- 2 collators with `--ipfs-server`, `--pool-kbytes=65536` (64MB tx pool)
- 2 RPC full nodes (non-collating) with 64MB tx pool
- Embedded relay chain forced to libp2p backend (`--network-backend=libp2p`)
- Stress test submits to RPC nodes; transactions propagate via gossip to collators
- Waits for relay session change (~20 blocks) before parachain starts collating

## Output

### Text Output

Prints a summary table per scenario with throughput, latency percentiles, and block statistics. When multiple variants are run, a combined summary table is printed at the end.

### JSON Output

Returns an array of `ScenarioResult` objects. Each contains:

- `name`: Scenario identifier (e.g., `"block-cap: Block Capacity (1200 accounts)"`)
- `duration`: Measurement window duration
- `payload_size`: Bytes per item
- `throughput_tps`, `throughput_bytes_per_sec`: Write metrics (throughput tests)
- `avg_tx_per_block`, `peak_tx_per_block`: Block utilization (throughput tests)
- `avg_block_interval_ms`: Average block interval from on-chain timestamps (throughput tests)
- `onchain_timing`: Whether throughput was computed from on-chain timestamps
- `fork_detections`: Number of chain forks detected during the test
- `total_submitted`, `total_confirmed`, `total_errors`: Transaction counts
- `total_reads`, `successful_reads`, `failed_reads`: Read counts (bitswap tests)
- `reads_per_sec`, `read_bytes_per_sec`: Read throughput (bitswap tests)
- `data_verified`: Whether all fetched data matched expected content
- `inclusion_latency`, `finalization_latency`, `retrieval_latency`: p50/p95/p99/max latency stats
- `theoretical`: Computed capacity limits (weight, length, count caps and bottleneck)
- `submission_stats`: Detailed tx pool stats (stale nonces, pool-full retries, errors)
- `blocks`: Per-block data (number, tx count, payload bytes, prefill flag, on-chain timestamp, block hash, finalized status, block interval)
- `chain_limits`: Runtime constants (block weight/length limits, storage limits, weight regression)
- `environment`: Chain name, version, node role

## Architecture

See [DESIGN.md](DESIGN.md) for detailed architecture, data flow diagrams, and module dependency graph.
