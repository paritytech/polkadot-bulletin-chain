// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Recovery of corrupted col11 values by re-splitting `BODY_INDEX.header ++ col11` at a
//! different offset, which is what the PR 574 bug needs.

use crate::common::*;
use codec::Decode;
use kvdb::KeyValueDB;
use std::{
	collections::HashMap,
	fmt,
	time::{Duration, Instant},
};

/// Outcome of a realignment attempt for a single corrupted col11 entry.
#[derive(Debug, Clone, Default)]
pub struct RealignOutcome {
	/// The corrupted col11 slot key being realigned.
	pub content_hash: DbHash,
	/// Block number whose BODY_INDEX entry was used to reconstruct the full encoded extrinsic.
	pub source_block: Option<u32>,
	/// All block numbers whose BODY_INDEX references this hash (`Indexed` or `MultiRenew`),
	/// sorted ascending, deduplicated.
	pub referencing_blocks: Vec<u32>,
	/// Size of the `header` field in BODY_INDEX for this hash in that block.
	pub header_size: usize,
	/// Size of the current (corrupted) col11 value.
	pub current_col11_size: usize,
	/// Total reconstructable bytes: `header.len() + col11_value.len()`.
	pub reconstructed_total: usize,
	/// Net length delta (positive = corrected data is longer than current col11). None if no
	/// alignment in the scanned range matched.
	pub matched_shift: Option<i64>,
	/// How many bytes the start of the corrected slice moved relative to the current split
	/// point. Negative = data starts earlier (bytes pulled from header tail). Positive =
	/// data starts later (some "data" bytes were actually header bytes that should be dropped).
	pub matched_start_shift: Option<i64>,
	/// How many bytes were chopped from the END of the current col11 value to find the
	/// correct slice. 0 = end unchanged. Positive = N bytes at the tail of col11 don't
	/// belong to the data (e.g., signer / signature / timestamp appended by the runtime).
	pub matched_end_chop: Option<i64>,
	/// Algorithm under which the realigned data matched.
	pub matched_algo: Option<HashAlgo>,
	/// Size of the corrected data slice.
	pub corrected_size: Option<usize>,
	/// True iff the corrected slice was written back to col11.
	pub applied: bool,
}

/// Where a candidate slice of `header ++ col11` was found, and under which algorithm.
pub struct Alignment {
	/// Offset into the reconstructed bytes where the corrected data starts.
	pub start: usize,
	/// Offset one past its last byte.
	pub end: usize,
	/// Algorithm under which the slice hashes to the content hash.
	pub algo: HashAlgo,
}

/// Find the slice of `full` (= `BODY_INDEX.header ++ col11 value`) that hashes to `want`.
///
/// The strategies run cheapest-first and the first match wins:
///   1. Known PR 574 sizes — a trailing `(MultiSigner, MultiSignature, u64)` tuple encodes to 106
///      bytes (Sr25519/Ed25519 both), 107 (mixed) or 108 (Ecdsa both) — tried both as a
///      length-preserving diagonal shift, and as a chop from the end.
///   2. Generic chop-from-end, for any trailing-bytes bug.
///   3. Generic start-shift, for a moved split point.
///   4. Generic length-preserving diagonal shift.
pub fn find_alignment(
	full: &[u8],
	header_size: usize,
	col11_size: usize,
	want: DbHash,
	max_shift: u32,
) -> Option<Alignment> {
	let matches = |start: usize, end: usize| -> Option<Alignment> {
		HashAlgo::identify(want, &full[start..end]).map(|algo| Alignment { start, end, algo })
	};
	let max_shift = max_shift as i64;

	// (1) The diagonal variant preserves length, so it applies even when the appended tuple is
	// larger than the payload itself; the chop-only variant needs the payload to be longer.
	for n in [106usize, 107, 108] {
		if n <= header_size && full.len() > n {
			let (start, end) = (header_size - n, full.len() - n);
			if end > start {
				if let Some(found) = matches(start, end) {
					return Some(found);
				}
			}
		}
		if n < col11_size {
			if let Some(found) = matches(header_size, full.len() - n) {
				return Some(found);
			}
		}
	}

	// (2) Chop k bytes off the end, start unchanged.
	for chop in 1..=max_shift {
		let chop = chop as usize;
		if chop >= col11_size {
			break;
		}
		if let Some(found) = matches(header_size, full.len() - chop) {
			return Some(found);
		}
	}

	// (3) Move the start within ±max_shift, end fixed.
	let start_lo = (header_size as i64 - max_shift).max(0) as usize;
	let start_hi = (header_size as i64 + max_shift).min(full.len() as i64) as usize;
	for start in start_lo..=start_hi {
		if start == header_size {
			continue;
		}
		if start >= full.len() {
			break;
		}
		if let Some(found) = matches(start, full.len()) {
			return Some(found);
		}
	}

	// (4) Shift the whole window back, preserving length.
	for delta in 1..=max_shift {
		let delta = delta as usize;
		if delta > header_size || delta >= col11_size {
			break;
		}
		if let Some(found) = matches(header_size - delta, full.len() - delta) {
			return Some(found);
		}
	}

	None
}

