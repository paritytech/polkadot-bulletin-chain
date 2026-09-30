# bulletin-indexer

Minimal event-to-Prometheus indexer for Bulletin Chain. Subscribes to finalised
blocks via PAPI, decodes `TransactionStorage` pallet events, and exposes
`/metrics` for SRE Prometheus to scrape.

Built so that `PermanentStorageNearCap`, `ProofChecked` cadence, authorisation
lifecycle, and renew traffic become Grafana panels rather than something only
visible on PolkadotJS / Subscan.

## Setup

```bash
cd indexer
npm install
npm run papi:generate
cp .env.example .env
npm start
```

Hit `http://localhost:9100/metrics` to see the scrape output.

## Env vars

| Name              | Default          | Notes                            |
| ----------------- | ---------------- | -------------------------------- |
| `INDEXER_NETWORK` | `paseo-next-v2`  | Key from `src/networks.ts`       |
| `INDEXER_PORT`    | `9100`           | HTTP listen port for `/metrics`  |
| `INDEXER_POLL_SEC`| `60`             | Backup state-poll cadence        |

## Metrics emitted

Counters (cumulative; rate them via Prometheus `rate()`):

```
bulletin_stored_total{network}
bulletin_renewed_total{network}
bulletin_proof_checked_total{network}
bulletin_data_auto_renewed_total{network}
bulletin_auto_renewal_failed_total{network}
bulletin_account_authorized_total{network}
bulletin_account_authorization_refreshed_total{network}
bulletin_expired_account_authorization_removed_total{network}
bulletin_permanent_storage_near_cap_events_total{network}
```

Gauges:

```
bulletin_indexer_last_finalised_block{network}
bulletin_permanent_storage_used_bytes{network}
bulletin_permanent_storage_max_bytes{network}
bulletin_retention_period_blocks{network}
```

## Useful Grafana queries

**Storage cap headroom (per the SLO doc threshold)**

```
bulletin_permanent_storage_used_bytes / bulletin_permanent_storage_max_bytes
```

Alert when this exceeds 0.8.

**Write rate, per minute**

```
sum(rate(bulletin_stored_total[5m])) by (network) * 60
```

**Proof success cadence (should be ~1/block)**

```
rate(bulletin_proof_checked_total[5m])
```

If this drops below `0.16` (~1 every 6s) the chain is stalling on proofs.

**Indexer health: blocks per minute observed**

```
rate(bulletin_indexer_last_finalised_block[1m]) * 60
```

A flat line means the indexer is stuck.

## Deployment

Same model as `probe/`: one process per network, scraped by Parity SRE
Prometheus. Typically a small Docker container in the SRE k8s namespace next to
the RPC nodes. Long-running, no per-invocation cadence (Prometheus pulls).
