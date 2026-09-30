// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Listing of the transaction data col11 currently holds, with an integrity check per
//! entry.

use crate::common::*;
use codec::Decode;
use kvdb::KeyValueDB;
use std::{
	collections::HashMap,
	fmt,
	time::{Duration, Instant},
};

/// Ordering key for `list_entries`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySort {
	/// By the lowest-numbered block referencing the entry (i.e. when it was stored).
	FirstBlock,
	/// By value size in bytes.
	Size,
	/// By the on-disk refcount counter.
	RefCount,
	/// By content hash, lexicographically.
	Hash,
}

impl EntrySort {
	/// Parse a sort key from its CLI spelling.
	pub fn parse(s: &str) -> Result<Self, String> {
		match s.to_lowercase().as_str() {
			"block" | "first-block" | "first_block" | "created" => Ok(EntrySort::FirstBlock),
			"size" | "bytes" => Ok(EntrySort::Size),
			"refcount" | "refs" | "counter" => Ok(EntrySort::RefCount),
			"hash" | "key" => Ok(EntrySort::Hash),
			other =>
				Err(format!("unknown sort key: {other} (expected: block | size | refcount | hash)")),
		}
	}

	/// Human-readable name, used in the report header.
	pub fn name(&self) -> &'static str {
		match self {
			EntrySort::FirstBlock => "first block",
			EntrySort::Size => "size",
			EntrySort::RefCount => "refcount",
			EntrySort::Hash => "hash",
		}
	}
}

/// Filters and formatting knobs for `list_entries`.
///
/// The filters apply in three tiers:
///
/// 1. `hash_filter` / `block_filter` pre-select keys, so the column is never walked and the
///    report's column-wide totals are left unset (`column_scanned == false`).
/// 2. `min_size` / `corrupted_only` are applied per value as it is read, so a skipped entry still
///    counts toward `total_bytes` but not toward `matched`.
/// 3. `orphans_only` / `from_block` / `to_block` are applied after the `BODY_INDEX` pass, so they
///    need `resolve_blocks`.
#[derive(Debug, Clone)]
pub struct ListOptions {
	/// Ordering key.
	pub sort: EntrySort,
	/// Reverse the ordering (largest / newest first).
	pub descending: bool,
	/// Cap on how many entries are rendered (`None` lists everything).
	pub limit: Option<usize>,
	/// How many leading bytes of each value to capture for the hexdump preview (0 disables).
	pub preview_len: usize,
	/// Only entries whose value doesn't hash to its slot key under any known algorithm.
	pub corrupted_only: bool,
	/// Only entries that no alive block references any more.
	pub orphans_only: bool,
	/// Skip entries smaller than this many bytes.
	pub min_size: usize,
	/// Restrict the listing to a single content hash.
	pub hash_filter: Option<DbHash>,
	/// Restrict the listing to the entries referenced by one block's body.
	pub block_filter: Option<u32>,
	/// Only entries first stored at or after this block. Needs `resolve_blocks`.
	pub from_block: Option<u32>,
	/// Only entries first stored at or before this block. Needs `resolve_blocks`.
	pub to_block: Option<u32>,
	/// Walk `BODY_INDEX` to resolve first/last referencing block and block timestamps.
	/// Disabling it makes the scan roughly twice as fast but leaves those columns empty.
	pub resolve_blocks: bool,
}

impl Default for ListOptions {
	fn default() -> Self {
		Self {
			sort: EntrySort::FirstBlock,
			descending: false,
			limit: Some(50),
			preview_len: 16,
			corrupted_only: false,
			orphans_only: false,
			min_size: 0,
			hash_filter: None,
			block_filter: None,
			from_block: None,
			to_block: None,
			resolve_blocks: true,
		}
	}
}

