// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Comparison of two databases: which entries one holds that the other doesn't, and where the
//! two disagree about an entry they share.

use crate::common::*;
use codec::Decode;
use kvdb::KeyValueDB;
use std::{
	collections::{HashMap, HashSet},
	fmt,
	time::{Duration, Instant},
};

/// What a comparison needs to know about one entry in one database. Values themselves are never
/// held: col11 is content-addressed, so two entries that verify under the same key are
/// byte-identical by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryFacts {
	/// Size of the stored value in bytes.
	pub size: usize,
	/// Refcount, or `None` when the counter row is missing.
	pub counter: Option<u32>,
	/// Whether the value hashes to the key it is filed under.
	pub verified: bool,
}

/// One entry that the two databases disagree about. `None` means the database doesn't have it.
#[derive(Debug, Clone)]
pub struct EntryDiff {
	/// The col11 slot key.
	pub content_hash: DbHash,
	/// Facts from the first database.
	pub a: Option<EntryFacts>,
	/// Facts from the second database.
	pub b: Option<EntryFacts>,
}

impl EntryDiff {
	/// Whether the refcounts disagree (only meaningful when both sides have the entry).
	pub fn refcount_differs(&self) -> bool {
		matches!((self.a, self.b), (Some(a), Some(b)) if a.counter != b.counter)
	}

	/// Whether the stored sizes disagree — one side's bytes are not the other's.
	pub fn size_differs(&self) -> bool {
		matches!((self.a, self.b), (Some(a), Some(b)) if a.size != b.size)
	}

	/// Whether the entry verifies on one side and not the other.
	pub fn integrity_differs(&self) -> bool {
		matches!((self.a, self.b), (Some(a), Some(b)) if a.verified != b.verified)
	}
}

/// Per-block comparison of `BODY_INDEX`, gathered only when asked for.
#[derive(Debug, Default)]
pub struct BlockDiff {
	/// Blocks with an indexed body in the first database.
	pub bodies_a: usize,
	/// Blocks with an indexed body in the second.
	pub bodies_b: usize,
	/// Blocks whose body only the first database has — the shape that leaves a collator unable
	/// to build a storage proof.
	pub only_in_a: Vec<u32>,
	/// Blocks whose body only the second database has.
	pub only_in_b: Vec<u32>,
	/// Blocks both have, but referencing different hashes.
	pub refs_differ: Vec<u32>,
}

/// What to compare.
#[derive(Debug, Clone)]
pub struct DiffOptions {
	/// Cap on per-entry lines rendered (`None` prints all of them).
	pub limit: Option<usize>,
	/// Also walk `BODY_INDEX` in both databases and compare per block.
	pub blocks: bool,
}

impl Default for DiffOptions {
	fn default() -> Self {
		Self { limit: Some(50), blocks: false }
	}
}

/// Result of `diff_databases`.
#[derive(Debug)]
pub struct DiffReport {
	/// Chain head of each database, as recorded in META.
	pub best_a: Option<(u32, DbHash)>,
	/// Chain head of the second database.
	pub best_b: Option<(u32, DbHash)>,
	/// col11 value rows in each.
	pub entries_a: u64,
	/// col11 value rows in the second.
	pub entries_b: u64,
	/// Bytes stored in each.
	pub bytes_a: u64,
	/// Bytes stored in the second.
	pub bytes_b: u64,
	/// Every entry the two disagree about, sorted by content hash.
	pub rows: Vec<EntryDiff>,
	/// How many rows there were before `limit` truncated them.
	pub differing: usize,
	/// Entries only the first database has. Counted before `limit`, unlike `rows`.
	pub only_in_a: usize,
	/// Entries only the second database has.
	pub only_in_b: usize,
	/// Shared entries whose refcounts disagree.
	pub refcount_differs: usize,
	/// Shared entries whose stored sizes disagree.
	pub size_differs: usize,
	/// Shared entries that verify on one side only.
	pub integrity_differs: usize,
	/// Per-block comparison, when `DiffOptions::blocks` was set.
	pub blocks: Option<BlockDiff>,
	/// Wall-clock duration.
	pub elapsed: Duration,
}

impl DiffReport {
	/// True when the two databases agree about every entry (and every block, if compared).
	pub fn is_identical(&self) -> bool {
		self.differing == 0 &&
			self.blocks.as_ref().is_none_or(|b| {
				b.only_in_a.is_empty() && b.only_in_b.is_empty() && b.refs_differ.is_empty()
			})
	}
}

