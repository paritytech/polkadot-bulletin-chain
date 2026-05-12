# Zombienet SDK Tests

Integration tests for Polkadot Bulletin Chain sync modes and transaction storage, using [zombienet-sdk](https://github.com/paritytech/zombienet-sdk) to spawn local networks.

## Prerequisites

Install `just` once: `cargo install just --locked`.

### Parachain binaries + chain spec

All driven through `just`. From the repo root:

```bash
# Fetch (or build from source) polkadot + workers + omni-node + chain-spec-builder.
# Reads POLKADOT_NODE_VERSION / CHAIN_SPEC_BUILDER_VERSION from .github/env.
# Outputs cached under <repo>/.polkadot-binaries/ (gitignored).
just binaries-polkadot
just binaries-chain-spec-builder

# Generate the bulletin-westend chain spec at ./zombienet/bulletin-westend-spec.json.
just chain-spec westend
```

To pin a different polkadot-sdk version for one session, override the env var:

```bash
POLKADOT_NODE_VERSION=polkadot-stable2603 just binaries-polkadot
# OR by commit hash (source-built):
POLKADOT_NODE_VERSION=afba6ccb0a75908f2181ed0e849ddf827c71c501 just binaries-polkadot
```

To use a different parachain ID for the spec, set `PARACHAIN_ID`:

```bash
PARACHAIN_ID=2000 just chain-spec westend
```

The all-in-one `just test-zombienet-auto-renew` / `just test-zombienet-sync` recipes
will fetch the binaries and generate the chain spec for you — see "Running tests" below.

### LDB tests (optional)

The `parachain_ldb_storage_verification_test` inspects RocksDB directly and requires the `ldb` tool:

```bash
export ROCKSDB_LDB_PATH=/path/to/ldb
```

## Running tests

Tests are gated behind feature flags (`zombie-sync-tests`, `zombie-auto-renew-tests`)
so `cargo test --workspace` doesn't accidentally fire them.

Recommended path — `just` recipes from the repo root:

```bash
# Whole sync suite against bulletin-westend (default; `paseo` works too).
just test-zombienet-sync

# Single sync test, runtime override:
just test-zombienet-sync paseo parachain_fast_sync_test

# Whole auto-renew suite (long-running soak skipped automatically).
just test-zombienet-auto-renew

# Single auto-renew test:
just test-zombienet-auto-renew westend parachain_auto_renew_quota_exhaustion_test
```

The recipes fetch the right binaries, generate the chain spec, export the env
vars and call cargo for you. Behind the scenes:

```bash
ZOMBIE_PROVIDER=native cargo test -p bulletin-chain-zombienet-sdk-tests \
  --features bulletin-chain-zombienet-sdk-tests/zombie-sync-tests \
  parachain_fast_sync_test -- --test-threads=1 --nocapture
```

Run tests one at a time (`--test-threads=1`) — each spawns a full network and they
are resource-intensive.

## Test matrix

| Test | Sync mode | Pruning | Expected outcome |
|---|---|---|---|
| `parachain_fast_sync_test` | fast | no | Sync completes, bitswap DONT_HAVE |
| `parachain_fast_sync_with_pruning_test` | fast | yes | Sync fails (pruned blocks) |
| `parachain_warp_sync_test` | warp | no | Sync completes, bitswap DONT_HAVE |
| `parachain_warp_sync_with_pruning_test` | warp | yes | Sync completes, bitswap DONT_HAVE |
| `parachain_full_sync_test` | full | no | Sync completes, bitswap works |
| `parachain_full_sync_with_pruning_test` | full | yes | Sync fails (pruned blocks) |
| `parachain_full_sync_relay_warp_sync_test` | full + warp (relay) | no | Relay warp syncs, parachain full syncs, bitswap works |
| `parachain_rpc_node_bitswap_test` | full | no | RPC node syncs and serves data via bitswap |
| `parachain_ldb_storage_verification_test` | - | yes | Verifies col11 refcounting and data expiration |

## Environment variables

### Binary paths

| Variable | Description | Default |
|---|---|---|
| `POLKADOT_RELAY_BINARY_PATH` | Relay chain binary | `polkadot` |
| `POLKADOT_PARACHAIN_BINARY_PATH` | Parachain node binary | `polkadot-omni-node` |
| `PARACHAIN_CHAIN_SPEC_PATH` | Parachain chain spec JSON | `./zombienet/bulletin-westend-spec.json` |
| `ROCKSDB_LDB_PATH` | RocksDB `ldb` tool (LDB tests only) | `rocksdb_ldb` |

### Parachain network topology

These allow running parachain tests against a different relay chain or deployment:

| Variable | Description | Default |
|---|---|---|
| `RELAY_CHAIN` | Relay chain spec name | `westend-local` |
| `PARACHAIN_ID` | Parachain ID | `1010` |
| `PARACHAIN_CHAIN_ID` | Chain ID for DB path resolution | `bulletin-westend` |

Example with a custom relay chain:

```bash
POLKADOT_RELAY_BINARY_PATH=/path/to/polkadot \
POLKADOT_PARACHAIN_BINARY_PATH=/path/to/polkadot-omni-node \
PARACHAIN_CHAIN_SPEC_PATH=./my-spec.json \
RELAY_CHAIN=rococo-local \
PARACHAIN_ID=2000 \
PARACHAIN_CHAIN_ID=bulletin-rococo \
  cargo test -p bulletin-chain-zombienet-sdk-tests \
  --features bulletin-chain-zombienet-sdk-tests/zombie-sync-tests \
  parachain_fast_sync_test -- --nocapture
```

## Key behaviors tested

- **Full sync** downloads all blocks including indexed body -- synced nodes **can** serve data via bitswap.
- **Fast sync** skips block bodies entirely -- synced nodes return `DONT_HAVE` via bitswap.
- **Warp sync** gap fill downloads block bodies (`HEADER|BODY|JUSTIFICATION`) but does not execute them. Bodies go to the BODY column, not TRANSACTIONS -- synced nodes return `DONT_HAVE` via bitswap.
- **Block pruning** deletes historical blocks. When all peers prune, fast/full sync cannot complete (no blocks to download).
- **Warp sync with pruning** still completes because it uses GRANDPA warp proofs instead of downloading historical blocks.
- **LDB tests** verify RocksDB column 11 (transaction storage): refcount increments on duplicate stores, data expires after the retention period.

## CI

A single workflow (`.github/workflows/zombienet-tests.yml`) hosts both suites:

| Job | Trigger |
|---|---|
| `zombienet-auto-renew-tests` | Every PR push + `workflow_dispatch` |
| `zombienet-sync-tests` | `zombienet-sync-tests` PR label + `workflow_dispatch` |

A shared `prepare-binaries` job fetches/builds the polkadot binaries once and uploads them
as an artifact; both suites download that artifact instead of building locally. Each suite
invokes the same `just test-zombienet-*` recipes used for local runs.
