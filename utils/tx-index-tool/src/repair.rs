// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Replacing a corrupted col11 value with known-good bytes.

use crate::common::*;
use kvdb::KeyValueDB;
use std::{
	fmt,
	time::{Duration, Instant},
};

/// Outcome of a planned col11 value repair for a single content hash.
#[derive(Debug, Clone)]
pub struct RepairOutcome {
	/// The content hash whose `TRANSACTION[hash]` value is being overwritten.
	pub content_hash: DbHash,
	/// Hash algorithm used to compute `new_data_hash` and to verify the repair.
	pub algo: HashAlgo,
	/// `algo(new_data)` — must equal `content_hash` for write to proceed.
	pub new_data_hash: DbHash,
	/// Whether `new_data_hash` equals `content_hash` — the precondition for writing.
	pub hash_matches: bool,
	/// Size of the value currently on disk (None if absent).
	pub old_value_size: Option<usize>,
	/// `algo(on-disk value)`, useful for diff reporting.
	pub old_value_hash: Option<DbHash>,
	/// Hash of the on-disk value under EVERY supported algorithm — helps the operator
	/// determine which `HashingAlgorithm` the slot actually uses when their algo guess
	/// produces no match.
	pub old_value_hash_all_algos: Vec<(HashAlgo, DbHash)>,
	/// Size of the replacement data in bytes.
	pub new_value_size: usize,
	/// Current on-disk refcount counter at `TRANSACTION[hash‖0x00]`.
	pub current_counter: Option<u32>,
	/// True if the on-disk hash (under `algo`) already matches `content_hash`.
	pub already_correct: bool,
	/// True iff the write was actually applied.
	pub applied: bool,
}

/// Plan or apply a repair: overwrite `TRANSACTION[hash]` with `new_data` if `algo(new_data)
/// == hash`. The counter entry at `TRANSACTION[hash‖0x00]` is **not** touched — only the value
/// half of the (counter, value) pair is replaced.
///
/// `algo` is the hash algorithm that was used at store-time (`HashingAlgorithm::Blake2b256` for
/// plain `store(data)`, others come from `store_with_cid_config`). The on-disk value is
/// additionally hashed under all three known algorithms to help diagnose the right choice
/// when the supplied algo doesn't match.
pub fn repair_value(
	db: &dyn KeyValueDB,
	content_hash: DbHash,
	algo: HashAlgo,
	new_data: &[u8],
	apply: bool,
) -> std::io::Result<RepairOutcome> {
	let new_data_hash = DbHash::from(algo.hash(new_data));
	let hash_matches = new_data_hash == content_hash;

	let old_value = db.get(columns::TRANSACTION, content_hash.as_ref())?;
	let old_value_size = old_value.as_ref().map(|v| v.len());
	// Hash the on-disk value under every algorithm once: the operator needs the whole set to
	// spot "your --algo was wrong — the slot uses sha2_256", and `algo`'s own hash is in there.
	let old_value_hash_all_algos: Vec<(HashAlgo, DbHash)> =
		old_value.as_ref().map(|v| HashAlgo::hash_all(v)).unwrap_or_default();
	let old_value_hash = old_value_hash_all_algos
		.iter()
		.find(|(a, _)| a.name() == algo.name())
		.map(|(_, h)| *h);
	let already_correct = old_value_hash == Some(content_hash);

	let current_counter = read_counter(db, &content_hash)?;

	let mut applied = false;
	if apply {
		if !hash_matches {
			return Err(std::io::Error::other(format!(
				"refused to write: {}(new_data) does not match content_hash",
				algo.name(),
			)));
		}
		if !already_correct {
			let mut tx = db.transaction();
			tx.put(columns::TRANSACTION, content_hash.as_ref(), new_data);
			db.write(tx)?;
			applied = true;
		}
	}

	Ok(RepairOutcome {
		content_hash,
		algo,
		new_data_hash,
		hash_matches,
		old_value_size,
		old_value_hash,
		old_value_hash_all_algos,
		new_value_size: new_data.len(),
		current_counter,
		already_correct,
		applied,
	})
}

impl fmt::Display for RepairOutcome {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(
			f,
			"Repair plan for {}  (algo: {})",
			hex(self.content_hash.as_ref()),
			self.algo.name()
		)?;
		writeln!(
			f,
			"  current on-disk refcount counter: {}",
			self.current_counter.map(|c| c.to_string()).unwrap_or_else(|| "<absent>".into())
		)?;
		match (self.old_value_size, &self.old_value_hash) {
			(Some(sz), Some(h)) => {
				writeln!(f, "  current value: {sz} bytes")?;
				writeln!(f, "    {}(value) = {}", self.algo.name(), hex(h.as_ref()))?;
				// Diagnostic: print the value hashed under all algorithms. If one of them
				// matches `content_hash`, that's the right algo to use.
				let mut suggested: Option<HashAlgo> = None;
				for (a, ah) in &self.old_value_hash_all_algos {
					if a.name() == self.algo.name() {
						continue;
					}
					let m =
						if *ah == self.content_hash { "  ← MATCHES content_hash" } else { "" };
					writeln!(f, "    {}(value) = {}{}", a.name(), hex(ah.as_ref()), m)?;
					if *ah == self.content_hash {
						suggested = Some(*a);
					}
				}
				if self.already_correct {
					writeln!(
						f,
						"  → on-disk value ALREADY matches content_hash under {}; nothing to do",
						self.algo.name()
					)?;
				} else if let Some(a) = suggested {
					writeln!(
						f,
						"  → on-disk value MATCHES content_hash under {} (not under {});",
						a.name(),
						self.algo.name()
					)?;
					writeln!(f, "    → the slot is fine, you guessed the wrong --algo")?;
				} else {
					writeln!(
						f,
						"  → on-disk value HASH MISMATCH under all known algorithms (corruption)"
					)?;
				}
			},
			_ => writeln!(f, "  current value: <absent>")?,
		}
		writeln!(
			f,
			"  proposed value: {} bytes, {}(data) = {}",
			self.new_value_size,
			self.algo.name(),
			hex(self.new_data_hash.as_ref())
		)?;
		if self.hash_matches {
			writeln!(f, "  hash check: OK ({}(new_data) == content_hash)", self.algo.name())?;
		} else {
			writeln!(f, "  hash check: FAILED — write would be refused")?;
		}
		writeln!(
			f,
			"  applied: {}",
			if self.applied { "yes (wrote new value)" } else { "no (dry-run)" }
		)?;
		Ok(())
	}
}