/// Read one database's col11 into the facts a comparison needs: size, refcount and whether the
/// value hashes to its key.
fn column_facts(db: &dyn KeyValueDB) -> std::io::Result<(HashMap<DbHash, EntryFacts>, u64, u64)> {
	let mut facts: HashMap<DbHash, EntryFacts> = HashMap::new();
	let mut counters: HashMap<DbHash, u32> = HashMap::new();
	let mut entries = 0u64;
	let mut bytes = 0u64;

	for entry in db.iter(columns::TRANSACTION) {
		let (k, v) = entry?;
		match k.len() {
			32 => {
				entries += 1;
				bytes += v.len() as u64;
				let mut key = [0u8; 32];
				key.copy_from_slice(&k);
				let content_hash = DbHash::from(key);
				let verified = HashAlgo::identify(content_hash, &v).is_some();
				facts.insert(content_hash, EntryFacts { size: v.len(), counter: None, verified });
			},
			33 if k[32] == 0 => {
				let mut key = [0u8; 32];
				key.copy_from_slice(&k[..32]);
				if let Ok(bytes) = <[u8; 4]>::try_from(&v[..]) {
					counters.insert(DbHash::from(key), u32::from_le_bytes(bytes));
				}
			},
			_ => {},
		}
	}

	for (hash, counter) in counters {
		if let Some(f) = facts.get_mut(&hash) {
			f.counter = Some(counter);
		}
	}
	Ok((facts, entries, bytes))
}

/// Which blocks have an indexed body, and which hashes each references.
fn body_index_map(db: &dyn KeyValueDB) -> std::io::Result<HashMap<u32, Vec<DbHash>>> {
	let mut bodies: HashMap<u32, Vec<DbHash>> = HashMap::new();
	for entry in db.iter(columns::BODY_INDEX) {
		let (k, v) = entry?;
		let Some((number, _)) = split_lookup_key(&k) else { continue };
		let Ok(index) = Vec::<BareDbExtrinsic>::decode(&mut &v[..]) else { continue };
		let mut hashes = Vec::new();
		for ex in index {
			match ex {
				BareDbExtrinsic::Indexed { hash, .. } => hashes.push(hash),
				BareDbExtrinsic::MultiRenew { hashes: inner, .. } => hashes.extend(inner),
				BareDbExtrinsic::Full(_) => {},
			}
		}
		hashes.sort_unstable();
		bodies.insert(number, hashes);
	}
	Ok(bodies)
}

/// Compare the indexed transaction storage of two databases.
///
/// Both are read-only. Values are compared through their keys rather than their bytes: col11 is
/// content-addressed, so a differing size or a differing verification result is the signal, and
/// two entries that verify under the same key hold the same bytes.
pub fn diff_databases(
	a: &dyn KeyValueDB,
	b: &dyn KeyValueDB,
	opts: &DiffOptions,
) -> std::io::Result<DiffReport> {
	let started = Instant::now();

	let (facts_a, entries_a, bytes_a) = column_facts(a)?;
	let (facts_b, entries_b, bytes_b) = column_facts(b)?;

	let mut hashes: Vec<DbHash> = facts_a
		.keys()
		.chain(facts_b.keys())
		.copied()
		.collect::<HashSet<_>>()
		.into_iter()
		.collect();
	hashes.sort_unstable();

	let (mut only_in_a, mut only_in_b) = (0, 0);
	let (mut refcount_differs, mut size_differs, mut integrity_differs) = (0, 0, 0);
	let mut rows: Vec<EntryDiff> = Vec::new();
	for content_hash in hashes {
		let (a, b) = (facts_a.get(&content_hash).copied(), facts_b.get(&content_hash).copied());
		if a == b {
			continue;
		}
		let row = EntryDiff { content_hash, a, b };
		// Tally before `limit` truncates, so the summary describes the whole comparison.
		only_in_a += usize::from(row.b.is_none());
		only_in_b += usize::from(row.a.is_none());
		refcount_differs += usize::from(row.refcount_differs());
		size_differs += usize::from(row.size_differs());
		integrity_differs += usize::from(row.integrity_differs());
		rows.push(row);
	}
	let differing = rows.len();
	if let Some(limit) = opts.limit {
		rows.truncate(limit);
	}

	let blocks = if opts.blocks {
		let bodies_a = body_index_map(a)?;
		let bodies_b = body_index_map(b)?;
		let mut diff =
			BlockDiff { bodies_a: bodies_a.len(), bodies_b: bodies_b.len(), ..Default::default() };
		for (number, refs_a) in &bodies_a {
			match bodies_b.get(number) {
				None => diff.only_in_a.push(*number),
				Some(refs_b) if refs_b != refs_a => diff.refs_differ.push(*number),
				Some(_) => {},
			}
		}
		for number in bodies_b.keys() {
			if !bodies_a.contains_key(number) {
				diff.only_in_b.push(*number);
			}
		}
		diff.only_in_a.sort_unstable();
		diff.only_in_b.sort_unstable();
		diff.refs_differ.sort_unstable();
		Some(diff)
	} else {
		None
	};

	Ok(DiffReport {
		best_a: best_block(a)?,
		best_b: best_block(b)?,
		entries_a,
		entries_b,
		bytes_a,
		bytes_b,
		rows,
		differing,
		only_in_a,
		only_in_b,
		refcount_differs,
		size_differs,
		integrity_differs,
		blocks,
		elapsed: started.elapsed(),
	})
}

