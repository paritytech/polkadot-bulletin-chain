// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! On-disk verification of the transaction-storage columns.
//!
//! Reads a running node's database through a rocksdb *secondary* instance, which takes no lock.
//! That view is point-in-time — a write the node has only just made may not be visible — so
//! every assertion polls until the expected state appears or the timeout expires.

use anyhow::{anyhow, Context, Result};
use std::{
	path::{Path, PathBuf},
	time::Duration,
};
pub use tx_index_tool::DbHash;
use tx_index_tool::{
	dry_run, inspect_block, list_entries, open_database, verify_seams, KeyValueDB, ListOptions,
	OpenMode, StorageEntry,
};

/// How long the polling assertions wait for the expected state to appear.
///
/// `--blocks-pruning` deletion and the col11 refcount-zero cleanup that follows it run
/// asynchronously after the finalized head crosses the boundary; that lag has been observed
/// above 180s under CI load. A passing assertion returns as soon as its predicate holds.
const ASSERT_TIMEOUT: Duration = Duration::from_secs(300);
/// Gap between polls while waiting.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// The key a `store` files its data under, as a `DbHash` for the assertions below.
pub fn content_hash(data: &[u8]) -> DbHash {
	DbHash::from(crate::utils::crypto::blake2_256(data))
}

/// Path: `<base_dir>/<node_name>/data/chains/<chain_id>/db/full/`
pub fn get_db_path(base_dir: &str, node_name: &str, chain_id: &str) -> PathBuf {
	Path::new(base_dir)
		.join(node_name)
		.join("data")
		.join("chains")
		.join(chain_id)
		.join("db")
		.join("full")
}

/// Run `f` against a lock-free secondary instance of the node's database.
///
/// `tag` must differ per node: two secondary instances in one process cannot share a state
/// directory, and the tests read several nodes.
fn with_db<T>(
	db_path: &Path,
	tag: &str,
	f: impl FnOnce(&dyn KeyValueDB) -> Result<T>,
) -> Result<T> {
	let mode = OpenMode::new(true, None).sibling(tag);
	let db = open_database(db_path, &mode)
		.with_context(|| format!("opening {} as a secondary instance", db_path.display()))?;
	let out = f(&db);
	drop(db);
	mode.cleanup();
	out
}

/// What col11 holds, as seen through the secondary.
#[derive(Debug, Clone)]
pub struct Col11 {
	/// Stored values (32-byte keys).
	pub values: u64,
	/// Refcount rows (`hash || 0x00`).
	pub counters: u64,
	/// Rows that are neither — should always be zero.
	pub unexpected_keys: u64,
	/// Values that do not hash to the key they are filed under.
	pub corrupted: u64,
	/// Every entry, with size, refcount, algorithm and the blocks referencing it.
	pub entries: Vec<StorageEntry>,
}

impl Col11 {
	pub fn is_empty(&self) -> bool {
		self.values == 0 && self.counters == 0
	}

	pub fn entry(&self, hash: &DbHash) -> Option<&StorageEntry> {
		self.entries.iter().find(|e| &e.content_hash == hash)
	}

	pub fn refcount(&self, hash: &DbHash) -> Option<u32> {
		self.entry(hash).and_then(|e| e.counter)
	}

	pub fn log(&self, label: &str) {
		tracing::info!(
			"{label}: {} value(s), {} counter(s), {} corrupted, {} unexpected key(s)",
			self.values,
			self.counters,
			self.corrupted,
			self.unexpected_keys,
		);
		for e in &self.entries {
			tracing::info!(
				"  {:?} {} bytes, refcount {:?}, algo {}, blocks {:?}..{:?} ({} referrer(s))",
				e.content_hash,
				e.size,
				e.counter,
				e.algo.map(|a| a.name()).unwrap_or("CORRUPTED"),
				e.first_block,
				e.last_block,
				e.referring_blocks,
			);
		}
	}
}

/// Read col11 once.
pub fn read_col11(db_path: &Path, tag: &str) -> Result<Col11> {
	with_db(db_path, tag, |db| {
		// `limit: None` so assertions see every entry; test chains hold a handful.
		let opts = ListOptions { limit: None, preview_len: 0, ..Default::default() };
		let report = list_entries(db, &opts)?;
		Ok(Col11 {
			values: report.value_entries,
			counters: report.counter_entries,
			unexpected_keys: report.unexpected_key_rows,
			corrupted: report.values_corrupted,
			entries: report.entries,
		})
	})
}

