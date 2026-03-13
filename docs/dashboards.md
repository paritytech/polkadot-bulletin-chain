# Bulletin Chain Dashboard

One Grafana dashboard, sixteen panels, four rows. Each row answers a different question about the chain's health: Is it alive? Is data flowing? Is IPFS serving? Is the machine holding up?

## How to import

1. Open Grafana → Dashboards → Import
2. Upload `monitoring/grafana/bulletin-health.json`
3. Select your Prometheus datasource when prompted
4. The dashboard auto-refreshes every 10 seconds and defaults to a 1-hour window

## Row 1 — Is the chain alive?

The top row is a glanceable status bar. Six panels, each answering one yes/no question. If you're paged at 3am, start here.

### Block Height (best vs finalized)

**Metrics:** `substrate_block_height{status="best"}`, `substrate_block_height{status="finalized"}`

Two lines that should climb together, one block every 6 seconds. The "best" line is the latest block the node knows about; the "finalized" line is the latest block confirmed by GRANDPA. A growing gap between them means finality is stalling — the chain is still producing blocks but can't agree they're final.

### Finality Lag

**Metric:** `substrate_block_height{status="best"} - substrate_block_height{status="finalized"}`

The gap between best and finalized as a single number. Green under 20, yellow 20–50, red above 50. On a healthy chain this hovers around 2–3. If it climbs past 20, GRANDPA consensus is struggling — usually a sign that validators can't communicate or too many are offline.

### Peers

**Metric:** `substrate_sub_libp2p_peers_count`

How many other nodes this one is connected to. Red at 0 (isolated — can't receive or propagate blocks), yellow at 1–2 (fragile), green at 3+. A node with zero peers is effectively offline even if the process is running.

### Proof Generation

**Metric:** `bulletin_proof_generation_failed`

The metric unique to Bulletin. Before authoring a block, the validator must prove it still holds data from ~7 days ago. This panel shows OK (green) or FAILED (red). A failure means this specific node cannot author blocks — its storage is corrupted or data has been lost. Other validators can still produce blocks, but this one is dead for authoring until storage is fixed.

### Validators

**Metric:** `bulletin_registered_validators`

How many validators are in the active set. Bulletin uses a PoA validator model managed through the `ValidatorSet` pallet. Red at 0 (chain halted — no one can produce blocks), yellow at 1–2 (fragile), green at 3+. A sudden drop means validators were explicitly removed via governance.

### Disk Free %

**Metric:** `100 * node_filesystem_avail_bytes{mountpoint="/"} / node_filesystem_size_bytes{mountpoint="/"}`

Bulletin stores up to 1.5–2 TB of data with 7-day retention. This gauge shows remaining disk as a percentage. Red below 5%, yellow 5–15%, green above 15%. Running out of disk is a slow-motion catastrophe — the node can't store new data, can't generate proofs, and eventually crashes.

**Why it uses `node_exporter`:** This metric comes from Prometheus's `node_exporter`, not from the Bulletin node itself. The node doesn't know about disk space — it just writes. You need `node_exporter` running alongside the Bulletin node for this panel to work.

## Row 2 — Is data flowing?

The second row shows what Bulletin exists for: storing data. Three time-series panels covering volume, size, and production rate.

### Store + Renew Transactions (per block)

**Metrics:** `bulletin_block_store_transactions`, `bulletin_block_renew_transactions`

Two overlaid lines. The "total" line counts all data operations (stores + renewals) in each block. The "renewals" line counts just the renewals — data whose retention is being extended for another cycle. The gap between them is pure new data.

On a healthy chain with active users you'll see both lines moving. If renewals drop to zero and stores continue, nobody is extending data retention — everything will expire after 7 days. If both are zero for extended periods, the chain is idle (which may be fine, or may mean clients can't submit transactions).

### Store + Renew Bytes (per block)

**Metrics:** `bulletin_block_store_bytes`, `bulletin_block_renew_bytes`

