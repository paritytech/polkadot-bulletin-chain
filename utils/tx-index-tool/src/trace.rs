// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! The reference ledger for a single content hash: every block that holds a reference on it,
//! what each contributes, and how the total compares with the one counter on disk.
//!
//! col11 keeps a single counter per entry, not a history, so a past value cannot be recovered.
//! What can be reconstructed is the ledger: walk `BODY_INDEX`, and every alive block that names
//! the hash contributes its occurrence count. The sum is what the counter must hold. A counter
//! below it means references were lost; above it means releases were missed; a hash referenced
//! by alive blocks with no value at all is a dangling reference, which no renewal can repair.

use crate::{
	chain::ChainFacts,
	common::{
		block_timestamp_ms, columns, fmt_occurrences, format_timestamp_ms, hex, read_counter,
		split_lookup_key, BareDbExtrinsic, DbHash, HashAlgo, Occurrences,
	},
};
use codec::Decode;
use kvdb::KeyValueDB;
use std::{
	collections::BTreeMap,
	fmt,
	time::{Duration, Instant},
};

/// What the ledger says about one block.
#[derive(Debug, Clone)]
pub struct TraceRow {
	/// Block number.
	pub block: u32,
	/// Reference shape from this database's `BODY_INDEX`. `None` when the block holds no
	/// reference here — either it never did, or its release has already happened.
	pub occurrences: Option<Occurrences>,
	/// Authoring time recovered from the block's timestamp inherent.
	pub time_ms: Option<u64>,
	/// Whether the chain agrees a reference was taken here. `None` when no chain was consulted.
	pub chain_took_reference: Option<bool>,
	/// The chain's account of this block, for the report.
	pub chain_summary: Option<String>,
	/// The chunk root the chain committed to here.
	pub chunk_root: Option<DbHash>,
}

impl TraceRow {
	/// References this block contributes in this database.
	pub fn delta(&self) -> u32 {
		self.occurrences.as_ref().map(|o| o.total()).unwrap_or(0)
	}

	/// The chain recorded a reference here that this database does not hold. Either the block
	/// was pruned (normal) or its reference was released early (not normal).
	pub fn released_or_missing(&self) -> bool {
		self.occurrences.is_none() && self.chain_took_reference == Some(true)
	}

	/// This database holds a reference the chain has no record of.
	pub fn spurious(&self) -> bool {
		self.occurrences.is_some() && self.chain_took_reference == Some(false)
	}
}

/// The verdict a trace arrives at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
	/// Nothing references the hash and nothing is stored — it is simply not here.
	Absent,
	/// Alive blocks reference the hash but no value is stored. Renewals cannot repair this: a
	/// renew extrinsic carries only the hash, and `Reference` on a missing counter is a no-op.
	Dangling { referring_blocks: usize },
	/// The counter is below the references the alive blocks carry.
	Short { expected: u32, actual: u32 },
	/// The counter is above them, so releases were missed.
	Excess { expected: u32, actual: u32 },
	/// Counter and ledger agree.
	Consistent { total: u32 },
}

impl Verdict {
	/// Whether this is something an operator has to act on.
	pub fn is_finding(&self) -> bool {
		!matches!(self, Verdict::Consistent { .. })
	}
}

/// Result of a trace.
#[derive(Debug)]
pub struct TraceReport {
	/// The hash that was traced.
	pub content_hash: DbHash,
	/// Size of the stored value, when one is present.
	pub value_size: Option<usize>,
	/// Which algorithm reproduces the key from the value. `None` with a present value means the
	/// value is corrupted.
	pub algo: Option<HashAlgo>,
	/// On-disk counter, `None` when the row is absent.
	pub counter: Option<u32>,
	/// One row per block that holds — or should hold — a reference, ascending.
	pub rows: Vec<TraceRow>,
	/// Sum of the references the alive blocks carry.
	pub alive_total: u32,
	/// `BODY_INDEX` entries walked.
	pub blocks_scanned: u64,
	/// Chain-side context, when `--rpc-url` was given.
	pub chain: Option<ChainFacts>,
	/// Wall-clock duration of the database pass.
	pub elapsed: Duration,
}

impl TraceReport {
	/// The verdict, derived from the value, the counter and the ledger.
	pub fn verdict(&self) -> Verdict {
		let referring = self.rows.iter().filter(|r| r.occurrences.is_some()).count();
		if self.value_size.is_none() {
			return if referring == 0 {
				Verdict::Absent
			} else {
				Verdict::Dangling { referring_blocks: referring }
			};
		}
		let actual = self.counter.unwrap_or(0);
		match actual.cmp(&self.alive_total) {
			std::cmp::Ordering::Less => Verdict::Short { expected: self.alive_total, actual },
			std::cmp::Ordering::Greater => Verdict::Excess { expected: self.alive_total, actual },
			std::cmp::Ordering::Equal => Verdict::Consistent { total: actual },
		}
	}

	/// Blocks the chain says took a reference that this database no longer holds.
	pub fn released(&self) -> Vec<u32> {
		self.rows.iter().filter(|r| r.released_or_missing()).map(|r| r.block).collect()
	}

