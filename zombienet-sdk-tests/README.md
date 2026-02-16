# Zombienet SDK Tests

Integration tests that simulate real parachain incidents using [zombienet-sdk](https://github.com/paritytech/zombienet-sdk).

## Tests

### `parachain_migration_recovery_test`

Recreates the bulletin-westend incident where a runtime upgrade without the `TransactionInfo` v0→v1 migration caused `check_proof` to fail on old entries, stalling block production. Then recovers the chain using `force_set_current_code` on the relay chain + `codeSubstitutes` in the collator's chain spec + collator restart.

**Flow:** start chain → store data (v0) → upgrade to broken runtime (no migration) → wait for stall → recover via relay + code substitutes → verify chain resumes → normal on-chain upgrade to next runtime → verify normal upgrades work.

## Prerequisites

### Binaries

| Binary | Source | Notes |
|--------|--------|-------|
| `polkadot` | [polkadot-sdk](https://github.com/paritytech/polkadot-sdk) | Relay chain validator |
| `polkadot-omni-node` | [polkadot-sdk](https://github.com/paritytech/polkadot-sdk) | Parachain collator |

### Chain spec

A bulletin-westend chain spec JSON file. Default location: `./zombienet/bulletin-westend-spec.json` (relative to repo root).

Generate it with:

```bash
# Requires chain-spec-builder (cargo install staging-chain-spec-builder)
./scripts/create_bulletin_westend_spec.sh
```

This builds the runtime WASM and runs `chain-spec-builder` to produce `./zombienet/bulletin-westend-spec.json`.

### Runtime WASMs

Four pre-built `bulletin-westend-runtime` WASM blobs in `zombienet-sdk-tests/runtimes/`:

| File | TransactionInfo | Migration | Description |
|------|-----------------|-----------|-------------|
| `old_runtime.compact.compressed.wasm` | v0 | None | Chain starts with this runtime |
| `broken_runtime.compact.compressed.wasm` | v1 | **None** | Causes stall (can't decode v0 entries) |
| `fix_runtime.compact.compressed.wasm` | v1 | v0→v1 in `on_initialize` | Recovers chain via `codeSubstitutes` |
| `next_runtime.compact.compressed.wasm` | v1 | None (un-wired) | Normal upgrade after recovery |

The broken and fix runtimes can share the same `spec_version` — recovery uses `codeSubstitutes` (client-side code replacement), not `on_runtime_upgrade`. The next runtime must have a bumped `spec_version` for a standard on-chain upgrade.

These are gitignored — build them from the appropriate commits.

## Run

```bash
POLKADOT_RELAY_BINARY_PATH=/path/to/polkadot \
POLKADOT_PARACHAIN_BINARY_PATH=/path/to/polkadot-omni-node \
cargo test -p bulletin-chain-zombienet-sdk-tests -- --nocapture
```

All environment variables are optional if binaries are on `$PATH` and files are at default locations.

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `POLKADOT_RELAY_BINARY_PATH` | `polkadot` | Relay chain binary |
| `POLKADOT_PARACHAIN_BINARY_PATH` | `polkadot-omni-node` | Parachain collator binary |
| `PARACHAIN_CHAIN_SPEC_PATH` | `./zombienet/bulletin-westend-spec.json` | Chain spec |
| `OLD_RUNTIME_WASM_PATH` | `./zombienet-sdk-tests/runtimes/old_runtime.compact.compressed.wasm` | Old runtime |
| `BROKEN_RUNTIME_WASM_PATH` | `./zombienet-sdk-tests/runtimes/broken_runtime.compact.compressed.wasm` | Broken runtime |
| `FIX_RUNTIME_WASM_PATH` | `./zombienet-sdk-tests/runtimes/fix_runtime.compact.compressed.wasm` | Fix runtime |
| `NEXT_RUNTIME_WASM_PATH` | `./zombienet-sdk-tests/runtimes/next_runtime.compact.compressed.wasm` | Next runtime (post-recovery upgrade) |
| `RELAY_CHAIN` | `westend-local` | Relay chain name |
| `PARACHAIN_ID` | `2487` | Parachain ID |
| `PARACHAIN_CHAIN_ID` | `bulletin-westend` | Parachain chain ID |
| `ZOMBIENET_SDK_BASE_DIR` | `/tmp/zombienet-test-{pid}` | Temp dir for network data |