/// Record an alignment on the outcome, writing the corrected slice back when `apply` is set.
fn record_alignment(
	db: &dyn KeyValueDB,
	outcome: &mut RealignOutcome,
	full: &[u8],
	found: &Alignment,
	apply: bool,
) -> std::io::Result<()> {
	let candidate = &full[found.start..found.end];
	outcome.matched_shift = Some(candidate.len() as i64 - outcome.current_col11_size as i64);
	outcome.matched_start_shift = Some(found.start as i64 - outcome.header_size as i64);
	outcome.matched_end_chop = Some(full.len() as i64 - found.end as i64);
	outcome.matched_algo = Some(found.algo);
	outcome.corrected_size = Some(candidate.len());
	if apply {
		let mut tx = db.transaction();
		tx.put(columns::TRANSACTION, outcome.content_hash.as_ref(), candidate);
		db.write(tx)?;
		outcome.applied = true;
	}
	Ok(())
}
/// Attempt to recover correct content for a corrupted col11 entry by re-splitting the
/// `header ++ col11_value` byte string at different offsets and finding the split where the
/// data side hashes to the slot key under one of the supported algorithms.
///
/// This addresses the case where the runtime/host passed a wrong `size` to
/// `sp_io::transaction_index::index`, so the boundary between BODY_INDEX.header and
/// TRANSACTION[hash] is shifted by a few bytes. The original encoded extrinsic is intact in
/// `header ++ col11_value`; only the cut position is wrong.
///
/// `max_shift` bounds the search (e.g. 16 is enough to catch SCALE compact-prefix mistakes,
/// which are typically 1, 2, or 4 bytes). Larger search ranges cost proportionally more
/// hashing work per entry.
pub fn realign_from_body_index(
	db: &dyn KeyValueDB,
	content_hash: DbHash,
	max_shift: u32,
	apply: bool,
) -> std::io::Result<RealignOutcome> {
	let mut outcome = RealignOutcome { content_hash, ..Default::default() };

	// Find the first (lowest-numbered) alive block whose BODY_INDEX references this hash as
	// an Indexed entry. Use that block's `header` bytes for reconstruction.
	let mut found_header: Option<(u32, Vec<u8>)> = None;
	for entry in db.iter(columns::BODY_INDEX) {
		let (k, v) = entry?;
		let Some((block_number, _)) = split_lookup_key(&k) else { continue };
		let Ok(index) = Vec::<BareDbExtrinsic>::decode(&mut &v[..]) else { continue };
		let mut references = false;
		for ex in index {
			match ex {
				BareDbExtrinsic::Indexed { hash, header } if hash == content_hash => {
					references = true;
					match &found_header {
						None => found_header = Some((block_number, header)),
						Some((bn, _)) if block_number < *bn =>
							found_header = Some((block_number, header)),
						_ => {},
					}
				},
				// MultiRenew doesn't carry a header field per hash — renewals reuse the
				// storer's data, which is what's already at TRANSACTION[hash] — but the
				// block still references the corrupted entry, so record it.
				BareDbExtrinsic::MultiRenew { hashes, .. } if hashes.contains(&content_hash) =>
					references = true,
				_ => {},
			}
		}
		if references {
			outcome.referencing_blocks.push(block_number);
		}
	}
	outcome.referencing_blocks.sort_unstable();
	outcome.referencing_blocks.dedup();
	let Some((block_number, header)) = found_header else { return Ok(outcome) };
	outcome.source_block = Some(block_number);
	outcome.header_size = header.len();

	let col11_value = db.get(columns::TRANSACTION, content_hash.as_ref())?.unwrap_or_default();
	outcome.current_col11_size = col11_value.len();

	let mut full = header;
	full.extend_from_slice(&col11_value);
	outcome.reconstructed_total = full.len();

	if let Some(found) = find_alignment(
		&full,
		outcome.header_size,
		outcome.current_col11_size,
		content_hash,
		max_shift,
	) {
		record_alignment(db, &mut outcome, &full, &found, apply)?;
	}

	Ok(outcome)
}

