# Zombienet SDK Tests

Integration tests for Polkadot Bulletin Chain sync modes and transaction storage, using [zombienet-sdk](https://github.com/nicokosi/zombienet-sdk) to spawn local networks.

## Prerequisites

### Solochain tests

Build the bulletin chain node:

```bash
cargo build --release
export POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain
```

### Parachain tests

Parachain tests require Polkadot SDK binaries and a chain spec. There are two ways to set this up:

**Option A: Use the setup script** (builds from source, takes a while):

```bash
# Builds polkadot, polkadot-omni-node, and chain-spec-builder from polkadot-sdk
./scripts/setup_parachain_prerequisites.sh

# Add built binaries to PATH
export PATH=~/local_bulletin_testing/bin:$PATH
```

**Option B: Provide your own binaries** (if you already have compatible builds):

Download or build `polkadot` and `polkadot-omni-node` from [polkadot-sdk](https://github.com/paritytech/polkadot-sdk) at the revision pinned in `Cargo.toml`.

Then set the paths:

```bash
export POLKADOT_RELAY_BINARY_PATH=/path/to/polkadot
export POLKADOT_PARACHAIN_BINARY_PATH=/path/to/polkadot-omni-node
```

**Generate the chain spec** (required for both options):

The parachain chain spec is not checked into git. Generate it with:

```bash
# Requires chain-spec-builder on PATH (installed by setup_parachain_prerequisites.sh
# or via `cargo install staging-chain-spec-builder`)
./scripts/create_bulletin_westend_spec.sh
```

This builds the bulletin-westend runtime WASM, generates a chain spec with `chain-spec-builder`, and places it at `./zombienet/bulletin-westend-spec.json`.

To use a different para ID, set `PARACHAIN_ID` before running:

```bash
PARACHAIN_ID=2000 ./scripts/create_bulletin_westend_spec.sh
```

### LDB tests (optional)

The `ldb_storage_verification_test` tests inspect RocksDB directly and require the `ldb` tool:

```bash
export ROCKSDB_LDB_PATH=/path/to/ldb
```

## Running tests

```bash
# All solochain tests
cargo test -p bulletin-chain-zombienet-sdk-tests solochain_sync_storage -- --nocapture

# All parachain tests
cargo test -p bulletin-chain-zombienet-sdk-tests parachain_sync_storage -- --nocapture

# Single test
cargo test -p bulletin-chain-zombienet-sdk-tests fast_sync_test -- --nocapture
```

Run tests one at a time or with `--test-threads=1`. Each test spawns a full network and they are resource-intensive.

## Test matrix

### Solochain (`solochain_sync_storage`)

| Test | Sync mode | Pruning | Expected outcome |
|---|---|---|---|
| `fast_sync_test` | fast | no | Sync completes, bitswap DONT_HAVE |
| `fast_sync_with_pruning_test` | fast | yes | Sync fails (pruned blocks) |
| `warp_sync_test` | warp | no | Sync completes, bitswap DONT_HAVE (expected) |
| `warp_sync_with_pruning_test` | warp | yes | Sync completes, bitswap DONT_HAVE (expected) |
| `full_sync_test` | full | no | Sync completes, bitswap works |
| `full_sync_with_pruning_test` | full | yes | Sync fails (pruned blocks) |
| `ldb_storage_verification_test` | - | yes | Verifies col11 refcounting and data expiration |

### Parachain (`parachain_sync_storage`)

Same matrix as solochain, prefixed with `parachain_`. Additionally includes `parachain_ldb_storage_verification_test`.

## Environment variables

### Binary paths

| Variable | Description | Default |
|---|---|---|
| `POLKADOT_BULLETIN_BINARY_PATH` | Solochain node binary | `./target/release/polkadot-bulletin-chain` |
| `POLKADOT_RELAY_BINARY_PATH` | Relay chain binary | `polkadot` |
| `POLKADOT_PARACHAIN_BINARY_PATH` | Parachain node binary | `polkadot-omni-node` |
| `PARACHAIN_CHAIN_SPEC_PATH` | Parachain chain spec JSON | `./zombienet/bulletin-westend-spec.json` |
| `ROCKSDB_LDB_PATH` | RocksDB `ldb` tool (LDB tests only) | `rocksdb_ldb` |

### Parachain network topology

These allow running parachain tests against a different relay chain or deployment:

| Variable | Description | Default |
|---|---|---|
| `RELAY_CHAIN` | Relay chain spec name | `westend-local` |
| `PARACHAIN_ID` | Parachain ID | `2487` |
| `PARACHAIN_CHAIN_ID` | Chain ID for DB path resolution | `bulletin-westend` |

Example with a custom relay chain:

```bash
POLKADOT_RELAY_BINARY_PATH=/path/to/polkadot \
POLKADOT_PARACHAIN_BINARY_PATH=/path/to/polkadot-omni-node \
PARACHAIN_CHAIN_SPEC_PATH=./my-spec.json \
RELAY_CHAIN=rococo-local \
PARACHAIN_ID=2000 \
PARACHAIN_CHAIN_ID=bulletin-rococo \
  cargo test -p bulletin-chain-zombienet-sdk-tests parachain_fast_sync_test -- --nocapture
```

## Key behaviors tested

- **Full sync** downloads all blocks including indexed body -- synced nodes **can** serve data via bitswap.
- **Fast sync** skips block bodies entirely -- synced nodes return `DONT_HAVE` via bitswap.
- **Warp sync** gap fill downloads block bodies (`HEADER|BODY|JUSTIFICATION`) but does not execute them. Bodies go to the BODY column, not TRANSACTIONS -- synced nodes return `DONT_HAVE` via bitswap.
- **Block pruning** deletes historical blocks. When all peers prune, fast/full sync cannot complete (no blocks to download).
- **Warp sync with pruning** still completes because it uses GRANDPA warp proofs instead of downloading historical blocks.
- **LDB tests** verify RocksDB column 11 (transaction storage): refcount increments on duplicate stores, data expires after the retention period.
