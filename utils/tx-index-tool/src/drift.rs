// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Refcount drift: which counters the pre-aggregation commit path under-counted, and
//! which of those would lose data when the referencing blocks prune.

use crate::common::*;
use codec::Decode;
use kvdb::KeyValueDB;
use std::{
	collections::HashMap,
	fmt,
	time::{Duration, Instant},
};

/// A single (block, content-hash) pair where the body lists ≥2 references in one tx.
#[derive(Debug, Clone)]
pub struct DuplicatePattern {
	/// The content hash this pattern targets.
	pub content_hash: DbHash,
	/// Total occurrences across the block's body (sum across Indexed + MultiRenew inner counts).
	pub total_occurrences: u32,
	/// True if this hash is also referenced by ≥1 other alive block in BODY_INDEX.
	/// Only at-risk patterns can cause data loss when migrating — others saturate.
	pub at_risk: bool,
	/// True if the on-disk col11 counter for this hash actually equals the expected
	/// per-occurrence sum across all alive blocks. False means the old commit path's
	/// collapse-bug did under-count this entry on disk and a migration would be needed.
	pub on_disk_correct: bool,
}

/// A (block number, per-hash occurrence shape) pair stored in `all_referrers`. The shape
/// lets the at-risk summary annotate each block with whether its references are `Indexed`
/// or `MultiRenew`. Note: an `Indexed` extrinsic carries either an initial `store` or a
/// single `renew` — they're byte-indistinguishable in BODY_INDEX without runtime decoding.
#[derive(Debug, Clone)]
pub struct BlockOccurrence {
	/// Block number.
	pub number: u32,
	/// `Indexed` extrinsics in this block referencing the hash.
	pub indexed: u32,
	/// One entry per `MultiRenew` in this block, holding its inner occurrence count.
	pub multirenew_inner: Vec<u32>,
}

/// One block whose body triggered the bug for ≥1 content hash.
#[derive(Debug, Clone)]
pub struct AffectedBlock {
	/// Block number recovered from the `BODY_INDEX` lookup key (`None` if the key shape
	/// doesn't match the standard `[number_be, hash]` layout).
	pub number: Option<u32>,
	/// Per-content-hash patterns observed in this block (usually 1 entry).
	pub patterns: Vec<DuplicatePattern>,
}

/// Aggregated result of a dry-run scan.
#[derive(Debug, Default)]
pub struct DryRunReport {
	/// `BODY_INDEX` entries visited.
	pub blocks_scanned: u64,
	/// Entries that failed SCALE-decode.
	pub decode_failures: u64,
	/// Blocks whose body contained at least one hash appearing ≥2 times — *informational*,
	/// from body-shape analysis only. Doesn't mean the on-disk counter is necessarily wrong.
	pub blocks_with_duplicates: u64,
	/// Body-shape analysis only: hashes that *would* be under-counted if pre-aggregation code
	/// had written every block. Compare against `on_disk_drift` to see whether the bug
	/// actually fired in this DB's lifetime.
	pub body_pattern_undercount: HashMap<DbHash, u32>,
	/// **The migration-relevant measurement.** `hash → expected − actual` from the on-disk
	/// `TRANSACTION` counter. Only contains entries where drift > 0 — i.e. the bug fired
	/// for this entry and a migration would need to backfill exactly this many units.
	/// On a healthy fresh-sync this map is empty even when `body_pattern_undercount` is not.
	pub on_disk_drift: HashMap<DbHash, u32>,
	/// The counter actually on disk for each drifted hash. With `on_disk_drift` this gives the
	/// value a backfill would write, so the analysis can show `counter 10 → 4500` rather than
	/// only the shortfall.
	pub on_disk_actual: HashMap<DbHash, u32>,
	/// On-disk counter exceeds the expected per-occurrence sum. Should be empty in any
	/// well-behaved DB; non-empty would indicate a different bug (over-counting / stuck
	/// counters). Flagged separately so it's not silently lumped with under-counts.
	pub on_disk_excess: HashMap<DbHash, u32>,
	/// Subset of `on_disk_drift` for hashes also referenced by ≥1 other alive block — the
	/// hashes whose actual on-disk under-count will cause data loss on pruning.
	pub at_risk_drift: HashMap<DbHash, u32>,
	/// `hash → every alive block referencing it, with the per-block Indexed/MultiRenew
	/// breakdown`. Used to annotate the at-risk summary with shape details.
	pub all_referrers: HashMap<DbHash, Vec<BlockOccurrence>>,
	/// `max occurrences in one block → number of blocks at that level` (informational).
	pub duplicate_histogram: HashMap<u32, u64>,
	/// Per-block details for every block whose body triggered the duplicate-pattern detector.
	pub affected_blocks: Vec<AffectedBlock>,
	/// Wall-clock duration of the scan.
	pub elapsed: Duration,
}