The byte-level companion to the transaction count panel. Shows how much data (in bytes) was stored and renewed per block. Useful for capacity planning — a sustained spike in bytes means more disk consumption per block. Each block can hold up to ~10 MB total.

### Block Production Rate

**Metric:** `rate(substrate_block_height{status="best"}[1m]) * 60`

Blocks produced per minute. Expected value is ~10 (one block every 6 seconds). Drops below 10 mean blocks are being skipped — either the validator scheduled to produce missed its slot, or the network is partitioned. Consistently below 8 warrants investigation.

## Row 3 — Is IPFS serving?

Bulletin serves stored data over IPFS via the Bitswap protocol. These three panels tell you whether external peers can actually retrieve data. The metrics come from polkadot-sdk's `BitswapServer` (not from litep2p directly — see [metrics-monitoring.md](metrics-monitoring.md) for why).

All three panels use `rate(...[1m])` because the underlying metrics are Counters (monotonically increasing), unlike the Gauges in rows 1–2.

### Bitswap Requests/sec

**Metrics:** `rate(substrate_bitswap_requests_received_total[1m])`, `rate(substrate_bitswap_cids_requested_total[1m])`

Two lines: incoming Bitswap request rate and the CID request rate. A single Bitswap request can ask for multiple CIDs, so the CIDs/s line is always ≥ requests/s. Activity here means external IPFS peers are fetching data from this node. Zero activity isn't necessarily bad — it just means nobody is requesting data right now.

### Bitswap Hit/Miss

**Metrics:** `rate(substrate_bitswap_blocks_sent_total[1m])`, `rate(substrate_bitswap_blocks_not_found_total[1m])`

The most telling Bitswap panel. "Blocks found" means the node had the requested data and served it. "Not found" means the data was requested but wasn't in storage — either it expired (past the 7-day retention), was never stored on this node, or storage is corrupted.

A healthy node shows mostly hits. A rising miss rate with constant request rate means data is expiring faster than expected, or the node's indexed storage is damaged. Combined with `bulletin_proof_generation_failed = 1`, this confirms a storage problem.

### Bitswap Throughput

**Metric:** `rate(substrate_bitswap_blocks_sent_bytes_total[1m])`

Bytes per second of data served over Bitswap. This is the "bandwidth" panel — how much data this node is actually delivering to the IPFS network. Useful for sizing network capacity and understanding load patterns.

## Row 4 — Is the machine holding up?

The bottom row tracks system-level health over time. These are the "did something change?" panels — useful for spotting slow degradation.

### Memory RSS

**Metric:** `process_resident_memory_bytes`

Resident memory of the node process. A steady line is normal. A gradual upward trend over days suggests a memory leak. A sudden spike might indicate a burst of large transactions being processed.

### Proof Generation Status Over Time

**Metric:** `bulletin_proof_generation_failed`

The same metric as the Row 1 stat panel, but as a time-series. Shows the history of proof generation success/failure. Useful for correlating proof failures with other events — did proofs start failing when disk hit 95%? When a validator was removed? The stat panel tells you "right now"; this panel tells you "when did it start?"

### Network Peers Over Time

**Metric:** `substrate_sub_libp2p_peers_count`

Peer count as a time-series. A stable line around 10–25 is healthy. Sudden drops correlate with network issues. A slow decline might mean the node's address is falling out of peer tables. Useful for understanding connectivity trends that the Row 1 stat panel can't show.

## What the dashboard doesn't cover

The dashboard focuses on a single node's view. It doesn't show:

- **Cross-node comparison** — you'd need multi-instance Prometheus labels and `instance` selectors
- **Bridge health** — relay between Bulletin and People Chain has its own metrics (see bridge monitoring)
- **Total data stored by accounts** — no metric for this yet (see [metrics-monitoring.md](metrics-monitoring.md#whats-not-here-yet))
- **Admin operations** — validator/relayer set changes aren't tracked per-block yet

For the full list of available metrics and what's planned, see [metrics-monitoring.md](metrics-monitoring.md).