/// Poll col11 until `pred` holds. Returns the matching snapshot, or an error naming what was
/// expected and what the last observation was.
pub async fn await_col11(
	db_path: &Path,
	tag: &str,
	expectation: &str,
	pred: impl Fn(&Col11) -> bool,
) -> Result<Col11> {
	let deadline = std::time::Instant::now() + ASSERT_TIMEOUT;
	loop {
		let snapshot = read_col11(db_path, tag)?;
		if pred(&snapshot) {
			snapshot.log(expectation);
			return Ok(snapshot);
		}
		if std::time::Instant::now() >= deadline {
			snapshot.log(&format!("{expectation} — LAST OBSERVED"));
			return Err(anyhow!(
				"col11 did not reach the expected state within {ASSERT_TIMEOUT:?}: \
				 {expectation}"
			));
		}
		tokio::time::sleep(POLL_INTERVAL).await;
	}
}

/// Assert col11 holds nothing — before the first store, or after everything expired.
pub async fn assert_col11_empty(db_path: &Path, tag: &str, label: &str) -> Result<()> {
	await_col11(db_path, tag, &format!("{label}: col11 empty"), |c| c.is_empty()).await?;
	Ok(())
}

/// Assert exactly one stored value, with the given content hash, size and refcount.
pub async fn assert_single_entry(
	db_path: &Path,
	tag: &str,
	label: &str,
	hash: DbHash,
	size: usize,
	refcount: u32,
) -> Result<()> {
	let snapshot =
		await_col11(db_path, tag, &format!("{label}: one value with refcount {refcount}"), |c| {
			c.values == 1 && c.counters == 1 && c.refcount(&hash) == Some(refcount)
		})
		.await?;

	let entry = snapshot
		.entry(&hash)
		.ok_or_else(|| anyhow!("{label}: no col11 entry for {hash:?}"))?;
	if entry.size != size {
		anyhow::bail!("{label}: expected {size} bytes for {hash:?}, found {}", entry.size);
	}
	// `algo` is Some only when the value hashes to the key it is filed under, so this is the
	// content-hash check the ldb helper did by hand — and it names the algorithm.
	let algo = entry
		.algo
		.ok_or_else(|| anyhow!("{label}: value for {hash:?} does not hash to its key"))?;
	tracing::info!("✓ {label}: {hash:?} verified under {}, refcount {refcount}", algo.name());
	assert_column_sane(&snapshot, label)
}

/// Assert every one of `items` is on disk under its content hash, at its submitted size, with
/// at least one reference — and that col11 holds nothing beyond them.
pub async fn assert_items_stored(
	db_path: &Path,
	tag: &str,
	label: &str,
	items: &[&[u8]],
) -> Result<Col11> {
	let expected: Vec<(DbHash, usize)> = items.iter().map(|d| (content_hash(d), d.len())).collect();
	let snapshot =
		await_col11(db_path, tag, &format!("{label}: {} item(s) stored", items.len()), |c| {
			c.values == expected.len() as u64 &&
				expected.iter().all(|(h, _)| c.refcount(h).is_some_and(|n| n >= 1))
		})
		.await?;

	for (hash, size) in &expected {
		let entry = snapshot
			.entry(hash)
			.ok_or_else(|| anyhow!("{label}: no col11 entry for {hash:?}"))?;
		if entry.size != *size {
			anyhow::bail!("{label}: expected {size} bytes for {hash:?}, found {}", entry.size);
		}
		entry
			.algo
			.ok_or_else(|| anyhow!("{label}: value for {hash:?} does not hash to its key"))?;
	}
	tracing::info!("✓ {label}: all {} item(s) stored and verified", items.len());
	assert_column_sane(&snapshot, label)?;
	Ok(snapshot)
}