	/// Blocks this database references that the chain has no record of.
	pub fn spurious(&self) -> Vec<u32> {
		self.rows.iter().filter(|r| r.spurious()).map(|r| r.block).collect()
	}

	/// Every block that holds a reference here, ascending — the input to a chain cross-check.
	pub fn referring_blocks(&self) -> Vec<u32> {
		self.rows.iter().filter(|r| r.occurrences.is_some()).map(|r| r.block).collect()
	}
}

/// Walk `BODY_INDEX` and build the reference ledger for `content_hash`.
///
/// Read-only, one full `BODY_INDEX` pass. Chain facts are merged in separately by
/// [`merge_chain`] so this stays offline and testable.
pub fn trace_hash(db: &dyn KeyValueDB, content_hash: DbHash) -> std::io::Result<TraceReport> {
	let started = Instant::now();

	let value = db.get(columns::TRANSACTION, content_hash.as_ref())?;
	let value_size = value.as_ref().map(|v| v.len());
	let algo = value.as_ref().and_then(|v| HashAlgo::identify(content_hash, v));
	let counter = read_counter(db, &content_hash)?;

	let mut per_block: BTreeMap<u32, (Occurrences, Option<u64>)> = BTreeMap::new();
	let mut blocks_scanned = 0u64;

	for entry in db.iter(columns::BODY_INDEX) {
		let (k, v) = entry?;
		blocks_scanned += 1;
		let Some((number, _)) = split_lookup_key(&k) else { continue };
		let Ok(index) = Vec::<BareDbExtrinsic>::decode(&mut &v[..]) else { continue };

		let mut occ = Occurrences::default();
		let mut fulls: Vec<Vec<u8>> = Vec::new();
		for ex in index {
			match ex {
				BareDbExtrinsic::Indexed { hash, .. } =>
					if hash == content_hash {
						occ.indexed += 1;
					},
				BareDbExtrinsic::MultiRenew { hashes, .. } => {
					let inner = hashes.iter().filter(|h| **h == content_hash).count() as u32;
					if inner > 0 {
						occ.multirenew_inner.push(inner);
					}
				},
				BareDbExtrinsic::Full(bytes) => fulls.push(bytes),
			}
		}
		if occ.total() > 0 {
			per_block.insert(number, (occ, block_timestamp_ms(&fulls)));
		}
	}

	let rows: Vec<TraceRow> = per_block
		.into_iter()
		.map(|(block, (occ, time_ms))| TraceRow {
			block,
			occurrences: Some(occ),
			time_ms,
			chain_took_reference: None,
			chain_summary: None,
			chunk_root: None,
		})
		.collect();
	let alive_total = rows.iter().map(|r| r.delta()).sum();

	Ok(TraceReport {
		content_hash,
		value_size,
		algo,
		counter,
		rows,
		alive_total,
		blocks_scanned,
		chain: None,
		elapsed: started.elapsed(),
	})
}

/// Fold chain facts into a report, adding rows for blocks the chain knows about that this
/// database holds no reference for.
pub fn merge_chain(report: &mut TraceReport, facts: ChainFacts) {
	let mut extra: Vec<TraceRow> = Vec::new();
	for (number, bf) in &facts.per_block {
		match report.rows.iter_mut().find(|r| r.block == *number) {
			Some(row) => {
				row.chain_took_reference = Some(bf.took_a_reference());
				row.chain_summary = Some(bf.summary());
				row.chunk_root = bf.chunk_root;
			},
			None =>
			// Only worth a row if the chain actually recorded something there; cadence probes
			// that found nothing are noise.
				if bf.took_a_reference() || !bf.events.is_empty() {
					extra.push(TraceRow {
						block: *number,
						occurrences: None,
						time_ms: None,
						chain_took_reference: Some(bf.took_a_reference()),
						chain_summary: Some(bf.summary()),
						chunk_root: bf.chunk_root,
					});
				},
		}
	}
	report.rows.extend(extra);
	report.rows.sort_by_key(|r| r.block);
	report.chain = Some(facts);
}

impl fmt::Display for TraceReport {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(f, "Reference trace for {}", hex(self.content_hash.as_ref()))?;
		writeln!(
			f,
			"=============================================================================="
		)?;
		writeln!(
			f,
			"Scan duration:         {:?}  ({} BODY_INDEX entries)",
			self.elapsed, self.blocks_scanned
		)?;
		match (self.value_size, self.algo) {
			(Some(n), Some(algo)) =>
				writeln!(f, "Value on disk:         {n} bytes, {}(value) == key", algo.name())?,
			(Some(n), None) => writeln!(
				f,
				"Value on disk:         {n} bytes, CORRUPTED — hashes to none of the known \
				 algorithms",
			)?,
			(None, _) => writeln!(f, "Value on disk:         <absent>")?,
		}
		writeln!(
			f,
			"Counter on disk:       {}",
			self.counter.map(|c| c.to_string()).unwrap_or_else(|| "<absent>".into()),
		)?;

