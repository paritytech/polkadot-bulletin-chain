# Grafana dashboards

Grafana dashboard models for the Bulletin networks. Edit here, then re-import.

- `bulletin-paseo-dashboard.json`: Paseo Next V2
- `bulletin-polkadot-dashboard.json`: Polkadot. The `kubo job` variable holds the
  Prometheus job of the mainnet IPFS gateway; none is deployed yet, so verify it first.
- `bulletin-polkadot-hop-dashboard.json`: Polkadot HOP pool, promotion and RPC health
  (`substrate_hop_*`). Needs nodes built with polkadot-sdk#12662 and `--enable-hop`.

Import: [Grafana](https://grafana.teleport.parity.io/dashboard/new?orgId=1&from=now-6h&to=now&timezone=browser)
→ New → New dashboard → Import dashboard → upload or paste JSON → Load → Overwrite.
Each file has a fixed `uid`, so re-importing updates the dashboard in place.