impl fmt::Display for RealignOutcome {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(f, "Realignment for {}", hex(self.content_hash.as_ref()))?;
		match self.source_block {
			Some(n) => writeln!(f, "  source block:               #{n}")?,
			None => {
				writeln!(f, "  source block:               <none — no BODY_INDEX references this Indexed hash>")?;
				return Ok(());
			},
		}
		if !self.referencing_blocks.is_empty() {
			let blocks = joined_blocks(&self.referencing_blocks, usize::MAX);
			writeln!(f, "  referencing blocks:         {blocks}")?;
		}
		writeln!(f, "  header.len() in BODY_INDEX: {}", self.header_size)?;
		writeln!(f, "  current col11 value size:   {}", self.current_col11_size)?;
		writeln!(f, "  full reconstructed bytes:   {}", self.reconstructed_total)?;
		match self.matched_shift {
			Some(_) => {
				let start_shift = self.matched_start_shift.unwrap_or(0);
				let end_chop = self.matched_end_chop.unwrap_or(0);
				writeln!(f, "  ✓ alignment matched under {}", self.matched_algo.unwrap().name())?;
				writeln!(f, "     start shift: {start_shift:+} bytes  (negative = bytes pulled from header tail)")?;
				writeln!(f, "     end chop:    {end_chop:+} bytes  (positive = bytes at tail of col11 don't belong)")?;
				writeln!(
					f,
					"     corrected data size: {} (current {})",
					self.corrected_size.unwrap(),
					self.current_col11_size
				)?;
				if end_chop > 0 && start_shift == 0 {
					writeln!(
						f,
						"     → PR 574 pattern: trailing (signer, sig, timestamp) tuple appended"
					)?;
				}
				writeln!(
					f,
					"  applied: {}",
					if self.applied { "yes (col11 overwritten)" } else { "no (dry-run)" }
				)?;
				if end_chop > 0 || start_shift != 0 {
					writeln!(f)?;
					writeln!(
						f,
						"  WARNING: this entry's extrinsic has bytes after the indexed data, so"
					)?;
					writeln!(
						f,
						"  integrity and executability cannot both hold. A body is reassembled as"
					)?;
					writeln!(
						f,
						"  exactly `BODY_INDEX.header ++ col11` (sc-client-db `body_uncached`), and"
					)?;
					writeln!(
						f,
						"  the original extrinsic was `header ++ data ++ trailing-fields`. Writing"
					)?;
					writeln!(
						f,
						"  the aligned data here makes col11 hash correctly — bitswap and storage"
					)?;
					writeln!(
						f,
						"  proofs work — but the reassembled body no longer matches what was"
					)?;
					writeln!(f, "  authored, so every block referencing it becomes permanently")?;
					writeln!(
						f,
						"  unexecutable: any node replaying it panics with \"The extrinsic could"
					)?;
					writeln!(
						f,
						"  not be decoded correctly\". It also destroys the trailing fields, which"
					)?;
					writeln!(
						f,
						"  live only in the value being overwritten and are not recoverable"
					)?;
					writeln!(f, "  afterwards.")?;
					writeln!(f)?;
					writeln!(
						f,
						"  Run `incident bulletin-574 verify` to classify entries before repairing."
					)?;
				}
			},
			None => writeln!(f, "  ✗ no shift in the scanned range produced a matching hash")?,
		}
		Ok(())
	}
}

