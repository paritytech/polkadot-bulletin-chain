# Grafana dashboards

Grafana dashboard JSON models for the Bulletin networks. Source of truth: edit
here, then re-import into Grafana.

## Dashboards

- `bulletin-summit-dashboard.json` — Bulletin on the Summit network: chain
  liveness, IPFS gateway (kubo), bitswap, HOP RPC.
- `bulletin-paseo-dashboard.json` — same for Paseo Next V2 (`next-bulletin-paseo`).
- `bulletin-ipfs-gateway.json` — IPFS gateway detail.

## Import / update

Grafana → Dashboards → Import → paste the JSON → Load → Import. Each model
carries a fixed `uid`, so re-importing the same file updates the existing
dashboard (choose Overwrite) instead of creating a copy. Pick the Prometheus
datasource when prompted.

## Modify

Copy a dashboard's JSON, change the panel queries, re-import. Conventions used
throughout:

- Panels are scoped by the `chain` template variable, so queries filter on
  `chain=~"$chain"` rather than a hard-coded chain.
- HOP RPC panels read Substrate's `substrate_rpc_calls_*` metrics, filtered to
  `method=~"hop_.*"` and excluding RPC nodes (`node!~".*rpc.*"`) since HOP is
  served by the `--enable-hop` collators.
- Counts use `increase(...[$__rate_interval])`; `$__rate_interval` keeps the
  window at least one scrape, so short windows don't return empty.