impl DryRunReport {
	/// Number of distinct hashes whose body-shape would have under-counted under old code.
	pub fn body_pattern_hashes(&self) -> usize {
		self.body_pattern_undercount.len()
	}

	/// Number of distinct hashes whose on-disk counter actually drifted (bug fired).
	pub fn on_disk_drifted_hashes(&self) -> usize {
		self.on_disk_drift.len()
	}

	/// Subset of drifted hashes that are also referenced by ≥1 other alive block — these
	/// would cause actual data loss / chain halt on prune under the new code.
	pub fn at_risk_hashes(&self) -> usize {
		self.at_risk_drift.len()
	}

	/// Total counter units a migration would need to add.
	pub fn total_units_to_backfill(&self) -> u64 {
		self.on_disk_drift.values().map(|&n| n as u64).sum()
	}

	/// Sum of per-hash drift restricted to at-risk hashes.
	pub fn at_risk_units(&self) -> u64 {
		self.at_risk_drift.values().map(|&n| n as u64).sum()
	}

	/// Top `n` drifted hashes sorted by drift descending (hash ascending on ties).
	pub fn top_n_drifted(&self, n: usize) -> Vec<(DbHash, u32)> {
		let mut v: Vec<_> = self.on_disk_drift.iter().map(|(h, n)| (*h, *n)).collect();
		v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
		v.truncate(n);
		v
	}

	/// True if no at-risk drift was found and no decode failures — safe to upgrade in-place.
	pub fn is_clean(&self) -> bool {
		self.at_risk_drift.is_empty() && self.decode_failures == 0
	}
}

/// Scan the BODY_INDEX column, predict per-hash under-counts from body shapes, then
/// VERIFY against the actual on-disk `TRANSACTION` counters to report real drift.
/// Read-only; safe to run against a stopped node's database.
pub fn dry_run(db: &dyn KeyValueDB) -> std::io::Result<DryRunReport> {
	let started = Instant::now();
	let mut report = DryRunReport::default();

	// hash → distinct alive blocks referencing it. At-risk = >1 referrer.
	let mut referrer_count: HashMap<DbHash, u32> = HashMap::new();
	// hash → expected on-disk counter = total occurrences across all alive blocks.
	let mut expected_refcount: HashMap<DbHash, u32> = HashMap::new();

	for entry in db.iter(columns::BODY_INDEX) {
		let (k, v) = entry?;
		report.blocks_scanned += 1;

		// BODY_INDEX is keyed by `number_and_hash_to_lookup_key`.
		let number = split_lookup_key(&k).map(|(number, _)| number);

		let index = match Vec::<BareDbExtrinsic>::decode(&mut &v[..]) {
			Ok(idx) => idx,
			Err(_) => {
				report.decode_failures += 1;
				continue;
			},
		};

		let mut per_block: HashMap<DbHash, Occurrences> = HashMap::new();
		for ex in index {
			match ex {
				BareDbExtrinsic::Indexed { hash, .. } => {
					per_block.entry(hash).or_default().indexed += 1;
				},
				BareDbExtrinsic::MultiRenew { hashes, .. } => {
					let mut inner: HashMap<DbHash, u32> = HashMap::new();
					for h in hashes {
						*inner.entry(h).or_default() += 1;
					}
					for (h, n) in inner {
						per_block.entry(h).or_default().multirenew_inner.push(n);
					}
				},
				BareDbExtrinsic::Full(_) => {},
			}
		}

		// One referrer per distinct hash; expected refcount += occurrences.
		for (h, occ) in &per_block {
			*referrer_count.entry(*h).or_default() += 1;
			let total = occ.total();
			*expected_refcount.entry(*h).or_default() += total;
			if let Some(n) = number {
				report.all_referrers.entry(*h).or_default().push(BlockOccurrence {
					number: n,
					indexed: occ.indexed,
					multirenew_inner: occ.multirenew_inner.clone(),
				});
			}
		}

		let mut block_max = 0u32;
		let mut block_patterns = Vec::new();
		for (h, occ) in &per_block {
			let total = occ.total();
			if total > 1 {
				block_max = block_max.max(total);
				*report.body_pattern_undercount.entry(*h).or_default() += total - 1;
				block_patterns.push(DuplicatePattern {
					content_hash: *h,
					total_occurrences: total,
					at_risk: false,         // filled in second pass
					on_disk_correct: false, // filled in second pass
				});
			}
		}
		if block_max >= 2 {
			report.blocks_with_duplicates += 1;
			*report.duplicate_histogram.entry(block_max).or_default() += 1;
			block_patterns.sort_by(|a, b| b.total_occurrences.cmp(&a.total_occurrences));
			report.affected_blocks.push(AffectedBlock { number, patterns: block_patterns });
		}
	}

	// Second pass: read actual on-disk counter for each body-pattern hash and compute drift.
	// Only hashes that could have been affected are checked — those flagged by the body
	// analysis. A hash with no intra-tx duplicates anywhere in alive blocks can't have been
	// hit by the bug, so its counter is correct by construction.
	let body_hashes: Vec<DbHash> = report.body_pattern_undercount.keys().copied().collect();
	for h in body_hashes {
		let actual = read_counter(db, &h)?.unwrap_or(0);
		let expected = expected_refcount.get(&h).copied().unwrap_or(0);
		match actual.cmp(&expected) {
			std::cmp::Ordering::Less => {
				report.on_disk_drift.insert(h, expected - actual);
				report.on_disk_actual.insert(h, actual);
			},
			std::cmp::Ordering::Greater => {
				report.on_disk_excess.insert(h, actual - expected);
			},
			std::cmp::Ordering::Equal => {},
		}
	}

	// Mark at-risk and on_disk_correct per pattern; populate at_risk_drift.
	for block in &mut report.affected_blocks {
		for pattern in &mut block.patterns {
			let cnt = referrer_count.get(&pattern.content_hash).copied().unwrap_or(0);
			pattern.at_risk = cnt > 1;
			pattern.on_disk_correct = !report.on_disk_drift.contains_key(&pattern.content_hash);
		}
	}
	for (h, delta) in &report.on_disk_drift {
		if referrer_count.get(h).copied().unwrap_or(0) > 1 {
			report.at_risk_drift.insert(*h, *delta);
		}
	}

	report.affected_blocks.sort_by_key(|b| b.number.unwrap_or(u32::MAX));
	report.elapsed = started.elapsed();
	Ok(report)
}