/// Outcome of `realign_all_corrupted` — a batch realignment over every corrupted entry.
#[derive(Debug, Default)]
pub struct BatchRealignReport {
	/// One row per corrupted entry found, sorted by content hash.
	pub rows: Vec<RealignOutcome>,
	/// Wall-clock duration of the batch.
	pub elapsed: Duration,
}

impl BatchRealignReport {
	/// Corrupted entries no search strategy could recover — the ones needing manual repair.
	pub fn unrecovered(&self) -> usize {
		self.rows.iter().filter(|r| r.matched_shift.is_none()).count()
	}
}

/// Scan col11 for corrupted entries and try to realign each by re-splitting against its
/// BODY_INDEX header. Equivalent to running `scan_corruption` followed by
/// `realign_from_body_index` for every corrupted hash, but does the BODY_INDEX walk once.
pub fn realign_all_corrupted(
	db: &dyn KeyValueDB,
	max_shift: u32,
	apply: bool,
) -> std::io::Result<BatchRealignReport> {
	let started = Instant::now();
	let mut report = BatchRealignReport::default();

	// Pass 1: identify corrupted slots (same logic as scan_corruption but only collecting
	// the bad keys + their value sizes).
	let mut bad_keys: HashMap<DbHash, usize> = HashMap::new();
	for entry in db.iter(columns::TRANSACTION) {
		let (k, v) = entry?;
		if k.len() != 32 {
			continue;
		}
		let mut kb = [0u8; 32];
		kb.copy_from_slice(&k);
		let key_hash = DbHash::from(kb);
		if HashAlgo::identify(key_hash, &v).is_none() {
			bad_keys.insert(key_hash, v.len());
		}
	}

	// Pass 2: walk BODY_INDEX once, collecting per-bad-hash headers (use the first / lowest-
	// numbered block to be deterministic).
	let mut headers: HashMap<DbHash, (u32, Vec<u8>)> = HashMap::new();
	let mut refs: HashMap<DbHash, Vec<u32>> = HashMap::new();
	for entry in db.iter(columns::BODY_INDEX) {
		let (k, v) = entry?;
		let Some((block_number, _)) = split_lookup_key(&k) else { continue };
		let Ok(index) = Vec::<BareDbExtrinsic>::decode(&mut &v[..]) else { continue };
		for ex in index {
			match ex {
				BareDbExtrinsic::Indexed { hash, header } if bad_keys.contains_key(&hash) => {
					refs.entry(hash).or_default().push(block_number);
					let pick = match headers.get(&hash) {
						Some((bn, _)) => block_number < *bn,
						None => true,
					};
					if pick {
						headers.insert(hash, (block_number, header));
					}
				},
				BareDbExtrinsic::MultiRenew { hashes, .. } =>
					for hash in hashes {
						if bad_keys.contains_key(&hash) {
							refs.entry(hash).or_default().push(block_number);
						}
					},
				_ => {},
			}
		}
	}
	for blocks in refs.values_mut() {
		blocks.sort_unstable();
		blocks.dedup();
	}

	// Pass 3: try realignment for each corrupted hash.
	let mut sorted_keys: Vec<DbHash> = bad_keys.keys().copied().collect();
	sorted_keys.sort();
	for hash in sorted_keys {
		let current_col11_size = *bad_keys.get(&hash).unwrap();
		let mut outcome = RealignOutcome {
			content_hash: hash,
			referencing_blocks: refs.remove(&hash).unwrap_or_default(),
			current_col11_size,
			..Default::default()
		};

		let Some((block_number, header)) = headers.get(&hash).cloned() else {
			report.rows.push(outcome);
			continue;
		};
		outcome.source_block = Some(block_number);
		outcome.header_size = header.len();

		let col11_value = db.get(columns::TRANSACTION, hash.as_ref())?.unwrap_or_default();
		let mut full = header;
		full.extend_from_slice(&col11_value);
		outcome.reconstructed_total = full.len();

		if let Some(found) =
			find_alignment(&full, outcome.header_size, current_col11_size, hash, max_shift)
		{
			record_alignment(db, &mut outcome, &full, &found, apply)?;
		}

		report.rows.push(outcome);
	}

	report.elapsed = started.elapsed();
	Ok(report)
}