/// One transaction-storage entry as it currently sits in col11.
#[derive(Debug, Clone)]
pub struct StorageEntry {
	/// The 32-byte slot key — the content hash the data was stored under.
	pub content_hash: DbHash,
	/// Size of the stored value in bytes.
	pub size: usize,
	/// Which algorithm reproduces `content_hash` from the value. `None` means none of the
	/// known algorithms do — the entry is corrupted.
	pub algo: Option<HashAlgo>,
	/// What the value actually hashes to under each known algorithm. Only filled in when the
	/// entry failed verification, where it's the evidence an operator needs.
	pub computed_hashes: Vec<(HashAlgo, DbHash)>,
	/// On-disk refcount at `TRANSACTION[hash‖0x00]` (`None` if the counter row is absent).
	pub counter: Option<u32>,
	/// How many alive blocks reference this hash in their `BODY_INDEX`.
	pub referring_blocks: u32,
	/// Lowest-numbered alive block referencing the hash — where it was (re)stored.
	pub first_block: Option<u32>,
	/// Wall-clock time of `first_block`, recovered from that block's timestamp inherent.
	pub first_block_time_ms: Option<u64>,
	/// Highest-numbered alive block referencing the hash — its most recent renewal.
	pub last_block: Option<u32>,
	/// Wall-clock time of `last_block`, recovered from that block's timestamp inherent.
	pub last_block_time_ms: Option<u64>,
	/// Every alive block referencing the hash, collected only for entries that failed
	/// verification, where the full set of referrers is the useful output. Collecting it for
	/// every entry would cost one entry per reference across the whole chain.
	pub referring_block_list: Vec<u32>,
	/// Leading bytes of the value, for the hexdump preview.
	pub preview: Vec<u8>,
}

/// Result of `list_entries`.
#[derive(Debug)]
pub struct EntriesReport {
	/// The entries that survived the filters, sorted and truncated per `ListOptions`.
	pub entries: Vec<StorageEntry>,
	/// Every 32-byte-keyed (value) row in col11, before filtering. Only meaningful when
	/// `column_scanned` is true.
	pub value_entries: u64,
	/// Every 33-byte-keyed (counter) row in col11, before filtering. Only meaningful when
	/// `column_scanned` is true.
	pub counter_entries: u64,
	/// Sum of all value sizes in col11, before filtering. Only meaningful when
	/// `column_scanned` is true.
	pub total_bytes: u64,
	/// Values that passed the integrity check during the scan, before any filter dropped them.
	/// Only meaningful when `column_scanned` is true.
	pub values_verified: u64,
	/// Values that failed it, likewise before filtering.
	pub values_corrupted: u64,
	/// col11 rows that are neither a 32-byte value nor a 33-byte `hash‖0x00` counter. Should be
	/// zero; anything else means the column holds a key shape this tool does not understand.
	/// Only meaningful when `column_scanned` is true.
	pub unexpected_key_rows: u64,
	/// Whether the whole column was walked. A `--hash` or `--block` filter resolves its
	/// entries by point lookup instead, which skips the walk — and the column-wide totals.
	pub column_scanned: bool,
	/// Authoring time of the block named by `ListOptions::block_filter`, when recoverable.
	pub block_time_ms: Option<u64>,
	/// Corrupted entries among those that matched the filters.
	pub corrupted: usize,
	/// Matched entries no alive block references any more.
	pub orphans: usize,
	/// How many entries matched the filters, before `limit` was applied.
	pub matched: usize,
	/// When `ListOptions::block_filter` is set: whether that block exists in the database.
	pub block_found: Option<bool>,
	/// The options this report was produced with, echoed for the header.
	pub options: ListOptions,
	/// Wall-clock duration of the scan.
	pub elapsed: Duration,
}

