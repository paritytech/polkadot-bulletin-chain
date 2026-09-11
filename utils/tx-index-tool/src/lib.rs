// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Inspection and repair of a Bulletin Chain node's indexed transaction storage.
//!
//! Everything here works directly on the node's RocksDB directory, without a runtime: the
//! `BODY_INDEX` entries are decoded through a mirror of `sc-client-db`'s private
//! `DbExtrinsic` whose `Full` variant is opaque bytes, which is SCALE-identical and keeps the
//! tool runtime-agnostic.
//!
//! Two columns matter:
//!
//! - `TRANSACTION` (11) holds the stored data as `content_hash -> bytes`, alongside a `content_hash
//!   || 0x00 -> LE u32` refcount for each.
//! - `BODY_INDEX` (12) holds, per block, the list of transactions its body stores or renews.
//!
//! Reads never need the node stopped — see [`OpenMode`].

pub mod chain;
pub mod common;
pub mod diff;
pub mod drift;
pub mod inspect;
pub mod listing;
pub mod proof;
pub mod realign;
pub mod repair;
pub mod seam;
pub mod trace;

#[cfg(test)]
mod tests;

// Re-export the entry points the binary and tests use, rather than glob-flattening every
// module: with an explicit list, anything that stops being used shows up as dead code.

// The scan functions below are expressed in terms of these two, so a consumer that wants to
// open a database once and run several of them needs to be able to name them.
pub use kvdb::KeyValueDB;
pub use kvdb_rocksdb::Database;

pub use common::{
	best_block, block_hash, block_lookup_key, block_referenced_hashes, block_timestamp_ms, columns,
	counter_key, format_timestamp_ms, hex, hexdump, joined, joined_blocks, open_database,
	parse_hex32, read_counter, split_lookup_key, BareDbExtrinsic, DbHash, HashAlgo, Occurrences,
	OpenMode, NUM_COLUMNS,
};
pub use diff::{diff_databases, BlockDiff, DiffOptions, DiffReport, EntryDiff, EntryFacts};
pub use drift::{dry_run, AffectedBlock, BlockOccurrence, DryRunReport, DuplicatePattern};
pub use inspect::{inspect_block, BlockHashInfo, BlockInspection};
pub use listing::{list_entries, EntriesReport, EntrySort, ListOptions, StorageEntry};
pub use proof::{compute_storage_proof, ProofResult};
pub use realign::{
	find_alignment, realign_all_corrupted, realign_from_body_index, Alignment, BatchRealignReport,
	RealignOutcome,
};
pub use repair::{
	repair_refcounts, repair_value, RefcountBackfillReport, RefcountRow, RepairOutcome,
};
pub use seam::{classify, verify_seams, SeamEntry, SeamReport, SeamState};
pub use trace::{merge_chain, trace_hash, TraceReport, TraceRow, Verdict};