impl fmt::Display for DryRunReport {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(f, "Refcount migration dry-run")?;
		writeln!(f, "==========================")?;
		writeln!(f, "Scan duration:                 {:?}", self.elapsed)?;
		writeln!(f, "BODY_INDEX entries scanned:    {}", self.blocks_scanned)?;
		writeln!(f, "Decode failures:               {}", self.decode_failures)?;
		writeln!(f, "Blocks with intra-tx dups in body:    {}", self.blocks_with_duplicates)?;
		writeln!(f, "  └ body-pattern hashes (hypothetical): {}    (would under-count if buggy code wrote them)", self.body_pattern_hashes())?;
		writeln!(f)?;
		writeln!(f, "ACTUAL on-disk verification:")?;
		writeln!(
			f,
			"  Hashes with under-counted counter:    {}    ← THE migration-relevant number",
			self.on_disk_drifted_hashes()
		)?;
		writeln!(
			f,
			"    └ at-risk (other referrers):        {}    ← would cause data loss on prune",
			self.at_risk_hashes()
		)?;
		writeln!(
			f,
			"    └ harmless (saturates):             {}",
			self.on_disk_drifted_hashes().saturating_sub(self.at_risk_hashes())
		)?;
		writeln!(f, "  Hashes with EXCESS counter (anomaly): {}", self.on_disk_excess.len())?;
		writeln!(f, "  Total counter units to backfill:      {}", self.total_units_to_backfill())?;
		writeln!(f, "    └ at-risk subset:                   {}", self.at_risk_units())?;

		if !self.duplicate_histogram.is_empty() {
			writeln!(f)?;
			writeln!(f, "Max-occurrence-per-block histogram:")?;
			let mut hist: Vec<_> = self.duplicate_histogram.iter().collect();
			hist.sort_by_key(|(k, _)| *k);
			for (count, n_blocks) in hist {
				writeln!(f, "  hash seen {} times in body: {} blocks", count, n_blocks)?;
			}
		}

		if !self.on_disk_drift.is_empty() {
			writeln!(f)?;
			writeln!(f, "Top 10 on-disk-drifted counters (current → correct):")?;
			for (hash, delta) in self.top_n_drifted(10) {
				let actual = self.on_disk_actual.get(&hash).copied().unwrap_or(0);
				writeln!(
					f,
					"  {}  {} → {}  (+{delta})",
					hex(hash.as_ref()),
					actual,
					actual.saturating_add(delta),
				)?;
			}
		}