/// List the transaction-storage entries currently held in col11: size, which hash algorithm
/// the slot key was produced with, refcount, the blocks that reference it (with the wall-clock
/// time of the first and last one), and a `hexdump -C` style preview of the leading bytes.
///
/// Read-only; safe against a stopped node. Cost is one full col11 pass (every value is hashed
/// under up to three algorithms to identify the algorithm) plus, unless
/// `ListOptions::resolve_blocks` is off, one `BODY_INDEX` pass.
pub fn list_entries(db: &dyn KeyValueDB, opts: &ListOptions) -> std::io::Result<EntriesReport> {
	let started = Instant::now();

	// A `--block` filter is resolved up front, straight through KEY_LOOKUP, so the col11 pass
	// below can skip (and, more importantly, avoid hashing) everything that block doesn't touch.
	let mut block_found = None;
	let mut block_time_ms = None;
	let mut wanted: Option<std::collections::HashSet<DbHash>> = None;
	if let Some(number) = opts.block_filter {
		match block_referenced_hashes(db, number)? {
			Some((hashes, time_ms)) => {
				block_found = Some(true);
				block_time_ms = time_ms;
				wanted = Some(hashes.into_iter().collect());
			},
			None => {
				block_found = Some(false);
				wanted = Some(std::collections::HashSet::new());
			},
		}
	}
	if let Some(hash) = opts.hash_filter {
		// Both filters together mean "this hash, but only if that block references it".
		match &mut wanted {
			Some(set) => set.retain(|h| *h == hash),
			None => wanted = Some(std::iter::once(hash).collect()),
		}
	}

	let mut entries: HashMap<DbHash, StorageEntry> = HashMap::new();
	let mut counters: HashMap<DbHash, u32> = HashMap::new();
	let mut value_entries = 0u64;
	let mut counter_entries = 0u64;
	let mut unexpected_key_rows = 0u64;
	// Counted inside `build`, which only takes `&self`, so the tallies live in cells.
	let values_verified = std::cell::Cell::new(0u64);
	let values_corrupted = std::cell::Cell::new(0u64);
	let mut total_bytes = 0u64;

	let build = |content_hash: DbHash, value: &[u8]| -> Option<StorageEntry> {
		if value.len() < opts.min_size {
			return None;
		}
		// The integrity check. `identify` stops at the first algorithm that reproduces the key,
		// so a healthy entry is hashed once; only a mismatch pays for all three, and only then
		// is the evidence worth keeping.
		let algo = HashAlgo::identify(content_hash, value);
		if algo.is_some() {
			values_verified.set(values_verified.get() + 1);
		} else {
			values_corrupted.set(values_corrupted.get() + 1);
		}
		if opts.corrupted_only && algo.is_some() {
			return None;
		}
		Some(StorageEntry {
			content_hash,
			size: value.len(),
			algo,
			computed_hashes: if algo.is_none() { HashAlgo::hash_all(value) } else { Vec::new() },
			counter: None,
			referring_blocks: 0,
			first_block: None,
			first_block_time_ms: None,
			last_block: None,
			last_block_time_ms: None,
			referring_block_list: Vec::new(),
			preview: value[..value.len().min(opts.preview_len)].to_vec(),
		})
	};

	// Pass 1: gather the candidate values. With a `--hash` or `--block` filter the set of keys
	// is already known, so point lookups replace the column walk — on a multi-gigabyte col11
	// that's the difference between milliseconds and minutes.
	let column_scanned = wanted.is_none();
	if let Some(targets) = &wanted {
		for hash in targets {
			let Some(value) = db.get(columns::TRANSACTION, hash.as_ref())? else { continue };
			if let Some(entry) = build(*hash, &value) {
				entries.insert(*hash, entry);
			}
		}
	} else {
		// 32-byte keys carry the values, 33-byte keys (`hash‖0x00`) the counters.
		for entry in db.iter(columns::TRANSACTION) {
			let (k, v) = entry?;
			match k.len() {
				32 => {
					value_entries += 1;
					total_bytes += v.len() as u64;
					let mut key_bytes = [0u8; 32];
					key_bytes.copy_from_slice(&k);
					let content_hash = DbHash::from(key_bytes);
					if let Some(entry) = build(content_hash, &v) {
						entries.insert(content_hash, entry);
					}
				},
				33 if k[32] == 0 => {
					counter_entries += 1;
					let mut key_bytes = [0u8; 32];
					key_bytes.copy_from_slice(&k[..32]);
					if let Ok(bytes) = <[u8; 4]>::try_from(&v[..]) {
						counters.insert(DbHash::from(key_bytes), u32::from_le_bytes(bytes));
					}
				},
				// Anything that is neither a 32-byte value nor a `hash‖0x00` counter row.
				_ => unexpected_key_rows += 1,
			}
		}
	}

	if column_scanned {
		for (hash, counter) in counters {
			if let Some(e) = entries.get_mut(&hash) {
				e.counter = Some(counter);
			}
		}
	} else {
		for (hash, e) in entries.iter_mut() {
			e.counter = read_counter(db, hash)?;
		}
	}

	// Pass 2: BODY_INDEX, to find who references each entry and when those blocks were authored.
	if opts.resolve_blocks && !entries.is_empty() {
		for entry in db.iter(columns::BODY_INDEX) {
			let (k, v) = entry?;
			let Some((number, _)) = split_lookup_key(&k) else { continue };
			let Ok(index) = Vec::<BareDbExtrinsic>::decode(&mut &v[..]) else { continue };

			let mut touched: Vec<DbHash> = Vec::new();
			let mut fulls: Vec<Vec<u8>> = Vec::new();
			for ex in index {
				match ex {
					BareDbExtrinsic::Indexed { hash, .. } =>
						if entries.contains_key(&hash) {
							touched.push(hash);
						},
					BareDbExtrinsic::MultiRenew { hashes, .. } =>
						for h in hashes {
							if entries.contains_key(&h) {
								touched.push(h);
							}
						},
					BareDbExtrinsic::Full(bytes) => fulls.push(bytes),
				}
			}
			if touched.is_empty() {
				continue;
			}
			touched.sort_unstable();
			touched.dedup();

			let time_ms = block_timestamp_ms(&fulls);
			for h in touched {
				let Some(e) = entries.get_mut(&h) else { continue };
				e.referring_blocks += 1;
				if e.algo.is_none() {
					e.referring_block_list.push(number);
				}
				if e.first_block.is_none_or(|n| number < n) {
					e.first_block = Some(number);
					e.first_block_time_ms = time_ms;
				}
				if e.last_block.is_none_or(|n| number > n) {
					e.last_block = Some(number);
					e.last_block_time_ms = time_ms;
				}
			}
		}
	}

	let mut list: Vec<StorageEntry> = entries.into_values().collect();
	for e in &mut list {
		e.referring_block_list.sort_unstable();
	}
	if opts.orphans_only {
		list.retain(|e| e.referring_blocks == 0);
	}
	// The range is on where an entry was *stored*, so an entry with no known first block —
	// an orphan, or any entry when block resolution is off — cannot satisfy it.
	if opts.from_block.is_some() || opts.to_block.is_some() {
		let from = opts.from_block.unwrap_or(0);
		let to = opts.to_block.unwrap_or(u32::MAX);
		list.retain(|e| e.first_block.is_some_and(|n| n >= from && n <= to));
	}
	let corrupted = list.iter().filter(|e| e.algo.is_none()).count();
	let orphans = list.iter().filter(|e| e.referring_blocks == 0).count();
	let matched = list.len();

	let by_hash = |a: &StorageEntry, b: &StorageEntry| a.content_hash.cmp(&b.content_hash);
	match opts.sort {
		EntrySort::FirstBlock =>
			list.sort_by(|a, b| a.first_block.cmp(&b.first_block).then_with(|| by_hash(a, b))),
		EntrySort::Size => list.sort_by(|a, b| a.size.cmp(&b.size).then_with(|| by_hash(a, b))),
		EntrySort::RefCount =>
			list.sort_by(|a, b| a.counter.cmp(&b.counter).then_with(|| by_hash(a, b))),
		EntrySort::Hash => list.sort_by(by_hash),
	}
	if opts.descending {
		list.reverse();
	}
	if let Some(n) = opts.limit {
		list.truncate(n);
	}

	Ok(EntriesReport {
		entries: list,
		value_entries,
		counter_entries,
		total_bytes,
		values_verified: values_verified.get(),
		values_corrupted: values_corrupted.get(),
		unexpected_key_rows,
		column_scanned,
		block_time_ms,
		corrupted,
		orphans,
		matched,
		block_found,
		options: opts.clone(),
		elapsed: started.elapsed(),
	})
}