/// Assert how many times one block's body references `hash` — `Indexed` occurrences plus every
/// `MultiRenew` inner count. Unlike a referring-block count this does not depend on how far
/// pruning has progressed elsewhere, only on `block` itself still being retained.
pub async fn assert_block_references(
	db_path: &Path,
	tag: &str,
	label: &str,
	block: u32,
	hash: DbHash,
	expected: u32,
) -> Result<()> {
	let count = |db: &dyn KeyValueDB| -> Result<Option<u32>> {
		let Some(inspection) = inspect_block(db, block)? else { return Ok(None) };
		if let Some(err) = &inspection.decode_failure {
			anyhow::bail!("{label}: block #{block} BODY_INDEX failed to decode: {err}");
		}
		Ok(Some(
			inspection
				.hashes
				.iter()
				.filter(|h| h.content_hash == hash)
				.map(|h| h.indexed + h.multirenew_inner.iter().sum::<u32>())
				.sum(),
		))
	};

	// The block may not have reached the secondary view yet, so poll rather than read once.
	let deadline = std::time::Instant::now() + ASSERT_TIMEOUT;
	loop {
		let found = with_db(db_path, tag, count)?;
		if found == Some(expected) {
			tracing::info!("✓ {label}: block #{block} references {hash:?} {expected} time(s)");
			return Ok(());
		}
		if std::time::Instant::now() >= deadline {
			anyhow::bail!(
				"{label}: block #{block} references {hash:?} {} time(s), expected {expected}",
				match found {
					Some(n) => n.to_string(),
					None => "no (block absent)".to_string(),
				},
			);
		}
		tokio::time::sleep(POLL_INTERVAL).await;
	}
}

/// Assert an entry is gone: no value, no counter row.
pub async fn assert_absent(db_path: &Path, tag: &str, label: &str, hash: DbHash) -> Result<()> {
	await_col11(db_path, tag, &format!("{label}: {hash:?} absent"), |c| c.entry(&hash).is_none())
		.await?;
	tracing::info!("✓ {label}: {hash:?} is no longer stored");
	Ok(())
}

/// Every value hashes to its key, and no unrecognised key shapes are present.
pub fn assert_column_sane(snapshot: &Col11, label: &str) -> Result<()> {
	if snapshot.corrupted != 0 {
		anyhow::bail!("{label}: {} col11 value(s) do not hash to their key", snapshot.corrupted);
	}
	if snapshot.unexpected_keys != 0 {
		anyhow::bail!(
			"{label}: {} col11 row(s) are neither a value nor a counter",
			snapshot.unexpected_keys
		);
	}
	if snapshot.values != snapshot.counters {
		anyhow::bail!(
			"{label}: {} values but {} counter rows — one of each is expected",
			snapshot.values,
			snapshot.counters,
		);
	}
	// Every block listing a hash contributes at least one reference, so a counter below the
	// number of referring blocks means references were lost — the polkadot-sdk#12106 collapse
	// class. The reverse is legitimate: one `MultiRenew` can reference a hash several times.
	//
	// The snapshot is internally consistent because a secondary instance only advances its
	// view on catch-up, so col11 and BODY_INDEX are read at the same point in the WAL.
	for e in &snapshot.entries {
		let counter = e.counter.unwrap_or(0);
		if counter < e.referring_blocks {
			anyhow::bail!(
				"{label}: refcount {counter} for {:?} is below the {} block(s) referencing it",
				e.content_hash,
				e.referring_blocks,
			);
		}
	}
	Ok(())
}

/// No refcount is short of the references its blocks actually carry (the polkadot-sdk#12106
/// collapse class), and nothing failed to decode.
pub fn assert_no_refcount_drift(db_path: &Path, tag: &str, label: &str) -> Result<()> {
	with_db(db_path, tag, |db| {
		let report = dry_run(db)?;
		if report.decode_failures != 0 {
			anyhow::bail!(
				"{label}: {} BODY_INDEX entries failed to decode",
				report.decode_failures
			);
		}
		if !report.on_disk_drift.is_empty() {
			anyhow::bail!(
				"{label}: {} refcount(s) short by {} units in total",
				report.on_disk_drift.len(),
				report.total_units_to_backfill(),
			);
		}
		if !report.on_disk_excess.is_empty() {
			anyhow::bail!(
				"{label}: {} refcount(s) exceed their reference count",
				report.on_disk_excess.len(),
			);
		}
		tracing::info!(
			"✓ {label}: refcounts agree across {} BODY_INDEX entr(ies), {} with intra-block \
			 duplicates",
			report.blocks_scanned,
			report.blocks_with_duplicates,
		);
		Ok(())
	})
}

