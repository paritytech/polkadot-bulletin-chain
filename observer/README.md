# Bulletin Observer

PAPI demo of three chain-derived SLIs: block production, block headroom, authorization lifecycle. SLO window is 2 weeks.

Read-ability (Bitswap) is out of scope here; it needs a Bitswap client and lives in a follow-up.

## Run

```bash
npm install
npx papi add bulletin -w wss://paseo-bulletin-rpc.polkadot.io
npm start
```

## Limits

- Paseo only. Polkadot Bulletin still runs the empty runtime without `pallet_bulletin_transaction_storage`.
- Authorization model tracks grants and removals but not allowance deduction on `Stored`/`Renewed`.
- No Prometheus exporter yet; verdicts log to stdout.
