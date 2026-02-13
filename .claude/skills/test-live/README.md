# `/test-live` - Live Bulletin Chain Testing

Test health, storage, and overall status of a live Bulletin chain deployment.

## Usage

```
/test-live <network> [health|check|smoke <seed>|full <seed>]
```

## Networks

| Network   | Endpoint                                    |
|-----------|---------------------------------------------|
| `westend` | `wss://westend-bulletin-rpc.polkadot.io`    |
| `paseo`   | `wss://paseo-bulletin-rpc.polkadot.io`      |
| Custom    | Any `wss://` URL                            |

Health and check levels use a PAPI-based Node.js script (`examples/health_check.js`) that connects via WebSocket and outputs structured JSON. The `smoke` level uses additional justfile recipes for transaction submission.

## Test Levels

Each level includes all checks from the levels above it.

| Level              | Credentials         | What it does                                                                          |
|--------------------|---------------------|---------------------------------------------------------------------------------------|
| `health` (default) | None               | RPC connectivity, peers, block production, finalization, runtime version, sync status |
| `check`           | None                | Health + read-only pallet verification (retention period, fees, limits)                |
| `smoke <seed>`    | Pre-authorized seed | Check + storage round-trip (`just run-live-tests-<network>` with `small` image)       |
| `full <seed>`     | Pre-authorized seed | All of the above, stops early on critical failures                                    |

## Examples

```bash
/test-live westend                                # health check (default)
/test-live paseo check                            # health + pallet verification
/test-live westend smoke "my seed phrase"         # storage round-trip test
/test-live paseo full "my seed phrase"            # all checks
/test-live wss://custom-rpc.example.com health    # custom endpoint
```