impl fmt::Display for EntriesReport {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(f, "Transaction storage entries (col11)")?;
		writeln!(f, "===================================")?;
		writeln!(f, "Scan duration:         {:?}", self.elapsed)?;
		if self.column_scanned {
			writeln!(
				f,
				"Value entries:         {}  ({} total)",
				self.value_entries,
				human_bytes(self.total_bytes),
			)?;
			writeln!(f, "Counter entries:       {}", self.counter_entries)?;
			writeln!(
				f,
				"Unexpected key rows:   {}{}",
				self.unexpected_key_rows,
				if self.unexpected_key_rows > 0 {
					"  ← unrecognised key shape in col11"
				} else {
					""
				},
			)?;
		} else {
			writeln!(f, "Column totals:         not scanned (targeted lookup)")?;
		}
		if let Some(number) = self.options.block_filter {
			writeln!(
				f,
				"Block filter:          #{number}{}",
				match (self.block_found, self.block_time_ms) {
					(Some(false), _) => "  (not in this database, or no indexed body)".to_string(),
					(_, Some(ms)) => format!("  authored {}", format_timestamp_ms(ms)),
					_ => String::new(),
				},
			)?;
		}
		if self.options.from_block.is_some() || self.options.to_block.is_some() {
			let render = |n: Option<u32>, fallback: &str| match n {
				Some(n) => format!("#{n}"),
				None => fallback.to_string(),
			};
			writeln!(
				f,
				"Stored between:        {} and {}",
				render(self.options.from_block, "genesis"),
				render(self.options.to_block, "chain head"),
			)?;
		}
		if let Some(hash) = self.options.hash_filter {
			let hex = hex(hash.as_ref());
			writeln!(f, "Hash filter:           {hex}")?;
		}
		if self.column_scanned {
			writeln!(
				f,
				"Integrity:             {} verified, {} corrupted{}",
				self.values_verified,
				self.values_corrupted,
				if self.values_corrupted == 0 { "  (every value hashes to its key)" } else { "" },
			)?;
		}
		writeln!(f, "Matched filters:       {}", self.matched)?;
		if !self.column_scanned {
			writeln!(
				f,
				"  integrity verified:  {} of {}",
				self.matched - self.corrupted,
				self.matched,
			)?;
		}
		writeln!(
			f,
			"  corrupted:           {}{}",
			self.corrupted,
			if self.corrupted > 0 { "  ← value does NOT hash to its key" } else { "" },
		)?;
		writeln!(
			f,
			"  orphans (0 refs):    {}{}",
			self.orphans,
			if self.options.resolve_blocks { "" } else { "  (block resolution off)" },
		)?;
		writeln!(
			f,
			"Showing:               {} of {}, sorted by {} {}",
			self.entries.len(),
			self.matched,
			self.options.sort.name(),
			if self.options.descending { "desc" } else { "asc" },
		)?;