		if !self.at_risk_drift.is_empty() {
			writeln!(f)?;
			writeln!(
				f,
				"At-risk summary ({} hash(es) would cause data loss on prune):",
				self.at_risk_hashes(),
			)?;
			writeln!(
				f,
				"  When the first over-releaser prunes it decrements by its whole occurrence"
			)?;
			writeln!(
				f,
				"  count, so an under-counted entry can reach zero while every remaining block"
			)?;
			writeln!(f, "  — over-releasers included — still references it.")?;
			writeln!(f, "  Each line lists the block's reference shape for the target hash:")?;
			writeln!(f, "    `Indexed` = a single store or a single renew (indistinguishable in BODY_INDEX)")?;
			writeln!(
				f,
				"    `MultiRenew(×N)` = one batch-renew extrinsic referencing the hash N times"
			)?;

			// Build hash → block_number → BlockOccurrence index for fast lookup.
			// Only at-risk hashes are ever looked up, so don't index the whole chain.
			let mut occ_index: HashMap<DbHash, HashMap<u32, &BlockOccurrence>> = HashMap::new();
			for hash in self.at_risk_drift.keys() {
				if let Some(list) = self.all_referrers.get(hash) {
					let entry = occ_index.entry(*hash).or_default();
					for bo in list {
						entry.insert(bo.number, bo);
					}
				}
			}

			// Buggy blocks per hash: from `affected_blocks` (only over-releasers).
			let mut buggy_blocks: HashMap<DbHash, Vec<u32>> = HashMap::new();
			for block in &self.affected_blocks {
				let Some(num) = block.number else { continue };
				for p in &block.patterns {
					if p.at_risk && !p.on_disk_correct {
						buggy_blocks.entry(p.content_hash).or_default().push(num);
					}
				}
			}

			let mut sorted: Vec<_> = self.at_risk_drift.iter().collect();
			sorted.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));

			const MAX_LIST: usize = 20;
			for (hash, drift) in sorted {
				let prefix = format!("{}..", hex(&hash.as_ref()[..8]));

				let mut buggy = buggy_blocks.get(hash).cloned().unwrap_or_default();
				buggy.sort();
				let buggy_set: std::collections::HashSet<u32> = buggy.iter().copied().collect();

				let mut others: Vec<u32> = self
					.all_referrers
					.get(hash)
					.map(|v| {
						v.iter().map(|bo| bo.number).filter(|n| !buggy_set.contains(n)).collect()
					})
					.unwrap_or_default();
				others.sort();

				let occs = occ_index.get(hash);
				let fmt_block = |n: u32| -> String {
					match occs.and_then(|m| m.get(&n)) {
						Some(bo) => {
							let occ = Occurrences {
								indexed: bo.indexed,
								multirenew_inner: bo.multirenew_inner.clone(),
							};
							format!("#{n}[{}]", fmt_occurrences(&occ))
						},
						None => format!("#{n}"),
					}
				};

				writeln!(f, "  +{drift}  {prefix}")?;
				writeln!(
					f,
					"         over-releasers ({}): {}",
					buggy.len(),
					joined(&buggy, MAX_LIST, |n| fmt_block(*n)),
				)?;
				writeln!(
					f,
					"         other referrers ({}): {}",
					others.len(),
					joined(&others, MAX_LIST, |n| fmt_block(*n)),
				)?;
			}
		}

		writeln!(f)?;
		if self.is_clean() {
			if self.on_disk_drifted_hashes() == 0 {
				if self.body_pattern_hashes() == 0 {
					writeln!(f, "Result: CLEAN — no duplicate patterns AND no on-disk drift.")?;
				} else {
					writeln!(
						f,
						"Result: CLEAN ON DISK — {} body patterns exist that COULD have under-counted \
						 under old code, but the on-disk counters all match the expected per-occurrence \
						 totals. The commit-path fix is correctly handling these patterns.",
						self.body_pattern_hashes(),
					)?;
				}
			} else {
				writeln!(
					f,
					"Result: SAFE TO UPGRADE — {} hash(es) drifted on disk but all are sole \
					 referrers; over-release will saturate. Migration optional / cosmetic.",
					self.on_disk_drifted_hashes(),
				)?;
			}
		} else {
			writeln!(
				f,
				"Result: MIGRATION RECOMMENDED — {} hash(es) are at-risk: under-counted on disk \
				 AND referenced by ≥1 other alive block. Upgrading without backfill will cause \
				 those col11 entries to be deleted prematurely when the affected blocks prune.",
				self.at_risk_hashes(),
			)?;
		}
		Ok(())
	}
}
