---
name: test-live
description: Health check, smoke test, or full test a live Bulletin chain deployment (Westend, Paseo, or custom endpoint).
allowed-tools: Bash Read Grep Glob
metadata:
  argument-hint: "<network> [health|check|smoke <seed>|full <seed>]"
---

# Live Bulletin Chain Test

Test a live Bulletin chain deployment. Supports multiple networks and test levels.

## Usage

```
/test-live <network> [test-level] [seed]
```

Parse `$ARGUMENTS` to extract:
1. **network** (required) - first argument
2. **test level** (optional, default: `health`) - second argument
3. **seed** (required for `smoke` and `full`) - remaining arguments

## Networks

Resolve the network name to a WSS endpoint:

| Network   | WSS Endpoint                              | Para ID |
|-----------|-------------------------------------------|---------|
| westend   | `wss://westend-bulletin-rpc.polkadot.io`  | 2487    |
| paseo     | `wss://paseo-bulletin-rpc.polkadot.io`    | 5118     |

If the network argument doesn't match any known name, treat it as a custom WSS URL.
Strip trailing slashes. If the user provides an `https://` URL, convert it to `wss://`.

If no network is provided, ask the user which network to test.

## How checks work

Health and check levels use a PAPI-based Node.js script (`examples/health_check.js`) that connects via WebSocket and outputs structured JSON to stdout. Diagnostics go to stderr.

The script is invoked via justfile recipes from the `examples/` directory. PAPI descriptors are regenerated from the live chain before each run to ensure the typed API matches the on-chain runtime.

## Test Levels

### 1. `health` (default) - Node health check

No credentials required. Run from the `examples/` directory:

For known networks:
```bash
cd examples && just health-check-<network>
```

For custom URLs:
```bash
cd examples && just health-check "<wss_url>"
```

The script outputs JSON with these checks:
- **rpc**: Endpoint connectivity (FAIL if unreachable)
- **peers**: Peer count (WARN if < 2)
- **syncing**: Sync status (WARN if syncing)
- **chain**: Chain name, node name, node version
- **runtime**: specName, specVersion, implVersion
- **blockProduction**: Samples best block twice with ~30s gap. Retries once after another ~30s if no progress. FAIL if stalled after ~60s total.
- **finalization**: Compares best vs finalized block number. OK if gap <= 10, WARN if > 10, FAIL if > 100.

Exit codes: 0 = all OK, 1 = any FAIL, 2 = connection error.

Parse the JSON output and present results as:

```
## <Network> Bulletin Chain - Health Check

| Check            | Status | Details                         |
|------------------|--------|---------------------------------|
| RPC Connectivity | OK     | Endpoint responds               |
| Chain Identity   | OK     | <chain> / <node> / <version>    |
| Runtime Version  | OK     | <specName> v<specVersion>       |
| Peers            | OK     | N peers                         |
| Syncing          | OK     | false                           |
| Block Production | OK     | #X -> #Y (+Z in ~30s)          |
| Finalization     | OK     | Best: #X, Final: #Y, Gap: Z    |

Overall: OK / WARN / FAIL
```

If any check is FAIL or WARN, add a **Diagnosis** section with possible causes and recommended actions.

### 2. `check` - Read-only pallet verification

No credentials required. Runs `health` checks plus pallet verification.

For known networks:
```bash
cd examples && just health-check-<network> --check
```

For custom URLs:
```bash
cd examples && just health-check "<wss_url>" --check
```

The `--check` flag adds pallet checks to the JSON output under the `pallet` key, using the PAPI typed API (automatic SCALE decoding, no manual storage key construction):
- **retentionPeriod**: `TransactionStorage.RetentionPeriod` storage value (blocks)
- **byteFee**: `TransactionStorage.ByteFee` storage value
- **entryFee**: `TransactionStorage.EntryFee` storage value
- **maxBlockTransactions**: `TransactionStorage.MaxBlockTransactions` constant
- **maxTransactionSize**: `TransactionStorage.MaxTransactionSize` constant

Present as an additional table after the health results:

```
## <Network> Bulletin Chain - Pallet Check

| Check                 | Status | Details                            |
|-----------------------|--------|------------------------------------|
| Retention Period      | OK     | N blocks (~X days)                 |
| Byte Fee              | OK     | <value>                            |
| Entry Fee             | OK     | <value>                            |
| Max Block Transactions| OK     | N                                  |
| Max Transaction Size  | OK     | N bytes (X MiB)                    |

Overall: OK / WARN / FAIL
```

### 3. `smoke <seed>` - Storage round-trip test

Requires a seed phrase for a pre-authorized account. Runs `check` first, then submits a small storage test to verify the chain accepts and includes transactions.

Steps:
1. Run `check` level first (using the justfile recipe with `--check`)
2. If health or pallet checks have critical failures, stop early
3. Resolve network to its `just` recipe name or WSS URL
4. Run from the `examples/` directory:
   - For known networks: `just run-live-tests-<network> "<seed>" "http://127.0.0.1:8283" small`
   - For custom URLs: `just _run-live-tests "<wss_url>" "<seed>" "http://127.0.0.1:8283" small`
5. If seed is missing, ask the user for it
6. Report results: throughput, blocks used, success/failure

If the test fails with authorization errors, inform the user their account needs to be authorized. On test networks, Alice account should be able to authorize, and its also possible to use https://paritytech.github.io/polkadot-bulletin-chain/authorizations

### 4. `full <seed>` - All checks + storage test

Runs all levels in sequence: `health` -> `check` -> `smoke`.
Stops early if any critical failure is detected at any level.

## Error Handling

- If the health check script exits with code 2, report the endpoint as unreachable and stop
- If `just` is not found, suggest: `cargo install just`
- If npm dependencies missing in `examples/`, the justfile recipes run `npm install` automatically
- Always show partial results - never fail silently
- Mask seed phrases in any displayed commands (show first 4 chars + `...`)
