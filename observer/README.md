# Bulletin Observer

PAPI demo of the block-headroom SLI for Bulletin Chain. SLO window is 2 weeks.

Block production, authorization lifecycle, and Read-ability (Bitswap) are out of scope here and ship as follow-ups.

## Run

```bash
npm install
npx papi add bulletin -w wss://paseo-bulletin-rpc.polkadot.io
npm start
```

## Limits

- Paseo only. Polkadot Bulletin still runs the empty runtime without `pallet_bulletin_transaction_storage`.
- No Prometheus exporter yet; verdicts log to stdout.
