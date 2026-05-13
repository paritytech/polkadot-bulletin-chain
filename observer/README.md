# Bulletin Observer

A small Polkadot-API (PAPI) script that demonstrates three chain-derived SLIs for Bulletin Chain:

- **Block production**: actual finalized blocks vs expected by slot cadence
- **Block headroom**: per-block count and byte capacity below 80% of the per-block caps
- **Authorization lifecycle**: local event-derived model matches on-chain `Authorizations` storage

This is a demo, not a production observer. It logs verdicts to stdout. A follow-up wires Prometheus metrics, alerting, and a Bitswap probe for the Read-ability SLI.

## Run

```bash
cd observer
npm install
npx papi add bulletin -w wss://paseo-bulletin-rpc.polkadot.io
npm start
```

The first time, `papi add` generates typed descriptors against the live Paseo Bulletin metadata.

## What it does

Every finalized block:
- Counts `transactionStorage.Stored` events.
- Reads `TransactionStorage::Transactions(N)` storage, sums each entry's `size` field.
- Classifies the block as good (both `count < 0.8 × 512` and `bytes < 0.8 × 8 MB`) or bad.
- Updates a local model of authorizations from `AccountAuthorized` / `AccountAuthorizationRefreshed` / `ExpiredAccountAuthorizationRemoved` events.

Every 60 seconds:
- Reads the on-chain `Authorizations` storage map and compares against the local model. Logs any mismatch.

Block production SLI is `observed_blocks / expected_blocks` (expected derived from wall-clock vs 6 s slot duration; async backing produces parablocks at the relay-chain slot cadence).

SLO window is 2 weeks. The script logs per-block; aggregation over the window is a Prometheus query concern once metrics are wired.

## Constants

These mirror the Bulletin runtime:
- `SLOT_DURATION_MS = 6_000` (parachain block cadence with async backing)
- `MAX_BLOCK_TX = 512` (`MaxBlockTransactions`)
- `BYTE_CAP = 8 MB`

## Limits and TODOs

- Only Paseo today. Polkadot Bulletin currently runs an empty runtime without `pallet_bulletin_transaction_storage`; the script will fail until the storage pallet ships there.
- `Stored` / `Renewed` allowance deduction in the auth model is omitted for brevity. Add it by matching the extrinsic signer.
- Read-ability (Bitswap) is intentionally out of scope; it needs a Bitswap client and belongs in a follow-up PR.
- No Prometheus exporter yet. Verdicts log to stdout.
