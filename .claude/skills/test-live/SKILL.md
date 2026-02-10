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

Resolve the network name to an HTTPS RPC endpoint:

| Network   | HTTPS Endpoint                              | Para ID |
|-----------|---------------------------------------------|---------|
| westend   | `https://westend-bulletin-rpc.polkadot.io`  | 2487    |
| paseo     | `https://paseo-bulletin-rpc.polkadot.io`    | TBD     |

If the network argument doesn't match any known name, treat it as a custom HTTPS URL.
Strip trailing slashes. If the user provides a `wss://` URL, convert it to `https://` for curl-based checks.

If no network is provided, ask the user which network to test.

## RPC Helper

All levels use `curl` with JSON-RPC over HTTPS:
```bash
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"id":1,"jsonrpc":"2.0","method":"<METHOD>","params":[<PARAMS>]}' \
  <ENDPOINT>
```

## Test Levels

### 1. `health` (default) - Node health check

No credentials required. Verifies the node is reachable, connected, and producing blocks.

Run these checks (parallelize independent calls for speed):

**a) RPC Connectivity + System Health** - `system_health`
- Parse `peers`, `isSyncing`, `shouldHavePeers`
- FAIL if endpoint unreachable (skip all remaining checks)
- WARN if `peers < 2`
- WARN if `isSyncing: true`

**b) Chain Identity** - `system_chain`, `system_version`, `system_name`
- Report chain name, node implementation, software version

**c) Runtime Version** - `state_getRuntimeVersion`
- Report `specName`, `specVersion`, `implVersion`

**d) Block Production** - `chain_getHeader` (twice, ~15s apart)
- First call: record block number (hex -> decimal)
- Wait ~15 seconds
- Second call: record new block number
- OK if block number increased
- **FAIL** if block number unchanged (chain stalled)

**e) Finalization** - `chain_getFinalizedHead` then `chain_getHeader` with that hash
- Compare finalized block number to best block number
- OK if gap <= 10 blocks
- WARN if gap > 10 (finalization lagging)
- FAIL if gap > 100 (finalization severely behind)

Present results as:

```
## <Network> Bulletin Chain - Health Check

| Check            | Status | Details                         |
|------------------|--------|---------------------------------|
| RPC Connectivity | OK     | Endpoint responds               |
| Chain Identity   | OK     | <chain> / <node> / <version>    |
| Runtime Version  | OK     | <specName> v<specVersion>       |
| Peers            | OK     | N peers                         |
| Syncing          | OK     | false                           |
| Block Production | OK     | #X -> #Y (+Z in ~15s)          |
| Finalization     | OK     | Best: #X, Final: #Y, Gap: Z    |

Overall: OK / WARN / FAIL
```

If any check is FAIL or WARN, add a **Diagnosis** section with possible causes and recommended actions.

### 2. `check` - Read-only pallet verification

No credentials required. Runs `health` first, then queries on-chain storage to verify the TransactionStorage pallet is configured and operational.

After the health table, add these pallet checks using `state_getStorage` RPC. Storage keys use FRAME's Twox128 hashing on pallet and storage item names.

**Storage key construction**: `twox128("TransactionStorage") + twox128("<StorageItem>")`

Pre-computed key prefixes (TransactionStorage pallet):
- `RetentionPeriod`: `0x` + `twox128("TransactionStorage")` + `twox128("RetentionPeriod")`
- `ByteFee`: `0x` + `twox128("TransactionStorage")` + `twox128("ByteFee")`
- `EntryFee`: `0x` + `twox128("TransactionStorage")` + `twox128("EntryFee")`

To compute twox128 hashes, use `subxt` or compute them inline. Alternatively, query the metadata via `state_getMetadata` to confirm the pallet exists, or use a known-good shortcut:

Query the runtime constants via `state_call` with `Metadata_metadata_versions` to verify the runtime is responsive, then use `state_getKeys` with the pallet prefix to verify storage items exist:
```bash
# Check if TransactionStorage pallet has any storage keys
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"id":1,"jsonrpc":"2.0","method":"state_getKeysPaged","params":["0x3a636f6465",null,1]}' \
  <ENDPOINT>
```

Perform these checks:

**a) Pallet existence** - `state_getMetadata`
- Fetch metadata, confirm it returns successfully (don't parse the full blob, just verify non-error response)
- OK if metadata returned, FAIL if error

**b) Recent storage activity** - `chain_getBlock` on the latest finalized block
- Fetch the finalized block body
- Check if any extrinsics reference the TransactionStorage pallet (pallet index 40)
- OK if chain has recent storage activity, INFO if no storage txs in the latest block (this is normal)

**c) Runtime constants** - `state_call` with `TransactionStorageApi_retention_period`
- Call: `{"method":"state_call","params":["TransactionStorageApi_retention_period","0x"]}`
- If the runtime API exists, decode the SCALE-encoded result (little-endian u32/u64 block count)
- Report the retention period in blocks
- WARN if the call fails (API may not be exposed)

Present as an additional table after the health results:

```
## <Network> Bulletin Chain - Pallet Check

| Check              | Status | Details                            |
|--------------------|--------|------------------------------------|
| Metadata           | OK     | Runtime metadata accessible        |
| Storage Activity   | INFO   | No storage txs in latest block     |
| Retention Period   | OK     | N blocks (~X days)                 |

Overall: OK / WARN / FAIL
```

### 3. `smoke <seed>` - Storage round-trip test

Requires a seed phrase for a pre-authorized account. Runs `check` first, then submits a small storage test to verify the chain accepts and includes transactions.

Steps:
1. Run `check` level first
2. If health or pallet checks have critical failures, stop early
3. Resolve network to its `just` recipe name or WSS URL
4. Run from the `examples/` directory:
   - For known networks: `just run-live-tests-<network> "<seed>" "http://127.0.0.1:8283" small`
   - For custom URLs: `just _run-live-tests "<wss_url>" "<seed>" "http://127.0.0.1:8283" small`
5. If seed is missing, ask the user for it
6. Report results: throughput, blocks used, success/failure

To convert HTTPS endpoint back to WSS for the justfile: replace `https://` with `wss://`.

If the test fails with authorization errors, inform the user their account needs to be authorized via sudo on that network.

### 4. `full <seed>` - All checks + storage test

Runs all levels in sequence: `health` -> `check` -> `smoke`.
Stops early if any critical failure is detected at any level.

## Error Handling

- If `curl` fails to connect, report the endpoint as unreachable and stop
- If `just` is not found, suggest: `cargo install just`
- If npm dependencies missing in `examples/`, run `npm install` automatically
- Always show partial results - never fail silently
- Mask seed phrases in any displayed commands (show first 4 chars + `...`)
