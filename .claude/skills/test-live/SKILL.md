---
name: test-live
description: Health check, smoke test, or full test a live Bulletin chain deployment (Westend, Paseo, or custom endpoint).
allowed-tools: Bash Read Grep Glob
metadata:
  argument-hint: "<network> [health|smoke <seed>|full <seed>]"
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

## Test Levels

### `health` (default) - Read-only chain health check

No credentials required. Uses `curl` with JSON-RPC over HTTPS.

RPC call pattern:
```bash
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"id":1,"jsonrpc":"2.0","method":"<METHOD>","params":[<PARAMS>]}' \
  <ENDPOINT>
```

Run these checks (parallelize independent calls for speed):

**1. RPC Connectivity + System Health** - `system_health`
- Parse `peers`, `isSyncing`, `shouldHavePeers`
- FAIL if endpoint unreachable (skip all remaining checks)
- WARN if `peers < 2`
- WARN if `isSyncing: true`

**2. Chain Identity** - `system_chain`, `system_version`, `system_name`
- Report chain name, node implementation, software version

**3. Runtime Version** - `state_getRuntimeVersion`
- Report `specName`, `specVersion`, `implVersion`

**4. Block Production** - `chain_getHeader` (twice, ~15s apart)
- First call: record block number (hex -> decimal)
- Wait ~15 seconds
- Second call: record new block number
- OK if block number increased
- **FAIL** if block number unchanged (chain stalled)

**5. Finalization** - `chain_getFinalizedHead` then `chain_getHeader` with that hash
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

### `smoke <seed>` - Quick storage round-trip test

Requires a seed phrase for a pre-authorized account. Runs a small storage test to verify the chain can accept and include transactions.

Steps:
1. Resolve network to its `just` recipe name or WSS URL
2. Run from the `examples/` directory:
   - For known networks: `just run-live-tests-<network> "<seed>" "http://127.0.0.1:8283" small`
   - For custom URLs: `just _run-live-tests "<wss_url>" "<seed>" "http://127.0.0.1:8283" small`
3. If seed is missing, ask the user for it
4. Report results: throughput, blocks used, success/failure

To convert HTTPS endpoint back to WSS for the justfile: replace `https://` with `wss://`.

If the test fails with authorization errors, inform the user their account needs to be authorized via sudo on that network.

### `full <seed>` - Health check + smoke test

1. Run `health` check first
2. If no critical failures (RPC reachable, blocks producing), proceed to `smoke` test
3. If health check shows FAIL on block production or RPC, stop and report - no point running smoke test on a stalled chain

## Error Handling

- If `curl` fails to connect, report the endpoint as unreachable and stop
- If `just` is not found, suggest: `cargo install just`
- If npm dependencies missing in `examples/`, run `npm install` automatically
- Always show partial results - never fail silently
- Mask seed phrases in any displayed commands (show first 4 chars + `...`)