		if let Some(chain) = &self.chain {
			writeln!(
				f,
				"Chain:                 {}  head #{}, finalized #{}{}",
				chain.url,
				chain.head,
				chain.finalized,
				match chain.retention_period {
					Some(rp) => format!(", RetentionPeriod {rp}"),
					None => String::new(),
				},
			)?;
			match chain.latest_location {
				Some((block, index)) => writeln!(
					f,
					"Chain location:        Transactions(#{block}) index {index}{}",
					match chain.proof_due_at(block) {
						Some(due) => format!("   next proof due at #{due}"),
						None => String::new(),
					},
				)?,
				None => writeln!(
					f,
					"Chain location:        none — TransactionByContentHash has no entry, so the \
					 chain considers this data expired",
				)?,
			}
		}

		if self.rows.is_empty() {
			writeln!(f)?;
			writeln!(f, "No block references this hash, and nothing is stored under it.")?;
			return Ok(());
		}

		writeln!(f)?;
		writeln!(
			f,
			"  {:<10} {:>4} {:>6}  {:<16} {:<22} chain",
			"block", "Δ", "cum", "body shape", "authored",
		)?;
		let mut cum = 0u32;
		for row in &self.rows {
			let (delta, cum_s) = match &row.occurrences {
				Some(occ) => {
					cum += occ.total();
					(format!("+{}", occ.total()), cum.to_string())
				},
				None => ("-".to_string(), "-".to_string()),
			};
			let shape = match &row.occurrences {
				Some(occ) => fmt_occurrences(occ),
				None => "not in this database".to_string(),
			};
			let when = row.time_ms.map(format_timestamp_ms).unwrap_or_else(|| String::from(""));
			let mut chain_s = row.chain_summary.clone().unwrap_or_default();
			if row.released_or_missing() {
				chain_s.push_str("   ← reference released here");
			}
			if row.spurious() {
				chain_s.push_str("   ← chain has no record of this");
			}
			writeln!(
				f,
				"  #{:<9} {:>4} {:>6}  {:<16} {:<22} {}",
				row.block, delta, cum_s, shape, when, chain_s,
			)?;
		}

		writeln!(f)?;
		writeln!(
			f,
			"Alive references:      {}   (sum over {} referring block(s))",
			self.alive_total,
			self.referring_blocks().len(),
		)?;

		let verdict = self.verdict();
		writeln!(f)?;
		match &verdict {
			Verdict::Consistent { total } =>
				writeln!(f, "Result: CONSISTENT — the counter holds {total}, matching the ledger.")?,
			Verdict::Short { expected, actual } => {
				writeln!(
					f,
					"Result: COUNTER SHORT — {actual} on disk against {expected} references \
					 carried by alive blocks.",
				)?;
				writeln!(
					f,
					"  The first block to prune will decrement by its whole occurrence count, so \
					 the value can reach zero while other blocks still reference it. See \
					 `incident sdk-12106 drift`.",
				)?;
			},
			Verdict::Excess { expected, actual } => writeln!(
				f,
				"Result: COUNTER EXCESS — {actual} on disk against {expected} references; \
				 {} release(s) were missed, so the value will never be reclaimed.",
				actual - expected,
			)?,
			Verdict::Dangling { referring_blocks } => {
				writeln!(
					f,
					"Result: DANGLING — {referring_blocks} alive block(s) reference this hash and \
					 no value is stored.",
				)?;
				writeln!(
					f,
					"  A renewal cannot repair this: the extrinsic carries only the hash, and a \
					 reference against a missing counter is a silent no-op. Any block whose proof \
					 targets one of those blocks cannot be authored.",
				)?;
			},
			Verdict::Absent =>
				writeln!(f, "Result: ABSENT — nothing stored, and no block references it.")?,
		}

		let released = self.released();
		if !released.is_empty() {
			writeln!(f)?;
			writeln!(
				f,
				"The chain recorded a reference at {} block(s) this database no longer holds:",
				released.len(),
			)?;
			writeln!(f, "  {}", crate::common::joined_blocks(&released, 30))?;
			writeln!(
				f,
				"  Normal if those blocks were pruned. If any is inside the pruning window, its \
				 reference was released early.",
			)?;
		}
		let spurious = self.spurious();
		if !spurious.is_empty() {
			writeln!(f)?;
			writeln!(
				f,
				"This database references {} block(s) the chain has no entry for: {}",
				spurious.len(),
				crate::common::joined_blocks(&spurious, 30),
			)?;
		}

		// The chunk root turns `proof` from a self-consistency check into a real one.
		if let Some(root) = self.rows.iter().rev().find_map(|r| r.chunk_root.map(|k| (r.block, k)))
		{
			writeln!(f)?;
			writeln!(f, "Verify the stored bytes against what the chain committed to:")?;
			writeln!(
				f,
				"  tx-index-tool proof <db> {} --expect-root {}",
				root.0,
				hex(root.1.as_ref()),
			)?;
		}
		Ok(())
	}
}