/// Every indexed entry's `BODY_INDEX.header ++ col11` pair round-trips, so the bodies remain
/// executable (the polkadot-bulletin-chain#574 class).
pub fn assert_seams_clean(db_path: &Path, tag: &str, label: &str) -> Result<()> {
	with_db(db_path, tag, |db| {
		let report = verify_seams(db)?;
		if !report.is_clean() {
			anyhow::bail!(
				"{label}: {} mis-split and {} half-repaired entr(ies); blocks {:?} would not execute",
				report.original_misaligned,
				report.half_repaired,
				report.unexecutable_blocks(),
			);
		}
		tracing::info!("✓ {label}: {} indexed entr(ies) reassemble correctly", report.examined);
		Ok(())
	})
}

/// No stored value is left behind with nothing referencing it.
pub fn assert_no_orphans(db_path: &Path, tag: &str, label: &str) -> Result<()> {
	with_db(db_path, tag, |db| {
		let opts =
			ListOptions { limit: None, preview_len: 0, orphans_only: true, ..Default::default() };
		let report = list_entries(db, &opts)?;
		if report.matched != 0 {
			anyhow::bail!(
				"{label}: {} col11 value(s) survive with no block referencing them",
				report.matched,
			);
		}
		tracing::info!("✓ {label}: no orphaned values");
		Ok(())
	})
}

/// Assert how a block's body references its indexed data: how many standalone `Indexed`
/// entries, and the per-`MultiRenew` inner counts. This is the only way to observe the
/// `MultiRenew` shape that a batch renewal produces.
pub fn assert_block_shape(
	db_path: &Path,
	tag: &str,
	label: &str,
	block: u32,
	expect_indexed: usize,
	expect_multirenew_inner: &[u32],
) -> Result<()> {
	with_db(db_path, tag, |db| {
		let inspection = inspect_block(db, block)?
			.ok_or_else(|| anyhow!("{label}: block #{block} is not in the database"))?;
		if let Some(err) = &inspection.decode_failure {
			anyhow::bail!("{label}: block #{block} BODY_INDEX failed to decode: {err}");
		}
		if inspection.indexed_entries != expect_indexed {
			anyhow::bail!(
				"{label}: block #{block} has {} Indexed entr(ies), expected {expect_indexed}",
				inspection.indexed_entries,
			);
		}
		let mut inner: Vec<u32> =
			inspection.hashes.iter().flat_map(|h| h.multirenew_inner.clone()).collect();
		inner.sort_unstable();
		let mut expected = expect_multirenew_inner.to_vec();
		expected.sort_unstable();
		if inner != expected {
			anyhow::bail!(
				"{label}: block #{block} MultiRenew inner counts {inner:?}, expected {expected:?}",
			);
		}
		tracing::info!(
			"✓ {label}: block #{block} shape — {} Indexed, MultiRenew inner {expected:?}",
			expect_indexed,
		);
		Ok(())
	})
}

/// The full on-disk health check: column sanity, refcount agreement, seam integrity, orphans.
pub async fn assert_storage_healthy(db_path: &Path, tag: &str, label: &str) -> Result<()> {
	let snapshot = read_col11(db_path, tag)?;
	assert_column_sane(&snapshot, label)?;
	assert_no_refcount_drift(db_path, tag, label)?;
	assert_seams_clean(db_path, tag, label)?;
	assert_no_orphans(db_path, tag, label)
}

/// Assert an entry's referring blocks: how many alive blocks list it, and the highest-numbered
/// one. A renewal must add a block, so `last_block` moving forward is the on-disk evidence that
/// the renewal actually re-anchored the data rather than merely bumping a counter.
pub async fn assert_referrers(
	db_path: &Path,
	tag: &str,
	label: &str,
	hash: DbHash,
	blocks: u32,
	last_block_above: Option<u32>,
) -> Result<Col11> {
	let expectation = match last_block_above {
		Some(n) => format!("{label}: {hash:?} in {blocks} block(s), latest above #{n}"),
		None => format!("{label}: {hash:?} in {blocks} block(s)"),
	};
	let snapshot = await_col11(db_path, tag, &expectation, |c| {
		c.entry(&hash).is_some_and(|e| {
			e.referring_blocks == blocks &&
				last_block_above.is_none_or(|n| e.last_block.is_some_and(|l| l > n))
		})
	})
	.await?;
	let entry = snapshot.entry(&hash).expect("predicate matched; qed");
	tracing::info!(
		"✓ {label}: {hash:?} referenced by {blocks} block(s), latest #{:?}",
		entry.last_block,
	);
	Ok(snapshot)
}