impl fmt::Display for BatchRealignReport {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(f, "Batch realignment from BODY_INDEX header")?;
		writeln!(f, "========================================")?;
		writeln!(f, "Elapsed:               {:?}", self.elapsed)?;
		writeln!(f, "Corrupted entries:     {}", self.rows.len())?;
		let recoverable = self.rows.len() - self.unrecovered();
		let applied = self.rows.iter().filter(|r| r.applied).count();
		let no_body_index = self.rows.iter().filter(|r| r.source_block.is_none()).count();
		writeln!(f, "  recoverable (match):  {recoverable}")?;
		writeln!(f, "  applied (written):    {applied}")?;
		writeln!(f, "  unrecoverable:        {}", self.rows.len() - recoverable - no_body_index)?;
		writeln!(f, "  no BODY_INDEX entry:  {no_body_index}")?;

		// Group recoverable rows by (shift, algo) — for a single systematic cause the shift is
		// be uniform across all entries, so this collapses to a one-line summary in the happy
		// case.
		let mut by_pattern: HashMap<(i64, &'static str), usize> = HashMap::new();
		for r in &self.rows {
			if let (Some(s), Some(a)) = (r.matched_shift, r.matched_algo) {
				*by_pattern.entry((s, a.name())).or_default() += 1;
			}
		}
		if !by_pattern.is_empty() {
			writeln!(f)?;
			writeln!(f, "Recovery pattern distribution (shift bytes, algo → count):")?;
			let mut by_pattern: Vec<_> = by_pattern.into_iter().collect();
			by_pattern.sort_by_key(|((s, _), _)| *s);
			for ((shift, algo), count) in by_pattern {
				writeln!(f, "  shift = {shift:+}, algo = {algo}: {count} entries")?;
			}
		}

		// List every corrupted entry with the blocks whose BODY_INDEX references it.
		if !self.rows.is_empty() {
			writeln!(f)?;
			writeln!(f, "Corrupted entries (hash → referencing blocks):")?;
			for r in &self.rows {
				let hex = hex(r.content_hash.as_ref());
				let blocks = if r.referencing_blocks.is_empty() {
					String::from("<none>")
				} else {
					joined_blocks(&r.referencing_blocks, usize::MAX)
				};
				let status = if r.applied {
					"fixed"
				} else if r.matched_shift.is_some() {
					"recoverable"
				} else if r.source_block.is_none() {
					"no BODY_INDEX entry"
				} else {
					"unrecoverable"
				};
				writeln!(f, "  {hex}  [{status}]")?;
				writeln!(f, "    blocks: {blocks}")?;
				match r.corrected_size {
					Some(c) if c as i64 != r.current_col11_size as i64 => writeln!(
						f,
						"    data size: {} bytes (corrected: {c} bytes)",
						r.current_col11_size
					)?,
					_ => writeln!(f, "    data size: {} bytes", r.current_col11_size)?,
				}
			}

			let mut affected: Vec<u32> =
				self.rows.iter().flat_map(|r| r.referencing_blocks.iter().copied()).collect();
			affected.sort_unstable();
			affected.dedup();
			if !affected.is_empty() {
				let list = joined_blocks(&affected, usize::MAX);
				writeln!(f)?;
				writeln!(f, "Affected blocks ({} distinct): {list}", affected.len())?;
			}
		}

		// List unrecoverable entries explicitly so the operator knows which need manual repair.
		let unrec: Vec<_> = self
			.rows
			.iter()
			.filter(|r| r.matched_shift.is_none() && r.source_block.is_some())
			.collect();
		if !unrec.is_empty() {
			writeln!(f)?;
			writeln!(f, "Unrecoverable entries (no shift in range produced a match):")?;
			for r in unrec {
				let hex = hex(r.content_hash.as_ref());
				writeln!(
					f,
					"  {hex}  (col11 size {}, header size {}, source #{})",
					r.current_col11_size,
					r.header_size,
					r.source_block.unwrap()
				)?;
			}
		}

		Ok(())
	}
}
