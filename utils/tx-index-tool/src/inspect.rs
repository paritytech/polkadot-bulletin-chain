// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! What a single block's body references in col11, and the on-disk state of each entry.

use crate::common::*;
use codec::Decode;
use kvdb::KeyValueDB;
use std::{collections::HashMap, fmt};

/// Per-hash inspection result for a block: the hash, how many times it appears in the body
/// (Indexed vs MultiRenew), and what the on-disk col11 state says about it.
#[derive(Debug, Clone)]
pub struct BlockHashInfo {
	/// The col11 content hash this block references.
	pub content_hash: DbHash,
	/// `Indexed` extrinsics in the block referencing it.
	pub indexed: u32,
	/// One entry per `MultiRenew` in the block, holding its inner occurrence count.
	pub multirenew_inner: Vec<u32>,
	/// On-disk refcount (None if no counter entry exists).
	pub on_disk_counter: Option<u32>,
	/// Length in bytes of the on-disk value entry (None if absent).
	pub on_disk_value_size: Option<usize>,
}

/// Full result of `inspect_block`.
#[derive(Debug, Clone)]
pub struct BlockInspection {
	/// Block number that was inspected.
	pub number: u32,
	/// Block hash recovered from `KEY_LOOKUP`.
	pub hash: DbHash,
	/// Whether the block's `BODY_INDEX` entry decoded successfully.
	pub body_index_decoded: bool,
	/// The decode error, when `body_index_decoded` is false.
	pub decode_failure: Option<String>,
	/// Number of `Indexed` variants found (informational).
	pub indexed_entries: usize,
	/// Number of `MultiRenew` variants found (informational).
	pub multirenew_entries: usize,
	/// Number of `Full` variants found (informational).
	pub full_entries: usize,
	/// Per content-hash details for hashes appearing in the body index.
	pub hashes: Vec<BlockHashInfo>,
}

/// Look up a block by number and report on every col11 content-hash it references, including
/// the live on-disk counter and value-presence for each. Read-only; safe against a stopped node.
pub fn inspect_block(
	db: &dyn KeyValueDB,
	block_number: u32,
) -> std::io::Result<Option<BlockInspection>> {
	let Some((block_hash, lookup_key)) = block_lookup_key(db, block_number)? else {
		return Ok(None);
	};

	let body_index_bytes = db.get(columns::BODY_INDEX, &lookup_key)?;
	let mut inspection = BlockInspection {
		number: block_number,
		hash: block_hash,
		body_index_decoded: false,
		decode_failure: None,
		indexed_entries: 0,
		multirenew_entries: 0,
		full_entries: 0,
		hashes: Vec::new(),
	};

	let Some(bytes) = body_index_bytes else {
		// No indexed body. Block may have a plain body (col5) or no body at all.
		return Ok(Some(inspection));
	};

	let index = match Vec::<BareDbExtrinsic>::decode(&mut &bytes[..]) {
		Ok(idx) => idx,
		Err(e) => {
			inspection.decode_failure = Some(format!("{e:?}"));
			return Ok(Some(inspection));
		},
	};
	inspection.body_index_decoded = true;

	let mut per_hash: HashMap<DbHash, Occurrences> = HashMap::new();
	for ex in index {
		match ex {
			BareDbExtrinsic::Indexed { hash, .. } => {
				inspection.indexed_entries += 1;
				per_hash.entry(hash).or_default().indexed += 1;
			},
			BareDbExtrinsic::MultiRenew { hashes, .. } => {
				inspection.multirenew_entries += 1;
				let mut inner: HashMap<DbHash, u32> = HashMap::new();
				for h in hashes {
					*inner.entry(h).or_default() += 1;
				}
				for (h, n) in inner {
					per_hash.entry(h).or_default().multirenew_inner.push(n);
				}
			},
			BareDbExtrinsic::Full(_) => {
				inspection.full_entries += 1;
			},
		}
	}

	// For each referenced hash, look up the col11 counter and value.
	let mut hashes: Vec<_> = per_hash.into_iter().collect();
	hashes.sort_by(|a, b| a.0.cmp(&b.0));
	for (hash, occ) in hashes {
		let on_disk_counter = read_counter(db, &hash)?;
		let value = db.get(columns::TRANSACTION, hash.as_ref())?;
		inspection.hashes.push(BlockHashInfo {
			content_hash: hash,
			indexed: occ.indexed,
			multirenew_inner: occ.multirenew_inner,
			on_disk_counter,
			on_disk_value_size: value.as_ref().map(|v| v.len()),
		});
	}

	Ok(Some(inspection))
}

impl fmt::Display for BlockInspection {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let block_hash_hex = hex(self.hash.as_ref());
		writeln!(f, "Block #{} ({})", self.number, block_hash_hex)?;
		writeln!(
			f,
			"  body extrinsics: {} Indexed, {} MultiRenew, {} Full",
			self.indexed_entries, self.multirenew_entries, self.full_entries,
		)?;
		if let Some(err) = &self.decode_failure {
			writeln!(f, "  BODY_INDEX decode failed: {err}")?;
		}
		if !self.body_index_decoded {
			writeln!(f, "  no BODY_INDEX entry for this block (plain body or no body)")?;
			return Ok(());
		}
		if self.hashes.is_empty() {
			writeln!(f, "  no col11 content hashes referenced (all extrinsics are Full)")?;
			return Ok(());
		}
		writeln!(f, "  col11 content hashes referenced:")?;
		for h in &self.hashes {
			let hex = hex(h.content_hash.as_ref());
			let occ =
				Occurrences { indexed: h.indexed, multirenew_inner: h.multirenew_inner.clone() };
			let counter_str = match h.on_disk_counter {
				Some(c) => format!("counter={c}"),
				None => "counter=<absent>".into(),
			};
			let value_str = match h.on_disk_value_size {
				Some(n) => format!("value={} bytes", n),
				None => "value=<absent>".into(),
			};
			writeln!(f, "    {hex}",)?;
			writeln!(
				f,
				"      body shape: {}    on-disk: {counter_str}, {value_str}",
				fmt_occurrences(&occ),
			)?;
		}
		Ok(())
	}
}
