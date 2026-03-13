# Bulletin Chain Metrics

Bulletin is a data storage chain. Its job is simple: accept data, store it for 7 days, prove you still have it, and serve it over IPFS. The metrics below tell you whether that's working.

## What makes Bulletin different

A regular parachain just processes transactions. Bulletin does something extra — every block, the producer must prove they still hold data from ~7 days ago (100,800 blocks back). If they can't, the block is invalid. This is the core contract of the chain: store data, prove it, or stop.

This means Bulletin has failure modes that other chains don't. A validator can have perfect networking, consensus, and block production — but if its storage is corrupted, it can't author blocks.

## The metrics

All six are Gauges — a value that represents the current state at the moment Prometheus scrapes it. Unlike counters (which only go up), gauges go up and down. Think of a thermometer: it shows the temperature right now, not the total heat accumulated since boot. Each time a new block is imported, these gauges are set to reflect that block's state.

### `bulletin_proof_generation_failed`

**Gauge: 0 = ok, 1 = failed**

The most important Bulletin-specific metric. Before producing a block, the node generates a storage proof for data from 7 days ago. If this fails, the node cannot author. The gauge stays at 1 until the next successful proof generation.

A validator showing `1` here means its data store is broken — it either lost data or can't read it. Other validators can still produce blocks, but this node is effectively offline for authoring.

### `bulletin_block_store_transactions`

**Gauge: number of store transactions in the latest block**

How many pieces of data were stored in the most recent block. Each block can hold up to 512 store transactions, each up to 8 MB (10 MB total block limit). On an active chain this is the "inflow" — how much new data is being committed.

Zero for a long time is fine if nobody is submitting data. But if clients are submitting and this stays zero, something is wrong with transaction processing.

### `bulletin_block_store_bytes`

**Gauge: bytes stored in the latest block**

The raw byte count of data stored in the latest block. This is the companion to `bulletin_block_store_transactions` — it tells you not just how many items but how large they are. Useful for capacity planning and spotting unusual patterns (e.g., sudden spike in large transactions).

### `bulletin_block_renew_transactions`

**Gauge: number of renew transactions in the latest block**

How many data renewals happened in the most recent block. Renewals extend the retention of previously stored data for another 7-day cycle. A healthy chain with active users will show a mix of stores and renewals. If important data isn't being renewed before expiry, it will be pruned.

Note: `bulletin_block_store_transactions` counts all data operations (stores + renewals). Pure new stores = `store_transactions - renew_transactions`.

### `bulletin_block_renew_bytes`

**Gauge: bytes renewed in the latest block**

The raw byte count of renewed data. Companion to `bulletin_block_renew_transactions`.

### `bulletin_registered_validators`

**Gauge: number of validators in the set**

How many validators are registered in the validator set. This is the chain's capacity for block production. On Bulletin, validators are managed via the `ValidatorSet` pallet (PoA model). Changes take effect in the session after next.

A sudden drop means validators were removed. Zero means the chain cannot produce blocks.

## Bitswap / IPFS serving metrics (upstream)

Bulletin serves stored data over IPFS via litep2p's Bitswap protocol. The metrics for this live in polkadot-sdk, not in litep2p or the Bulletin node — because the actual request handling (looking up stored data, deciding "have" vs "don't have") happens in `sc-network`'s `BitswapServer` shim, not in the litep2p networking library itself. litep2p is a transport layer; it doesn't know what the data means. The substrate layer is where CIDs are resolved to indexed transactions, so that's where the counters belong.

These are Counters (monotonically increasing, unlike the Gauges above). Use `rate()` in Grafana to see per-second values.

- `substrate_bitswap_requests_received_total` — incoming Bitswap requests
- `substrate_bitswap_cids_requested_total` — total CIDs requested across all requests
- `substrate_bitswap_blocks_sent_total` — blocks found and sent
- `substrate_bitswap_blocks_sent_bytes_total` — bytes of block data sent
- `substrate_bitswap_blocks_not_found_total` — CIDs not found (DontHave responses)

These are available to any chain running `--ipfs-server` with the litep2p backend, not just Bulletin. See [polkadot-sdk#11370](https://github.com/paritytech/polkadot-sdk/pull/11370).

## Planned metrics

Four metrics are planned, each as a separate PR. They follow the same patterns established above — Gauges read from on-chain storage after block import.

### `bulletin_total_stored_bytes`

**Gauge: total bytes of data currently held on-chain**

A global running total. Implementation: add a `TotalStoredBytes` StorageValue to `pallet-transaction-storage`. Increment by `data.len()` in `do_store()`. Decrement when blocks expire past `RetentionPeriod` in `on_initialize()` — before removing `Transactions` for the obsolete block, sum their sizes and subtract. The node reads it like any other StorageValue.

This replaces the rough `sum_over_time(bulletin_block_store_bytes[7d])` approximation with an exact value. Combined with disk metrics, it tells operators exactly how much of their 1.5–2 TB is occupied.

Per-account breakdown is not planned — `TransactionInfo` doesn't include an owner field, and attributing data to accounts would require tracking the signer through `do_store()` plus a `TransactionOwner` double-map for pruning. That's a significant pallet refactor for minimal monitoring value.

### `bulletin_block_admin_ops`

**Gauge: number of admin operations in the latest block**

Per-block counter using the `BlockRenewCount` pattern. Implementation: add `BlockAdminOps` StorageValue to `pallet-transaction-storage` (or a shared common pallet). Increment in:
- `pallet-validator-set`: `add_validator`, `remove_validator`
- `pallet-relayer-set`: `add_relayer`, `remove_relayer`
- `pallet-transaction-storage`: `authorize_account`, `authorize_preimage`, `remove_expired_account_authorization`, `remove_expired_preimage_authorization`, `refresh_account_authorization`, `refresh_preimage_authorization`

Clear in `on_initialize()`. The node reads it the same way it reads `BlockRenewCount`.

This avoids the event-decoding problem — the node doesn't need to parse runtime events, it just reads a u32 from storage.

### `bulletin_bridge_outbound_pending`

**Gauge: messages waiting to be relayed to People Chain**

Read `OutboundLanes` storage from `pallet_bridge_messages` for lane `[0,0,0,0]`. The `OutboundLaneData` struct contains `latest_generated_nonce` (sent) and `latest_received_nonce` (confirmed delivered). Pending = generated − received. The node reads the raw storage key using the same `storage_value_key` / Blake2_128Concat pattern.

Additional gauges: `bulletin_bridge_outbound_latest_generated_nonce` and `bulletin_bridge_outbound_latest_received_nonce` for the raw nonce values.

Only available in the Polkadot runtime (solochain mode). The Westend runtime is a parachain and doesn't include bridge pallets.

### Cross-node comparison (dashboard only)

No new metric. The dashboard gets a `$instance` template variable added to all queries as `{instance=~"$instance"}`. Requires Prometheus scraping multiple nodes with distinct `instance` labels. This is a dashboard-level change — no pallet or node code needed.

## What's not here yet

- **Per-account storage breakdown** — would need `TransactionOwner(BlockNumber, Index → AccountId)` double-map plus refactoring `do_store()` to accept an account parameter. Significant pallet change for marginal monitoring value. Can be explored if there's a product need.
