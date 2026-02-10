---
name: test-live-westend
description: Test and verify the live Westend bulletin parachain health, connectivity, and storage functionality.
allowed-tools: Bash Read Grep Glob
metadata:
  argument-hint: "[check|store-test <seed>|full <seed>]"
---

# Live Westend Bulletin Chain Health Check & Test

Verify that the live Westend bulletin parachain is operating correctly by running connectivity checks, chain state queries, and optionally storage integration tests.

## Network Info

- **RPC Endpoint (HTTPS)**: `https://westend-bulletin-rpc.polkadot.io`
- **RPC Endpoint (WSS)**: `wss://westend-bulletin-rpc.polkadot.io`
- **Para ID**: 2487
- **Chain**: Westend Bulletin

## Modes

Parse `$ARGUMENTS` to determine what to do:

### 1. `check` (default when no arguments or just "check")

Run a comprehensive health check against the live chain using `curl` with the HTTPS JSON-RPC endpoint. Do NOT require a seed phrase. Run as many queries in parallel as possible for speed.

Use this helper pattern for all RPC calls:
```bash
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"id":1,"jsonrpc":"2.0","method":"METHOD","params":[]}' \
  https://westend-bulletin-rpc.polkadot.io
```

Perform ALL of the following checks and report results in a summary table:

**a) RPC Connectivity & System Health** - method: `system_health`
Parse `peers`, `isSyncing`, `shouldHavePeers` from the response. Flag a warning if peers < 2.

**b) Chain Identity** - methods: `system_chain`, `system_version`, `system_name`
Report the chain name, node implementation, and software version.

**c) Block Production** - method: `chain_getHeader` (call twice with ~15 second gap)
Parse the block number (it's hex, convert to decimal). Wait ~15 seconds and query again. Verify the block number has increased. If it hasn't, flag that **block production is stalled** - this is a critical failure.

**d) Finalization** - methods: `chain_getFinalizedHead`, then `chain_getHeader` with that hash as param
Get the finalized head hash, then get its header. Compare the finalized block number to the best block number. If the gap is larger than 10 blocks, flag a finalization lag warning.

**e) Runtime Version** - method: `state_getRuntimeVersion`
Report `specName`, `specVersion`, `implVersion`, `transactionVersion`.

**Summary**: Present results as a clear markdown table:
```
| Check              | Status | Details                          |
|--------------------|--------|----------------------------------|
| RPC Connectivity   | OK/FAIL| Endpoint responds / unreachable  |
| Chain Identity     | OK     | chain / node_name / version      |
| Peers              | OK/WARN| N peers connected                |
| Block Production   | OK/FAIL| Block #X -> #Y in ~15s           |
| Finalization       | OK/WARN| Best: #X, Finalized: #Y, Gap: Z |
| Runtime Version    | OK     | spec_name v<spec_version>        |
| Syncing            | OK/WARN| isSyncing: true/false            |
```

After the table, if there are any FAIL or WARN results, provide a **diagnosis section** explaining possible causes and recommended next steps.

### 2. `store-test <seed>` - Run a storage integration test

Requires a seed phrase. Runs the live network store-big-data test with `small` image size (quick smoke test).

Steps:
1. Run from the `examples/` directory: `just run-live-tests-westend "<seed>" "http://127.0.0.1:8283" small`
2. If the seed is not provided in $ARGUMENTS, ask the user for it
3. Report the test results (throughput, blocks, success/failure)

Note: the account must be pre-authorized on the Westend bulletin chain. If authorization errors occur, inform the user they need sudo access to authorize their account first.

### 3. `full <seed>` - Run all checks plus store test

Runs the `check` health checks first, then if all critical checks pass (RPC connectivity, block production), proceeds to run the `store-test`.

## Error Handling

- If RPC is unreachable, report immediately and skip dependent checks
- If `just` is not found for store tests, suggest `cargo install just`
- If `npm install` hasn't been run in `examples/`, run it automatically
- Always report partial results - don't fail silently on individual checks

## Important Notes

- Never expose or log seed phrases in output - mask them in any displayed commands
- The `check` mode requires NO credentials and is safe to run anytime
- The `store-test` mode submits real transactions and requires a funded, pre-authorized account
- Block time on Westend bulletin is ~12 seconds (relay chain block time)