impl fmt::Display for DiffReport {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let head = |best: &Option<(u32, DbHash)>| match best {
			Some((number, _)) => format!("#{number}"),
			None => "<no META entry>".to_string(),
		};

		writeln!(f, "Database comparison (A vs B)")?;
		writeln!(f, "============================")?;
		writeln!(f, "Elapsed:               {:?}", self.elapsed)?;
		writeln!(f, "Best block:            A {}   B {}", head(&self.best_a), head(&self.best_b))?;
		writeln!(
			f,
			"col11 entries:         A {}   B {}   ({} / {})",
			self.entries_a,
			self.entries_b,
			human_bytes(self.bytes_a),
			human_bytes(self.bytes_b),
		)?;
		writeln!(f, "Entries differing:     {}", self.differing)?;
		writeln!(f, "  only in A:           {}", self.only_in_a)?;
		writeln!(f, "  only in B:           {}", self.only_in_b)?;
		writeln!(f, "  refcount differs:    {}", self.refcount_differs)?;
		writeln!(f, "  size differs:        {}", self.size_differs)?;
		writeln!(f, "  integrity differs:   {}", self.integrity_differs)?;

		if let Some(blocks) = &self.blocks {
			writeln!(f)?;
			writeln!(
				f,
				"Blocks with an indexed body: A {}   B {}",
				blocks.bodies_a, blocks.bodies_b,
			)?;
			writeln!(
				f,
				"  only in A ({}): {}",
				blocks.only_in_a.len(),
				joined_blocks(&blocks.only_in_a, 20),
			)?;
			writeln!(
				f,
				"  only in B ({}): {}",
				blocks.only_in_b.len(),
				joined_blocks(&blocks.only_in_b, 20),
			)?;
			writeln!(
				f,
				"  referencing different hashes ({}): {}",
				blocks.refs_differ.len(),
				joined_blocks(&blocks.refs_differ, 20),
			)?;
		}

		if self.is_identical() {
			writeln!(f)?;
			writeln!(f, "Result: IDENTICAL — both databases agree about every entry compared.")?;
			return Ok(());
		}

		if !self.rows.is_empty() {
			writeln!(f)?;
			for r in &self.rows {
				writeln!(f, "  {}", hex(r.content_hash.as_ref()))?;
				match (r.a, r.b) {
					(Some(a), None) => writeln!(
						f,
						"    only in A    {} , refcount {}{}",
						human_bytes(a.size as u64),
						a.counter.map(|c| c.to_string()).unwrap_or_else(|| "<absent>".into()),
						if a.verified { "" } else { "  (does not verify)" },
					)?,
					(None, Some(b)) => writeln!(
						f,
						"    only in B    {} , refcount {}{}",
						human_bytes(b.size as u64),
						b.counter.map(|c| c.to_string()).unwrap_or_else(|| "<absent>".into()),
						if b.verified { "" } else { "  (does not verify)" },
					)?,
					(Some(a), Some(b)) => {
						if a.counter != b.counter {
							writeln!(
								f,
								"    refcount     A {}   B {}",
								a.counter
									.map(|c| c.to_string())
									.unwrap_or_else(|| "<absent>".into()),
								b.counter
									.map(|c| c.to_string())
									.unwrap_or_else(|| "<absent>".into()),
							)?;
						}
						if a.size != b.size {
							writeln!(f, "    size         A {}   B {}", a.size, b.size)?;
						}
						if a.verified != b.verified {
							writeln!(
								f,
								"    integrity    A {}   B {}",
								if a.verified { "ok" } else { "CORRUPTED" },
								if b.verified { "ok" } else { "CORRUPTED" },
							)?;
						}
					},
					(None, None) => {},
				}
			}
		}

		if self.differing > self.rows.len() {
			writeln!(f)?;
			writeln!(
				f,
				"… {} more differing entries cut by --limit.",
				self.differing - self.rows.len()
			)?;
		}
		Ok(())
	}
}