		if self.entries.is_empty() {
			writeln!(f)?;
			writeln!(f, "No entries matched.")?;
			return Ok(());
		}

		let block_at = |number: Option<u32>, time: Option<u64>| match (number, time) {
			(Some(n), Some(ms)) => format!("#{n} ({})", format_timestamp_ms(ms)),
			(Some(n), None) => format!("#{n} (time unknown)"),
			(None, _) => "<none>".to_string(),
		};

		writeln!(f)?;
		for e in &self.entries {
			writeln!(f, "  {}", hex(e.content_hash.as_ref()))?;
			writeln!(
				f,
				"    size      {} ({})    refcount {}    referrers {}",
				e.size,
				human_bytes(e.size as u64),
				e.counter.map(|c| c.to_string()).unwrap_or_else(|| "<absent>".into()),
				e.referring_blocks,
			)?;
			match e.algo {
				Some(algo) =>
					writeln!(f, "    integrity OK — {}(value) == content hash", algo.name(),)?,
				None => {
					writeln!(
						f,
						"    integrity CORRUPTED — the stored bytes hash to none of the known \
						 algorithms:",
					)?;
					for (algo, got) in &e.computed_hashes {
						writeln!(f, "      {}(value) = {}", algo.name(), hex(got.as_ref()))?;
					}
					if self.options.resolve_blocks {
						writeln!(
							f,
							"      referring blocks ({}): {}",
							e.referring_block_list.len(),
							if e.referring_block_list.is_empty() {
								String::from("none — no alive block references this hash")
							} else {
								joined_blocks(&e.referring_block_list, 30)
							},
						)?;
					}
				},
			}
			if self.options.resolve_blocks {
				writeln!(f, "    created   {}", block_at(e.first_block, e.first_block_time_ms),)?;
				if e.last_block != e.first_block {
					writeln!(f, "    last seen {}", block_at(e.last_block, e.last_block_time_ms),)?;
				}
			}
			if !e.preview.is_empty() {
				write!(f, "{}", hexdump(&e.preview, "    "))?;
			}
		}

		if self.matched > self.entries.len() {
			writeln!(f)?;
			writeln!(
				f,
				"… {} more matched but were cut by --limit.",
				self.matched - self.entries.len(),
			)?;
		}
		if self.options.resolve_blocks {
			writeln!(f)?;
			writeln!(
				f,
				"Times are recovered from each block's timestamp inherent (heuristic decode).",
			)?;
		}
		Ok(())
	}
}
