# Bulletin Chain Dashboard

One Grafana dashboard, sixteen panels, four rows. Import `monitoring/grafana/bulletin-health.json`, pick your Prometheus datasource, done. Auto-refreshes every 10s, defaults to 1h window.

## Layout

```
bulletin-health
├── Row 1 — Is the chain alive?
│   ├── Block Height (best vs finalized)    substrate_block_height{status=best|finalized}
│   ├── Finality Lag                        best - finalized, green < 20 / yellow < 50 / red
│   ├── Peers                               substrate_sub_libp2p_peers_count, red at 0
│   ├── Proof Generation                    bulletin_proof_generation_failed, OK or FAILED
│   ├── Validators                          bulletin_registered_validators, red at 0
│   └── Disk Free %                         node_filesystem_{avail,size}_bytes, red < 5%
│
├── Row 2 — Is data flowing?
│   ├── Store + Renew Transactions          bulletin_block_store_transactions + renew overlay
│   ├── Store + Renew Bytes                 bulletin_block_store_bytes + renew overlay
│   └── Block Production Rate               rate(substrate_block_height{best}[1m]) * 60
│
├── Row 3 — Is IPFS serving?
│   ├── Bitswap Requests/sec                rate(substrate_bitswap_requests_received_total[1m])
│   ├── Bitswap Hit/Miss                    rate(blocks_sent_total) vs rate(blocks_not_found_total)
│   └── Bitswap Throughput                  rate(substrate_bitswap_blocks_sent_bytes_total[1m])
│
└── Row 4 — Is the machine holding up?
    ├── Memory RSS                          process_resident_memory_bytes
    ├── Proof Generation Over Time          bulletin_proof_generation_failed as timeseries
    └── Network Peers Over Time             substrate_sub_libp2p_peers_count as timeseries
```

## Row 1 — Is the chain alive?

The top row is a glanceable status bar. If paged at 3am, start here.

- **Block Height** — Two lines climbing together (best and finalized). Growing gap = GRANDPA stalling.
- **Finality Lag** — The gap as a single number. Healthy = 2–3. Above 20 = consensus struggling.
- **Peers** — 0 = isolated, < 3 = fragile. Zero peers means effectively offline.
- **Proof Generation** — Bulletin-unique: validator must prove it holds 7-day-old data to author blocks. FAILED = storage broken, can't author.
- **Validators** — Active PoA validator count. Drop = removal via governance. Zero = chain halted.
- **Disk Free %** — Bulletin needs 1.5–2 TB. Uses `node_exporter`, not the Bulletin process itself.

## Row 2 — Is data flowing?

- **Store + Renew Transactions** — Total data ops and renewal overlay per block. Gap between lines = pure new stores.
- **Store + Renew Bytes** — Same split but in bytes. Useful for capacity planning (~10 MB max per block).
- **Block Production Rate** — Blocks/min, expected ~10 (6s slots). Below 8 = slots being missed.

## Row 3 — Is IPFS serving?

All panels use `rate(...[1m])` — underlying metrics are Counters from polkadot-sdk's `BitswapServer`, not litep2p (see [metrics-monitoring.md](metrics-monitoring.md) for why).

- **Bitswap Requests/sec** — Incoming requests and CIDs/sec. Zero = nobody fetching right now.
- **Bitswap Hit/Miss** — Found vs not-found. Rising miss rate = data expiring or storage damaged.
- **Bitswap Throughput** — Bytes/sec served over IPFS. Network bandwidth panel.

## Row 4 — Is the machine holding up?

Time-series for spotting slow degradation.

- **Memory RSS** — Gradual climb over days = possible leak.
- **Proof Generation Over Time** — History of proof success/failure. "When did it start failing?"
- **Network Peers Over Time** — Connectivity trends the stat panel can't show.

## Planned panels

Four features are planned, each as a separate PR:

### Total data stored

**Metric:** `bulletin_total_stored_bytes` (Gauge)

A global running total of all data currently held in on-chain storage. Incremented in `do_store()`, decremented when blocks expire past `RetentionPeriod` in `on_initialize()`. Added as a stat panel in Row 1 or a time-series in Row 2.

Shows the chain's actual storage footprint. Combined with Disk Free %, tells you when you'll run out of space.

### Admin operations per block

**Metric:** `bulletin_block_admin_ops` (Gauge)

Per-block count of admin operations: validator add/remove, relayer add/remove, account authorization, preimage authorization, and auth removals/refreshes. Uses the same `BlockRenewCount` pattern — per-block StorageValues cleared in `on_initialize()`, incremented in each extrinsic.

Shows governance activity. Normally zero. Spikes during validator rotations or authorization batches.

### Bridge health

**Metrics:** `bulletin_bridge_outbound_pending`, `bulletin_bridge_outbound_latest_generated_nonce`, `bulletin_bridge_outbound_latest_received_nonce` (Gauges, Polkadot runtime only)

Read from `pallet_bridge_messages::OutboundLanes` storage for lane `[0,0,0,0]`. Pending = `latest_generated_nonce - latest_received_nonce`. Shows whether messages from Bulletin to People Chain are being relayed.

A growing pending count means the bridge is congested or relayers are down. Combined with the Validators panel, tells you whether it's a chain problem or a relay problem.

Not available in Westend runtime (parachain mode has no bridge pallets).

### Cross-node comparison

No new metrics — dashboard-only change. Adds a `$instance` template variable so you can filter or compare metrics across multiple Bulletin nodes.

Requires Prometheus scraping multiple nodes with distinct `instance` labels.

## What the dashboard doesn't cover

- **Per-account storage breakdown** — would need expensive storage iteration or pallet-level per-account tracking. See [metrics-monitoring.md](metrics-monitoring.md#whats-not-here-yet).

For the full metric reference, see [metrics-monitoring.md](metrics-monitoring.md).
