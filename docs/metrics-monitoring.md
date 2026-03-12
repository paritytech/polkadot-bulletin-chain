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

## What's not here yet

Bulletin serves stored data over IPFS via litep2p's Bitswap protocol, but there's no way to monitor that today. The bitswap handler in litep2p has no metrics instrumentation — no request counts, no "not found" tracking, no Prometheus integration. It just processes requests silently.

To get visibility into IPFS serving, someone would need to add counters to `litep2p/src/protocol/libp2p/bitswap/mod.rs` in the [litep2p crate](https://github.com/paritytech/litep2p). That would give us metrics like requests received and blocks not found, which we could then expose from the Bulletin node.