/// One counter the backfill would set.
#[derive(Debug, Clone)]
pub struct RefcountRow {
	/// The col11 entry whose refcount is short.
	pub content_hash: DbHash,
	/// Counter currently on disk.
	pub counter_before: u32,
	/// Counter it should hold: the sum of references across all alive blocks.
	pub counter_after: u32,
	/// Whether more than one block references the entry, i.e. whether the shortfall can cause
	/// the value to be deleted while something still points at it.
	pub at_risk: bool,
	/// Whether the write happened.
	pub applied: bool,
}

/// Result of `repair_refcounts`.
#[derive(Debug, Default)]
pub struct RefcountBackfillReport {
	/// One row per drifted counter, largest shortfall first.
	pub rows: Vec<RefcountRow>,
	/// Whether the rows were written or only planned.
	pub applied: bool,
	/// Wall-clock duration, including the drift analysis it is based on.
	pub elapsed: Duration,
}

impl RefcountBackfillReport {
	/// Total counter units the backfill adds.
	pub fn units(&self) -> u64 {
		self.rows.iter().map(|r| u64::from(r.counter_after - r.counter_before)).sum()
	}

	/// Rows whose shortfall can cause data loss.
	pub fn at_risk(&self) -> usize {
		self.rows.iter().filter(|r| r.at_risk).count()
	}
}

/// Backfill the refcounts that `dry_run` found short, setting each to the number of references
/// its entry actually has across all alive blocks.
///
/// This is the write half of the `drift` analysis: the pre-aggregation commit path collapsed N
/// same-key operations in one block to a single ±1, so a counter can read "one per referencing
/// block" where it should read "one per reference". Left alone, the first block to prune
/// decrements by its full occurrence count, takes the counter to zero, and the value is deleted
/// while the remaining blocks still reference it.
///
/// Only the counter row (`TRANSACTION[hash‖0x00]`) is written; stored values are never touched.
pub fn repair_refcounts(
	db: &dyn KeyValueDB,
	apply: bool,
) -> std::io::Result<RefcountBackfillReport> {
	let started = Instant::now();
	let drift = crate::drift::dry_run(db)?;

	// Largest shortfall first, matching how `drift` ranks its own output.
	let mut drifted: Vec<(DbHash, u32)> =
		drift.on_disk_drift.iter().map(|(hash, delta)| (*hash, *delta)).collect();
	drifted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

	let mut report = RefcountBackfillReport { applied: apply, ..Default::default() };
	for (content_hash, delta) in drifted {
		// Re-read rather than trusting the scan's snapshot: writes require the node stopped, so
		// this is the same value, and reading it here keeps the arithmetic auditable per row.
		let counter_before = read_counter(db, &content_hash)?.unwrap_or(0);
		let counter_after = counter_before.saturating_add(delta);

		let mut applied_row = false;
		if apply {
			let mut tx = db.transaction();
			tx.put(columns::TRANSACTION, &counter_key(&content_hash), &counter_after.to_le_bytes());
			db.write(tx)?;
			applied_row = true;
		}

		report.rows.push(RefcountRow {
			content_hash,
			counter_before,
			counter_after,
			at_risk: drift.at_risk_drift.contains_key(&content_hash),
			applied: applied_row,
		});
	}

	report.elapsed = started.elapsed();
	Ok(report)
}

impl fmt::Display for RefcountBackfillReport {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(f, "Refcount backfill")?;
		writeln!(f, "=================")?;
		writeln!(f, "Elapsed:               {:?}", self.elapsed)?;
		writeln!(f, "Mode:                  {}", if self.applied { "APPLY" } else { "DRY-RUN" })?;
		writeln!(f, "Drifted counters:      {}", self.rows.len())?;
		writeln!(f, "  at-risk:             {}", self.at_risk())?;
		writeln!(f, "Units to add:          {}", self.units())?;

		if self.rows.is_empty() {
			writeln!(f)?;
			writeln!(
				f,
				"Result: nothing to backfill — every counter matches its reference count."
			)?;
			return Ok(());
		}

		writeln!(f)?;
		for r in &self.rows {
			writeln!(
				f,
				"  {}  counter {} → {}  (+{}){}",
				hex(r.content_hash.as_ref()),
				r.counter_before,
				r.counter_after,
				r.counter_after - r.counter_before,
				if r.at_risk { "  at-risk" } else { "" },
			)?;
		}

		writeln!(f)?;
		if self.applied {
			writeln!(
				f,
				"Result: {} counter(s) written, {} units added. Re-run `drift` to confirm clean.",
				self.rows.len(),
				self.units(),
			)?;
		} else {
			writeln!(
				f,
				"Result: {} counter(s) would be written, {} units added. Re-run with --apply \
				 (node stopped) to do it.",
				self.rows.len(),
				self.units(),
			)?;
		}
		Ok(())
	}
}
