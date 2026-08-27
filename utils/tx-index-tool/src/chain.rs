// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! What the chain says about a content hash, for cross-checking the database against it.
//!
//! The database can only say how many references *survive* in it. The chain knows every store
//! and renewal that ever happened, so comparing the two is what separates "this node released a
//! reference it should have kept" from "this reference was legitimately pruned".
//!
//! Only this module talks to a node. It exposes a blocking API and runs its own single-threaded
//! runtime, so the rest of the tool stays synchronous.

use crate::common::DbHash;
use std::collections::{BTreeMap, BTreeSet};
use subxt::{
	backend::legacy::LegacyRpcMethods, config::substrate::SubstrateConfig, dynamic::Value,
	OnlineClient,
};

/// The pallets whose events are relevant to an indexed entry.
const PALLETS: [&str; 2] = ["TransactionStorage", "DataRenewal"];

/// What the chain recorded for one block, as far as one content hash is concerned.
#[derive(Debug, Clone, Default)]
pub struct BlockFacts {
	/// Whether `Transactions(number)` was readable at all. False means the node has pruned the
	/// state for that height, so silence there is not evidence of absence.
	pub state_readable: bool,
	/// Whether `Transactions(number)` holds an entry for the traced hash.
	pub has_entry: bool,
	/// The chunk root the chain committed to, when an entry was found. Feeds
	/// `proof --expect-root`.
	pub chunk_root: Option<DbHash>,
	/// `Pallet.Variant` for every event in the block that mentions the hash. This is the whole
	/// story for an indexed entry: `TransactionStorage.Stored` for a store,
	/// `DataRenewal.DataRenewed` for a renewal, `DataRenewal.RenewalFailed` for one that did
	/// not take.
	pub events: Vec<String>,
}

impl BlockFacts {
	/// Whether the chain says this block took a reference on the hash.
	pub fn took_a_reference(&self) -> bool {
		self.has_entry
	}

	/// A short description of what happened, for the report's `chain` column.
	pub fn summary(&self) -> String {
		let mut parts: Vec<String> = Vec::new();
		if self.has_entry {
			parts.push("entry".to_string());
		} else if self.state_readable {
			parts.push("no entry".to_string());
		} else {
			parts.push("state pruned".to_string());
		}
		parts.extend(self.events.iter().cloned());
		parts.join(" ")
	}
}

/// Chain-side context for a trace.
#[derive(Debug, Clone)]
pub struct ChainFacts {
	/// The endpoint this came from.
	pub url: String,
	/// Best block the node reports.
	pub head: u32,
	/// Finalized block the node reports.
	pub finalized: u32,
	/// `TransactionStorage::RetentionPeriod`, if readable.
	pub retention_period: Option<u32>,
	/// `TransactionByContentHash(hash)` — where the chain currently thinks the entry lives.
	pub latest_location: Option<(u32, u32)>,
	/// Per-block findings, keyed by block number.
	pub per_block: BTreeMap<u32, BlockFacts>,
	/// Blocks that were asked about but could not be resolved at all.
	pub unresolved: Vec<u32>,
}

impl ChainFacts {
	/// The renewal cadence: auto-renewal fires one block after the retention boundary.
	pub fn cadence(&self) -> Option<u32> {
		self.retention_period.map(|rp| rp.saturating_add(1))
	}

	/// When the proof for `block`'s entry comes due — the height at which a collator must be
	/// able to read the value, or it cannot author.
	pub fn proof_due_at(&self, block: u32) -> Option<u32> {
		self.retention_period.map(|rp| block.saturating_add(rp))
	}
}

/// Fetch everything the chain can tell us about `hash` around `blocks`.
///
/// `probe_cadence` additionally walks the renewal cadence outwards from the blocks already
/// known, which is how renewals this database no longer references get found. `max_blocks`
/// bounds the number of heights queried — a chain with a long retention period has far too
/// many live entries to enumerate.
pub fn fetch(
	url: &str,
	hash: DbHash,
	blocks: &[u32],
	probe_cadence: bool,
	max_blocks: usize,
) -> std::io::Result<ChainFacts> {
	let rt = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.map_err(|e| std::io::Error::other(format!("tokio runtime: {e}")))?;
	rt.block_on(fetch_async(url, hash, blocks, probe_cadence, max_blocks))
		.map_err(|e| std::io::Error::other(format!("{url}: {e}")))
}

async fn fetch_async(
	url: &str,
	hash: DbHash,
	blocks: &[u32],
	probe_cadence: bool,
	max_blocks: usize,
) -> Result<ChainFacts, Box<dyn std::error::Error>> {
	let rpc_client = subxt::backend::rpc::RpcClient::from_url(url).await?;
	let legacy = LegacyRpcMethods::<SubstrateConfig>::new(rpc_client.clone());
	let client = OnlineClient::<SubstrateConfig>::from_rpc_client(rpc_client).await?;

	let best = legacy
		.chain_get_header(None)
		.await?
		.ok_or("node returned no best header")?
		.number;
	let finalized_hash = legacy.chain_get_finalized_head().await?;
	let finalized = legacy
		.chain_get_header(Some(finalized_hash))
		.await?
		.map(|h| h.number)
		.unwrap_or(best);

	let mut facts = ChainFacts {
		url: url.to_string(),
		head: best,
		finalized,
		retention_period: None,
		latest_location: None,
		per_block: BTreeMap::new(),
		unresolved: Vec::new(),
	};

	// RetentionPeriod and the current location of the entry: two reads at the best block.
	let at_best = client.storage().at(legacy
		.chain_get_block_hash(Some(best.into()))
		.await?
		.ok_or("no hash for best block")?);

	let rp_addr =
		subxt::dynamic::storage("TransactionStorage", "RetentionPeriod", Vec::<Value>::new());
	if let Ok(Some(v)) = at_best.fetch(&rp_addr).await {
		let bytes = v.encoded();
		if bytes.len() >= 4 {
			facts.retention_period =
				Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
		}
	}

	let loc_addr = subxt::dynamic::storage(
		"TransactionStorage",
		"TransactionByContentHash",
		vec![Value::from_bytes(hash.as_ref())],
	);
	if let Ok(Some(v)) = at_best.fetch(&loc_addr).await {
		let bytes = v.encoded();
		// `(BlockNumber, u32)` — two little-endian u32s for this runtime's block number type.
		if bytes.len() >= 8 {
			facts.latest_location = Some((
				u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
				u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
			));
		}
	}

	// Build the set of heights worth asking about.
	let mut wanted: BTreeSet<u32> = blocks.iter().copied().collect();
	if let Some((block, _)) = facts.latest_location {
		wanted.insert(block);
	}
	if probe_cadence {
		if let Some(cadence) = facts.cadence() {
			// Half the budget going back towards the original store, the rest forwards to the
			// head. Probes that turn up nothing are dropped when the facts are merged, so an
			// over-generous walk costs round-trips rather than noise.
			let steps = (max_blocks / 2).max(1);
			for anchor in wanted.iter().copied().collect::<Vec<_>>() {
				let mut n = anchor;
				for _ in 0..steps {
					if n <= cadence {
						break;
					}
					n -= cadence;
					wanted.insert(n);
				}
				let mut n = anchor;
				while n + cadence <= best {
					n += cadence;
					wanted.insert(n);
				}
			}
		}
	}
	let wanted: Vec<u32> = wanted.into_iter().take(max_blocks).collect();

	let target = hash.as_ref().to_vec();
	for number in wanted {
		let Some(block_hash) = legacy.chain_get_block_hash(Some(number.into())).await? else {
			facts.unresolved.push(number);
			continue;
		};
		let mut bf = BlockFacts::default();

		// `Transactions(number)` read at that block's own state, where the entry is freshest.
		let addr = subxt::dynamic::storage(
			"TransactionStorage",
			"Transactions",
			vec![Value::u128(number as u128)],
		);
		match client.storage().at(block_hash).fetch(&addr).await {
			Ok(opt) => {
				bf.state_readable = true;
				if let Some(v) = opt {
					let bytes = v.encoded();
					if let Some(off) = find(bytes, &target) {
						bf.has_entry = true;
						// `TransactionInfo` starts with `chunk_root` then `content_hash`, so the
						// root is the 32 bytes immediately before the match.
						if off >= 32 {
							bf.chunk_root = Some(DbHash::from_slice(&bytes[off - 32..off]));
						}
					}
				}
			},
			// A pruned-state read fails rather than returning None; that distinction matters.
			Err(_) => bf.state_readable = false,
		}

		// Events mentioning the hash. Read `System::Events` directly and decode it with the
		// runtime metadata rather than going through `blocks().at()`, which pulls the whole
		// body: a block full of `store` calls carries megabytes of payload and trips
		// jsonrpsee's response-size limit. The events blob is small, and it is where the
		// answer lives — an auto-renewal fires in `on_initialize` and has no extrinsic at all.
		let events_addr = subxt::dynamic::storage("System", "Events", Vec::<Value>::new());
		if let Ok(Some(v)) = client.storage().at(block_hash).fetch(&events_addr).await {
			let events = subxt::events::Events::<SubstrateConfig>::decode_from(
				v.encoded().to_vec(),
				client.metadata(),
			);
			for ev in events.iter().flatten() {
				if !PALLETS.contains(&ev.pallet_name()) {
					continue;
				}
				if ev.field_bytes().windows(32).any(|w| w == target.as_slice()) {
					bf.events.push(format!("{}.{}", ev.pallet_name(), ev.variant_name()));
				}
			}
		}

		facts.per_block.insert(number, bf);
	}

	Ok(facts)
}

/// First offset at which `needle` occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	if needle.is_empty() || haystack.len() < needle.len() {
		return None;
	}
	haystack.windows(needle.len()).position(|w| w == needle)
}
