// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Unit tests over an in-memory kvdb.

use crate::*;
use codec::Encode;
use kvdb::KeyValueDB;
use kvdb_memorydb::create;

fn h(byte: u8) -> DbHash {
	let mut bytes = [0u8; 32];
	bytes[0] = byte;
	DbHash::from(bytes)
}

fn put_body(db: &dyn KeyValueDB, key: &[u8], items: Vec<BareDbExtrinsic>) {
	let mut tx = db.transaction();
	tx.put(columns::BODY_INDEX, key, &items.encode());
	db.write(tx).unwrap();
}

/// Seed the on-disk col11 counter for a hash. Simulates what the kvdb commit path would
/// have written under either old (buggy) or new (correct) code.
fn seed_counter(db: &dyn KeyValueDB, hash: &DbHash, count: u32) {
	let mut tx = db.transaction();
	tx.put(columns::TRANSACTION, &counter_key(hash), &count.to_le_bytes());
	db.write(tx).unwrap();
}

#[test]
fn clean_db_no_patterns_no_drift() {
	let db = create(NUM_COLUMNS);
	put_body(&db, b"b1", vec![BareDbExtrinsic::Indexed { hash: h(1), header: vec![] }]);
	put_body(&db, b"b2", vec![BareDbExtrinsic::Full(vec![1, 2, 3])]);

	let report = dry_run(&db).unwrap();
	assert!(report.is_clean());
	assert_eq!(report.blocks_scanned, 2);
	assert_eq!(report.blocks_with_duplicates, 0);
	assert!(report.body_pattern_undercount.is_empty());
	assert!(report.on_disk_drift.is_empty());
}

#[test]
fn body_pattern_with_correct_counter_no_drift() {
	// Body has 2× h(1); counter correctly set to 2 — fresh-sync new-code state.
	let db = create(NUM_COLUMNS);
	put_body(
		&db,
		b"b",
		vec![
			BareDbExtrinsic::Indexed { hash: h(1), header: vec![] },
			BareDbExtrinsic::Indexed { hash: h(1), header: vec![] },
		],
	);
	seed_counter(&db, &h(1), 2);

	let report = dry_run(&db).unwrap();
	assert!(report.is_clean(), "counter matches expected → no drift");
	assert_eq!(report.body_pattern_hashes(), 1, "body pattern still detected");
	assert_eq!(report.on_disk_drifted_hashes(), 0, "but no actual drift");
	assert_eq!(report.blocks_with_duplicates, 1);
	assert!(report.affected_blocks[0].patterns[0].on_disk_correct);
}

#[test]
fn body_pattern_with_collapsed_counter_drifts_but_saturates() {
	// Old bug: counter only got +1 (collapsed). Sole referrer → saturates safely.
	let db = create(NUM_COLUMNS);
	put_body(
		&db,
		b"b",
		vec![
			BareDbExtrinsic::Indexed { hash: h(1), header: vec![] },
			BareDbExtrinsic::Indexed { hash: h(1), header: vec![] },
		],
	);
	seed_counter(&db, &h(1), 1);

	let report = dry_run(&db).unwrap();
	assert!(report.is_clean(), "drift exists but sole referrer saturates");
	assert_eq!(report.on_disk_drifted_hashes(), 1);
	assert_eq!(report.on_disk_drift.get(&h(1)), Some(&1));
	assert_eq!(report.at_risk_hashes(), 0);
	assert!(!report.affected_blocks[0].patterns[0].on_disk_correct);
}

#[test]
fn body_pattern_with_collapsed_counter_and_other_referrer_is_at_risk() {
	// Block A has 2× h(1); collapsed counter +1 from A. Block B also referenced h(1)
	// (+1 cross-tx). Counter=2; expected=3 → drift=1, at-risk.
	let db = create(NUM_COLUMNS);
	put_body(
		&db,
		b"A",
		vec![
			BareDbExtrinsic::Indexed { hash: h(1), header: vec![] },
			BareDbExtrinsic::Indexed { hash: h(1), header: vec![] },
		],
	);
	put_body(&db, b"B", vec![BareDbExtrinsic::Indexed { hash: h(1), header: vec![] }]);
	seed_counter(&db, &h(1), 2);

	let report = dry_run(&db).unwrap();
	assert!(!report.is_clean());
	assert_eq!(report.at_risk_hashes(), 1);
	assert_eq!(report.at_risk_drift.get(&h(1)), Some(&1));
	assert_eq!(report.at_risk_units(), 1);
	assert_eq!(report.affected_blocks.len(), 1, "B has no intra-tx dups");
	assert!(report.affected_blocks[0].patterns[0].at_risk);
	assert!(!report.affected_blocks[0].patterns[0].on_disk_correct);
}

#[test]
fn fresh_sync_state_with_multiple_blocks_no_drift() {
	// Simulates the user's "synced from scratch with new code" scenario: same body
	// patterns exist, counters correctly seeded.
	let db = create(NUM_COLUMNS);
	put_body(
		&db,
		b"A",
		vec![
			BareDbExtrinsic::Indexed { hash: h(5), header: vec![] },
			BareDbExtrinsic::Indexed { hash: h(5), header: vec![] },
		],
	);
	put_body(
		&db,
		b"B",
		vec![BareDbExtrinsic::MultiRenew { hashes: vec![h(5), h(5), h(5)], extrinsic: vec![] }],
	);
	put_body(&db, b"C", vec![BareDbExtrinsic::Indexed { hash: h(5), header: vec![] }]);
	seed_counter(&db, &h(5), 6); // 2 + 3 + 1

	let report = dry_run(&db).unwrap();
	assert!(report.is_clean());
	assert_eq!(report.body_pattern_hashes(), 1, "patterns detected");
	assert_eq!(report.on_disk_drifted_hashes(), 0, "but counter is correct");
	assert_eq!(report.at_risk_hashes(), 0);
	for block in &report.affected_blocks {
		for p in &block.patterns {
			assert!(p.on_disk_correct, "every pattern should report on_disk_correct");
		}
	}
}

#[test]
fn multirenew_with_duplicates_correct_counter_no_drift() {
	let db = create(NUM_COLUMNS);
	put_body(
		&db,
		b"b",
		vec![BareDbExtrinsic::MultiRenew { hashes: vec![h(7), h(7), h(7)], extrinsic: vec![] }],
	);
	seed_counter(&db, &h(7), 3);

	let report = dry_run(&db).unwrap();
	assert!(report.is_clean());
	assert_eq!(report.body_pattern_undercount.get(&h(7)), Some(&2));
	assert_eq!(report.duplicate_histogram.get(&3), Some(&1));
	assert!(report.on_disk_drift.is_empty());
}

#[test]
fn excess_on_disk_counter_flagged_separately() {
	let db = create(NUM_COLUMNS);
	put_body(
		&db,
		b"b",
		vec![
			BareDbExtrinsic::Indexed { hash: h(9), header: vec![] },
			BareDbExtrinsic::Indexed { hash: h(9), header: vec![] },
		],
	);
	seed_counter(&db, &h(9), 5);

	let report = dry_run(&db).unwrap();
	assert_eq!(report.on_disk_excess.get(&h(9)), Some(&3), "actual 5 > expected 2 ⇒ excess=3");
	assert!(report.on_disk_drift.is_empty(), "not in the under-count bucket");
}

#[test]
fn decode_failure_counted() {
	let db = create(NUM_COLUMNS);
	let mut tx = db.transaction();
	tx.put(columns::BODY_INDEX, b"bad", &[0xff, 0xff]);
	db.write(tx).unwrap();

	let report = dry_run(&db).unwrap();
	assert_eq!(report.blocks_scanned, 1);
	assert_eq!(report.decode_failures, 1);
	assert!(!report.is_clean());
}

#[test]
fn top_n_drifted_is_sorted_descending() {
	let db = create(NUM_COLUMNS);
	put_body(
		&db,
		b"b1",
		vec![
			BareDbExtrinsic::Indexed { hash: h(1), header: vec![] },
			BareDbExtrinsic::Indexed { hash: h(1), header: vec![] },
		],
	);
	seed_counter(&db, &h(1), 1); // drift = 1
	put_body(
		&db,
		b"b2",
		vec![BareDbExtrinsic::MultiRenew { hashes: vec![h(2); 5], extrinsic: vec![] }],
	);
	seed_counter(&db, &h(2), 1); // drift = 4
	put_body(
		&db,
		b"b3",
		vec![BareDbExtrinsic::MultiRenew { hashes: vec![h(3); 3], extrinsic: vec![] }],
	);
	seed_counter(&db, &h(3), 1); // drift = 2

	let report = dry_run(&db).unwrap();
	let top = report.top_n_drifted(2);
	assert_eq!(top.len(), 2);
	assert_eq!(top[0], (h(2), 4));
	assert_eq!(top[1], (h(3), 2));
}

// --- entry listing ---------------------------------------------------------------------

/// A block's `BODY_INDEX` key: 4 big-endian number bytes followed by the 32-byte block hash.
fn lookup_key(number: u32, block_hash: &DbHash) -> Vec<u8> {
	let mut key = number.to_be_bytes().to_vec();
	key.extend_from_slice(block_hash.as_ref());
	key
}

/// Store a value under its own content hash, so algorithm detection has something to find.
fn put_value(db: &dyn KeyValueDB, algo: HashAlgo, data: &[u8]) -> DbHash {
	let hash = DbHash::from(algo.hash(data));
	let mut tx = db.transaction();
	tx.put(columns::TRANSACTION, hash.as_ref(), data);
	db.write(tx).unwrap();
	hash
}

/// Seed a block: the `KEY_LOOKUP` row plus the `BODY_INDEX` entry under the canonical key.
fn put_block(db: &dyn KeyValueDB, number: u32, items: Vec<BareDbExtrinsic>) {
	let block_hash = h(0xB0 ^ (number as u8));
	let key = lookup_key(number, &block_hash);
	let mut tx = db.transaction();
	tx.put(columns::KEY_LOOKUP, &number.to_be_bytes(), &key);
	db.write(tx).unwrap();
	put_body(db, &key, items);
}

/// A stand-in for the `Timestamp::set` inherent: bare extrinsic, two call-index bytes, one
/// compact moment and nothing else.
fn timestamp_inherent(ms: u64) -> BareDbExtrinsic {
	let mut bytes = vec![0x04, 0x03, 0x00];
	bytes.extend(codec::Compact(ms).encode());
	BareDbExtrinsic::Full(bytes)
}

const TS: u64 = 1_777_000_000_000; // 2026-04-24 03:06:40 UTC

#[test]
fn list_reports_size_algo_counter_and_creation_time() {
	let db = create(NUM_COLUMNS);
	let data: Vec<u8> = (0..40u8).collect();
	let hash = put_value(&db, HashAlgo::Sha2_256, &data);
	seed_counter(&db, &hash, 2);
	put_block(
		&db,
		7,
		vec![timestamp_inherent(TS), BareDbExtrinsic::Indexed { hash, header: vec![] }],
	);

	let report = list_entries(&db, &ListOptions::default()).unwrap();
	assert_eq!(report.value_entries, 1);
	assert_eq!(report.counter_entries, 1);
	assert_eq!(report.total_bytes, 40);
	assert_eq!(report.matched, 1);
	assert_eq!(report.corrupted, 0);
	assert_eq!(report.orphans, 0);

	let e = &report.entries[0];
	assert_eq!(e.content_hash, hash);
	assert_eq!(e.size, 40);
	assert_eq!(e.algo.unwrap().name(), "sha2_256");
	assert_eq!(e.counter, Some(2));
	assert_eq!(e.referring_blocks, 1);
	assert_eq!(e.first_block, Some(7));
	assert_eq!(e.last_block, Some(7));
	assert_eq!(e.first_block_time_ms, Some(TS));
	// Default preview is one hexdump row.
	assert_eq!(e.preview, data[..16].to_vec());
}

#[test]
fn list_tracks_first_and_last_referring_block() {
	let db = create(NUM_COLUMNS);
	let hash = put_value(&db, HashAlgo::Blake2b256, b"renewed-payload");
	seed_counter(&db, &hash, 2);
	put_block(
		&db,
		10,
		vec![timestamp_inherent(TS), BareDbExtrinsic::Indexed { hash, header: vec![] }],
	);
	put_block(
		&db,
		99,
		vec![
			timestamp_inherent(TS + 534_000),
			BareDbExtrinsic::MultiRenew { hashes: vec![hash], extrinsic: vec![] },
		],
	);

	let report = list_entries(&db, &ListOptions::default()).unwrap();
	let e = &report.entries[0];
	assert_eq!(e.referring_blocks, 2);
	assert_eq!(e.first_block, Some(10));
	assert_eq!(e.first_block_time_ms, Some(TS));
	assert_eq!(e.last_block, Some(99));
	assert_eq!(e.last_block_time_ms, Some(TS + 534_000));
}

#[test]
fn list_block_filter_selects_only_that_blocks_entries() {
	let db = create(NUM_COLUMNS);
	let a = put_value(&db, HashAlgo::Blake2b256, b"stored-in-block-1");
	let b = put_value(&db, HashAlgo::Blake2b256, b"stored-in-block-2");
	put_block(&db, 1, vec![BareDbExtrinsic::Indexed { hash: a, header: vec![] }]);
	put_block(&db, 2, vec![BareDbExtrinsic::Indexed { hash: b, header: vec![] }]);

	let opts = ListOptions { block_filter: Some(2), ..Default::default() };
	let report = list_entries(&db, &opts).unwrap();
	assert_eq!(report.block_found, Some(true));
	assert_eq!(report.matched, 1);
	assert_eq!(report.entries[0].content_hash, b);
	// A filtered listing resolves its entries by point lookup, so it never walks the
	// column and reports no column-wide totals.
	assert!(!report.column_scanned);
	assert_eq!(report.value_entries, 0);
	assert_eq!(report.entries[0].counter, None);
}

#[test]
fn list_hash_filter_uses_point_lookup_and_reads_the_counter() {
	let db = create(NUM_COLUMNS);
	let hash = put_value(&db, HashAlgo::Blake2b256, b"targeted");
	seed_counter(&db, &hash, 4);
	put_value(&db, HashAlgo::Blake2b256, b"some other entry");

	let opts = ListOptions { hash_filter: Some(hash), resolve_blocks: false, ..Default::default() };
	let report = list_entries(&db, &opts).unwrap();
	assert!(!report.column_scanned);
	assert_eq!(report.matched, 1);
	assert_eq!(report.entries[0].content_hash, hash);
	assert_eq!(report.entries[0].counter, Some(4));
}

#[test]
fn list_block_filter_reports_a_missing_block() {
	let db = create(NUM_COLUMNS);
	put_value(&db, HashAlgo::Blake2b256, b"orphaned");

	let opts = ListOptions { block_filter: Some(4242), ..Default::default() };
	let report = list_entries(&db, &opts).unwrap();
	assert_eq!(report.block_found, Some(false));
	assert_eq!(report.matched, 0);
	assert!(report.entries.is_empty());
}

#[test]
fn list_flags_corrupted_and_orphaned_entries() {
	let db = create(NUM_COLUMNS);
	let good = put_value(&db, HashAlgo::Blake2b256, b"hashes-to-its-key");
	put_block(&db, 3, vec![BareDbExtrinsic::Indexed { hash: good, header: vec![] }]);
	// A value parked under a key nothing hashes to, referenced by no alive block.
	let mut tx = db.transaction();
	tx.put(columns::TRANSACTION, h(0xEE).as_ref(), b"not-the-preimage");
	db.write(tx).unwrap();

	let all = list_entries(&db, &ListOptions::default()).unwrap();
	assert_eq!(all.matched, 2);
	assert_eq!(all.corrupted, 1);
	assert_eq!(all.orphans, 1);

	let corrupted =
		list_entries(&db, &ListOptions { corrupted_only: true, ..Default::default() }).unwrap();
	assert_eq!(corrupted.matched, 1);
	assert_eq!(corrupted.entries[0].content_hash, h(0xEE));
	assert!(corrupted.entries[0].algo.is_none());

	let orphans =
		list_entries(&db, &ListOptions { orphans_only: true, ..Default::default() }).unwrap();
	assert_eq!(orphans.matched, 1);
	assert_eq!(orphans.entries[0].content_hash, h(0xEE));
}

#[test]
fn list_sorts_and_limits() {
	let db = create(NUM_COLUMNS);
	let small = put_value(&db, HashAlgo::Blake2b256, b"s");
	let large = put_value(&db, HashAlgo::Blake2b256, &vec![7u8; 512]);

	let opts = ListOptions {
		sort: EntrySort::Size,
		descending: true,
		limit: Some(1),
		resolve_blocks: false,
		..Default::default()
	};
	let report = list_entries(&db, &opts).unwrap();
	assert_eq!(report.matched, 2);
	assert_eq!(report.entries.len(), 1);
	assert_eq!(report.entries[0].content_hash, large);

	let opts = ListOptions { sort: EntrySort::Size, resolve_blocks: false, ..Default::default() };
	let report = list_entries(&db, &opts).unwrap();
	assert_eq!(report.entries[0].content_hash, small);
}

#[test]
fn list_block_range_filters_on_where_an_entry_was_stored() {
	let db = create(NUM_COLUMNS);
	let early = put_value(&db, HashAlgo::Blake2b256, b"stored-early");
	let late = put_value(&db, HashAlgo::Blake2b256, b"stored-late");
	put_block(&db, 10, vec![BareDbExtrinsic::Indexed { hash: early, header: vec![] }]);
	put_block(&db, 500, vec![BareDbExtrinsic::Indexed { hash: late, header: vec![] }]);

	let only_late = ListOptions { from_block: Some(100), ..Default::default() };
	let report = list_entries(&db, &only_late).unwrap();
	assert_eq!(report.matched, 1);
	assert_eq!(report.entries[0].content_hash, late);

	let only_early = ListOptions { to_block: Some(100), ..Default::default() };
	let report = list_entries(&db, &only_early).unwrap();
	assert_eq!(report.matched, 1);
	assert_eq!(report.entries[0].content_hash, early);

	let both = ListOptions { from_block: Some(1), to_block: Some(1000), ..Default::default() };
	assert_eq!(list_entries(&db, &both).unwrap().matched, 2);

	let neither = ListOptions { from_block: Some(11), to_block: Some(499), ..Default::default() };
	assert_eq!(list_entries(&db, &neither).unwrap().matched, 0);
}

#[test]
fn list_block_range_excludes_entries_with_no_known_first_block() {
	let db = create(NUM_COLUMNS);
	// Stored but referenced by no alive block, so there is no "stored at" to compare.
	put_value(&db, HashAlgo::Blake2b256, b"orphan");

	let ranged = ListOptions { from_block: Some(0), ..Default::default() };
	assert_eq!(list_entries(&db, &ranged).unwrap().matched, 0);
	assert_eq!(list_entries(&db, &ListOptions::default()).unwrap().matched, 1);
}

#[test]
fn block_hash_resolves_only_known_blocks() {
	let db = create(NUM_COLUMNS);
	put_block(&db, 42, vec![BareDbExtrinsic::Full(vec![1, 2, 3])]);

	assert_eq!(block_hash(&db, 42).unwrap(), Some(h(0xB0 ^ 42)));
	assert_eq!(block_hash(&db, 43).unwrap(), None);
}

#[test]
fn timestamp_heuristic_rejects_implausible_and_signed_extrinsics() {
	// Signed extrinsic (high bit set) is skipped even if the tail would decode.
	let mut signed = vec![0x84, 0x03, 0x00];
	signed.extend(codec::Compact(TS).encode());
	assert_eq!(block_timestamp_ms(&[signed]), None);

	// A bare extrinsic whose compact is out of range isn't a timestamp.
	let mut small = vec![0x04, 0x03, 0x00];
	small.extend(codec::Compact(42u64).encode());
	assert_eq!(block_timestamp_ms(&[small]), None);

	// Trailing bytes after the compact mean it's some other call.
	let mut trailing = vec![0x04, 0x03, 0x00];
	trailing.extend(codec::Compact(TS).encode());
	trailing.push(0xFF);
	assert_eq!(block_timestamp_ms(&[trailing]), None);

	let mut good = vec![0x04, 0x03, 0x00];
	good.extend(codec::Compact(TS).encode());
	assert_eq!(block_timestamp_ms(&[good]), Some(TS));
}

#[test]
fn timestamp_formatting_matches_utc() {
	assert_eq!(format_timestamp_ms(0), "1970-01-01 00:00:00 UTC");
	assert_eq!(format_timestamp_ms(TS), "2026-04-24 03:06:40 UTC");
	// Leap day.
	assert_eq!(format_timestamp_ms(1_709_164_800_000), "2024-02-29 00:00:00 UTC");
}

#[test]
fn corrupted_listing_names_the_blocks_left_holding_a_bad_entry() {
	let db = create(NUM_COLUMNS);
	// A value parked under a key nothing hashes to, referenced from two blocks.
	let bogus = h(0xEE);
	let mut tx = db.transaction();
	tx.put(columns::TRANSACTION, bogus.as_ref(), b"not-the-preimage");
	db.write(tx).unwrap();
	put_block(&db, 3, vec![BareDbExtrinsic::Indexed { hash: bogus, header: vec![] }]);
	put_block(&db, 4, vec![BareDbExtrinsic::MultiRenew { hashes: vec![bogus], extrinsic: vec![] }]);

	let report =
		list_entries(&db, &ListOptions { corrupted_only: true, ..Default::default() }).unwrap();
	assert_eq!(report.matched, 1);
	assert_eq!(report.corrupted, 1);
	assert_eq!(report.unexpected_key_rows, 0);

	let entry = &report.entries[0];
	assert!(entry.algo.is_none());
	// All three algorithms are kept as evidence once nothing matched...
	assert_eq!(entry.computed_hashes.len(), 3);
	// ...along with the full referrer list, which a healthy entry does not carry.
	assert_eq!(entry.referring_block_list, vec![3, 4]);

	let rendered = report.to_string();
	assert!(rendered.contains("referring blocks (2): #3, #4"), "{rendered}");
	assert!(rendered.contains("blake2b256(value) = 0x"), "{rendered}");
	assert!(rendered.contains("integrity CORRUPTED"), "{rendered}");
}

#[test]
fn healthy_entries_do_not_carry_a_referrer_list() {
	let db = create(NUM_COLUMNS);
	let good = put_value(&db, HashAlgo::Blake2b256, b"hashes-to-its-key");
	put_block(&db, 7, vec![BareDbExtrinsic::Indexed { hash: good, header: vec![] }]);

	let report = list_entries(&db, &ListOptions::default()).unwrap();
	let entry = &report.entries[0];
	assert_eq!(entry.referring_blocks, 1);
	assert!(entry.referring_block_list.is_empty());
	assert!(entry.computed_hashes.is_empty());
}

#[test]
fn unexpected_key_shapes_are_counted() {
	let db = create(NUM_COLUMNS);
	put_value(&db, HashAlgo::Blake2b256, b"fine");
	let mut tx = db.transaction();
	tx.put(columns::TRANSACTION, b"short-key", b"who-wrote-this");
	db.write(tx).unwrap();

	let report = list_entries(&db, &ListOptions::default()).unwrap();
	assert_eq!(report.value_entries, 1);
	assert_eq!(report.unexpected_key_rows, 1);
	assert!(report.to_string().contains("unrecognised key shape"));
}

#[test]
fn realign_finds_a_length_preserving_window_shift() {
	// `find_alignment` is the one search ladder both realign paths run; exercise the diagonal
	// case directly: the correct payload sits shifted two bytes back from the current split.
	let payload = b"the-actual-stored-payload".to_vec();
	let want = DbHash::from(HashAlgo::Blake2b256.hash(&payload));
	let mut full = b"HD".to_vec(); // two header bytes that belong to the payload's left
	full.extend_from_slice(&payload);
	full.extend_from_slice(b"XY"); // two bytes that don't belong to it
	let header_size = 4; // the wrong split point: 2 real header bytes + 2 stolen ones

	let found = find_alignment(&full, header_size, full.len() - header_size, want, 16)
		.expect("the diagonal shift should match");
	assert_eq!(&full[found.start..found.end], &payload[..]);
	assert_eq!(found.algo.name(), "blake2b256");
}

#[test]
fn column_integrity_counts_survive_the_corrupted_only_filter() {
	let db = create(NUM_COLUMNS);
	put_value(&db, HashAlgo::Blake2b256, b"healthy-one");
	put_value(&db, HashAlgo::Blake2b256, b"healthy-two");
	let mut tx = db.transaction();
	tx.put(columns::TRANSACTION, h(0xEE).as_ref(), b"not-the-preimage");
	db.write(tx).unwrap();

	// The filter drops the healthy entries, but the column-wide tally still describes the
	// whole scan — otherwise a clean check reads as "0 of 0 verified".
	let report =
		list_entries(&db, &ListOptions { corrupted_only: true, ..Default::default() }).unwrap();
	assert_eq!(report.matched, 1);
	assert_eq!(report.values_verified, 2);
	assert_eq!(report.values_corrupted, 1);
	assert!(report.to_string().contains("Integrity:             2 verified, 1 corrupted"));

	// A clean column reports the work done, not zeroes.
	let db = create(NUM_COLUMNS);
	put_value(&db, HashAlgo::Blake2b256, b"healthy-one");
	let report =
		list_entries(&db, &ListOptions { corrupted_only: true, ..Default::default() }).unwrap();
	assert_eq!(report.matched, 0);
	assert_eq!(report.values_verified, 1);
	assert_eq!(report.values_corrupted, 0);
	assert!(report.to_string().contains("1 verified, 0 corrupted"));
}

#[test]
fn refcount_backfill_sets_each_counter_to_its_true_reference_count() {
	let db = create(NUM_COLUMNS);
	// Two blocks each renewing the same hash three times: 6 references in total, but the
	// pre-aggregation commit path collapsed each block to a single +1, so the counter reads 2.
	let hash = h(9);
	let batch =
		|times| BareDbExtrinsic::MultiRenew { hashes: vec![hash; times], extrinsic: vec![] };
	put_block(&db, 1, vec![batch(3)]);
	put_block(&db, 2, vec![batch(3)]);
	seed_counter(&db, &hash, 2);

	let planned = repair_refcounts(&db, false).unwrap();
	assert_eq!(planned.rows.len(), 1);
	assert!(!planned.applied);
	assert_eq!(planned.rows[0].content_hash, hash);
	assert_eq!(planned.rows[0].counter_before, 2);
	assert_eq!(planned.rows[0].counter_after, 6);
	assert_eq!(planned.units(), 4);
	assert!(planned.rows[0].at_risk, "two blocks reference it, so the shortfall can delete data");
	assert!(!planned.rows[0].applied);
	// A dry run must leave the database alone.
	assert_eq!(read_counter(&db, &hash).unwrap(), Some(2));

	let done = repair_refcounts(&db, true).unwrap();
	assert!(done.applied);
	assert!(done.rows[0].applied);
	assert_eq!(read_counter(&db, &hash).unwrap(), Some(6));

	// The drift it was based on is now gone, so there is nothing left to do.
	assert!(dry_run(&db).unwrap().on_disk_drift.is_empty());
	let again = repair_refcounts(&db, false).unwrap();
	assert!(again.rows.is_empty());
	assert!(again.to_string().contains("nothing to backfill"));
}

#[test]
fn refcount_backfill_leaves_values_untouched() {
	let db = create(NUM_COLUMNS);
	let data = b"payload-that-must-not-move".to_vec();
	let hash = put_value(&db, HashAlgo::Blake2b256, &data);
	put_block(
		&db,
		1,
		vec![BareDbExtrinsic::MultiRenew { hashes: vec![hash; 4], extrinsic: vec![] }],
	);
	put_block(&db, 2, vec![BareDbExtrinsic::Indexed { hash, header: vec![] }]);
	seed_counter(&db, &hash, 2);

	repair_refcounts(&db, true).unwrap();
	assert_eq!(read_counter(&db, &hash).unwrap(), Some(5));
	// Only the counter row was written.
	assert_eq!(db.get(columns::TRANSACTION, hash.as_ref()).unwrap(), Some(data));
}

#[test]
fn diff_reports_entries_only_one_database_has() {
	let a = create(NUM_COLUMNS);
	let b = create(NUM_COLUMNS);

	let shared = put_value(&a, HashAlgo::Blake2b256, b"in-both");
	put_value(&b, HashAlgo::Blake2b256, b"in-both");
	seed_counter(&a, &shared, 3);
	seed_counter(&b, &shared, 3);

	let only_a = put_value(&a, HashAlgo::Blake2b256, b"only-in-a");
	let only_b = put_value(&b, HashAlgo::Blake2b256, b"only-in-b");

	let report = diff_databases(&a, &b, &DiffOptions::default()).unwrap();
	assert_eq!(report.entries_a, 2);
	assert_eq!(report.entries_b, 2);
	assert_eq!(report.differing, 2);
	assert_eq!(report.only_in_a, 1);
	assert_eq!(report.only_in_b, 1);
	assert_eq!(report.refcount_differs, 0);
	assert!(!report.is_identical());

	let hashes: Vec<DbHash> = report.rows.iter().map(|r| r.content_hash).collect();
	assert!(hashes.contains(&only_a) && hashes.contains(&only_b));
	assert!(!hashes.contains(&shared), "an entry both agree about is not a difference");

	let rendered = report.to_string();
	assert!(rendered.contains("only in A"), "{rendered}");
	assert!(rendered.contains("only in B"), "{rendered}");
}

#[test]
fn diff_spots_a_refcount_that_one_side_never_backfilled() {
	let a = create(NUM_COLUMNS);
	let b = create(NUM_COLUMNS);
	let hash = put_value(&a, HashAlgo::Blake2b256, b"same-bytes-both-sides");
	put_value(&b, HashAlgo::Blake2b256, b"same-bytes-both-sides");
	seed_counter(&a, &hash, 4500); // backfilled
	seed_counter(&b, &hash, 10); // still collapsed

	let report = diff_databases(&a, &b, &DiffOptions::default()).unwrap();
	assert_eq!(report.differing, 1);
	assert_eq!(report.refcount_differs, 1);
	assert_eq!(report.size_differs, 0);
	assert_eq!(report.integrity_differs, 0);
	assert!(report.to_string().contains("refcount     A 4500   B 10"));
}

#[test]
fn diff_spots_a_value_corrupted_on_one_side_only() {
	let a = create(NUM_COLUMNS);
	let b = create(NUM_COLUMNS);
	let hash = put_value(&a, HashAlgo::Blake2b256, b"the-real-payload");
	// Same key in B, but holding bytes that don't hash to it.
	let mut tx = b.transaction();
	tx.put(columns::TRANSACTION, hash.as_ref(), b"not-the-payload!");
	b.write(tx).unwrap();

	let report = diff_databases(&a, &b, &DiffOptions::default()).unwrap();
	assert_eq!(report.integrity_differs, 1);
	assert_eq!(report.only_in_a, 0, "both sides have the key");
	let row = &report.rows[0];
	assert!(row.a.unwrap().verified);
	assert!(!row.b.unwrap().verified);
	assert!(report.to_string().contains("integrity    A ok   B CORRUPTED"));
}

#[test]
fn diff_names_the_blocks_whose_indexed_body_is_missing() {
	let a = create(NUM_COLUMNS);
	let b = create(NUM_COLUMNS);
	let hash = put_value(&a, HashAlgo::Blake2b256, b"stored");
	put_value(&b, HashAlgo::Blake2b256, b"stored");
	let body = |h| vec![BareDbExtrinsic::Indexed { hash: h, header: vec![] }];
	put_block(&a, 10, body(hash));
	put_block(&a, 11, body(hash));
	put_block(&b, 10, body(hash));
	// B never indexed #11 — the shape that leaves a collator unable to prove it.

	let opts = DiffOptions { blocks: true, ..Default::default() };
	let report = diff_databases(&a, &b, &opts).unwrap();
	let blocks = report.blocks.as_ref().unwrap();
	assert_eq!(blocks.bodies_a, 2);
	assert_eq!(blocks.bodies_b, 1);
	assert_eq!(blocks.only_in_a, vec![11]);
	assert!(blocks.only_in_b.is_empty());
	assert!(blocks.refs_differ.is_empty());
	assert!(!report.is_identical());
	assert!(report.to_string().contains("only in A (1): #11"));
}

#[test]
fn diff_of_a_database_with_itself_is_identical() {
	let db = create(NUM_COLUMNS);
	let hash = put_value(&db, HashAlgo::Blake2b256, b"whatever");
	seed_counter(&db, &hash, 1);
	put_block(&db, 5, vec![BareDbExtrinsic::Indexed { hash, header: vec![] }]);

	let opts = DiffOptions { blocks: true, ..Default::default() };
	let report = diff_databases(&db, &db, &opts).unwrap();
	assert_eq!(report.differing, 0);
	assert!(report.is_identical());
	assert!(report.to_string().contains("IDENTICAL"));
}

#[test]
fn diff_summary_counts_survive_the_limit() {
	let a = create(NUM_COLUMNS);
	let b = create(NUM_COLUMNS);
	for i in 0..5u8 {
		put_value(&a, HashAlgo::Blake2b256, &[i; 64]);
	}
	put_value(&b, HashAlgo::Blake2b256, &[0u8; 64]);

	// Four entries differ; only one line may be printed.
	let opts = DiffOptions { limit: Some(1), ..Default::default() };
	let report = diff_databases(&a, &b, &opts).unwrap();
	assert_eq!(report.rows.len(), 1, "the printed rows are capped");
	assert_eq!(report.differing, 4, "the totals are not");
	assert_eq!(report.only_in_a, 4);
	assert_eq!(report.only_in_b, 0);
	assert!(report.to_string().contains("only in A:           4"));
	assert!(report.to_string().contains("3 more differing entries cut by --limit"));
}

/// Payload bytes that do not repeat with period 106, so a shifted window is distinguishable.
/// Uniform data (e.g. `vec![7; n]`) cannot be told apart after a 106-byte shift.
fn payload(len: usize) -> Vec<u8> {
	(0..len).map(|i| (i.wrapping_mul(7).wrapping_add(13) % 251) as u8).collect()
}

/// Build the three seam states from one authored extrinsic.
///
/// The authored bytes are `H' ++ compact(len) ++ D ++ T`, i.e. the pre-#574 `promote` shape
/// where the 106-byte `(MultiSigner, MultiSignature, u64)` tuple follows the data.
fn hop_extrinsic(data: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
	let preamble = vec![0x45u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x29, 0x00];
	let mut header = preamble;
	header.extend(codec::Compact(data.len() as u32).encode());
	let tail = vec![0xAAu8; 106];
	(header, data.to_vec(), tail)
}

#[test]
fn seam_classifies_a_healthy_pair() {
	// Data as the last field: header ++ value is exactly what was authored.
	let data = payload(500);
	let (header, value, _) = hop_extrinsic(&data);
	let hash = DbHash::from(HashAlgo::Blake2b256.hash(&value));
	let (state, hashes, window, trailing) = classify(&header, &value, hash);
	assert_eq!(state, SeamState::Healthy);
	assert!(hashes && window);
	assert_eq!(trailing, Some(0), "nothing follows the data");
}

#[test]
fn seam_classifies_the_original_mis_split() {
	// What #574 produced: the indexed window is the trailing `data.len()` bytes, so it holds
	// `D[106..] ++ T` and does not hash to the key — but the pair still reassembles correctly.
	let data = payload(500);
	let (mut header, data, tail) = hop_extrinsic(&data);
	let authored: Vec<u8> = header.iter().chain(&data).chain(&tail).copied().collect();
	let value = authored[authored.len() - data.len()..].to_vec();
	header = authored[..authored.len() - data.len()].to_vec();
	let hash = DbHash::from(HashAlgo::Blake2b256.hash(&data)); // key is hash(D), not hash(value)

	let (state, hashes, window, _) = classify(&header, &value, hash);
	assert_eq!(state, SeamState::OriginalMisaligned);
	assert!(!hashes, "the stored value is the wrong window");
	assert!(!window);
	assert!(state.body_executable(), "the block still replays");
	// The pair reassembles to the authored bytes, so the body stays executable.
	let reassembled: Vec<u8> = header.iter().chain(&value).copied().collect();
	assert_eq!(reassembled, authored);
}

#[test]
fn seam_catches_a_col11_only_repair() {
	// What the old repair logic left behind: value fixed to D, header untouched.
	let data = payload(500);
	let (header, data, tail) = hop_extrinsic(&data);
	let authored: Vec<u8> = header.iter().chain(&data).chain(&tail).copied().collect();
	let stale_header = authored[..authored.len() - data.len()].to_vec();
	let hash = DbHash::from(HashAlgo::Blake2b256.hash(&data));

	let (state, hashes, window, _) = classify(&stale_header, &data, hash);
	assert_eq!(state, SeamState::HalfRepaired);
	assert!(hashes, "the value hashes to its key, so an integrity scan reports nothing");
	assert!(!window, "but the declared data window does not hold the value");
	assert!(!state.body_executable());
	// Same length as the authored extrinsic, so the node's OpaqueExtrinsic decode passes and
	// only the runtime notices.
	let reassembled: Vec<u8> = stale_header.iter().chain(&data).copied().collect();
	assert_eq!(reassembled.len(), authored.len());
	assert_ne!(reassembled, authored);
}

#[test]
fn seam_verification_walks_a_database() {
	let db = create(NUM_COLUMNS);
	// One healthy entry.
	let good_data = payload(300);
	let good = put_value(&db, HashAlgo::Blake2b256, &good_data);
	let (good_header, _, _) = hop_extrinsic(&good_data);
	put_block(&db, 10, vec![BareDbExtrinsic::Indexed { hash: good, header: good_header }]);

	// One entry a col11-only repair left unexecutable.
	let data = payload(400);
	let (header, data, tail) = hop_extrinsic(&data);
	let authored: Vec<u8> = header.iter().chain(&data).chain(&tail).copied().collect();
	let stale_header = authored[..authored.len() - data.len()].to_vec();
	let repaired = put_value(&db, HashAlgo::Blake2b256, &data);
	put_block(&db, 11, vec![BareDbExtrinsic::Indexed { hash: repaired, header: stale_header }]);

	let report = verify_seams(&db).unwrap();
	assert_eq!(report.examined, 2);
	assert_eq!(report.healthy, 1);
	assert_eq!(report.half_repaired, 1);
	assert_eq!(report.original_misaligned, 0);
	assert!(!report.is_clean());
	assert_eq!(report.unexecutable_blocks(), vec![11]);

	let rendered = report.to_string();
	assert!(rendered.contains("col11-only repair"), "{rendered}");
	assert!(rendered.contains("cannot be executed by any runtime"), "{rendered}");
}

#[test]
fn seam_recognises_a_single_renewal() {
	// sc-client-db stores a single renewal as `Indexed { hash, header: <whole extrinsic> }`, so
	// the header is itself length-consistent and the value belongs to an earlier block. That is
	// not a split and must not be reported as damage.
	let renewed = payload(605);
	let hash = DbHash::from(HashAlgo::Blake2b256.hash(&renewed));
	let call = vec![0x84u8; 112]; // a complete renew extrinsic, whatever its contents
	let mut header = codec::Compact(call.len() as u32).encode();
	header.extend(&call);

	let (state, hashes, _, _) = classify(&header, &renewed, hash);
	assert_eq!(state, SeamState::SingleRenewal);
	assert!(hashes, "the renewed value still hashes to its key");
	assert!(state.body_executable());
}

#[test]
fn seam_report_treats_renewals_as_clean() {
	let db = create(NUM_COLUMNS);
	let renewed = payload(400);
	let hash = put_value(&db, HashAlgo::Blake2b256, &renewed);
	let call = vec![0x84u8; 112];
	let mut renew_header = codec::Compact(call.len() as u32).encode();
	renew_header.extend(&call);
	put_block(&db, 20, vec![BareDbExtrinsic::Indexed { hash, header: renew_header }]);

	let report = verify_seams(&db).unwrap();
	assert_eq!(report.examined, 1);
	assert_eq!(report.single_renewals, 1);
	assert_eq!(report.half_repaired, 0, "a renewal is not a col11-only repair");
	assert!(report.is_clean());
	assert!(report.unexecutable_blocks().is_empty());
}

// --- storage proof ---------------------------------------------------------------------

/// A proof recomputed from the bytes on disk always verifies against its own root, so
/// `verified` alone cannot tell whether those bytes are the ones the chain committed to.
/// `--expect-root` is what closes that gap: feed the proof its own root and it agrees, feed it
/// any other and it reports a mismatch.
#[test]
fn expect_root_distinguishes_agreement_from_a_stale_value() {
	let db = create(NUM_COLUMNS);
	// Two chunks' worth, so chunk selection has something to choose between.
	let data = payload(600);
	let hash = put_value(&db, HashAlgo::Blake2b256, &data);
	put_block(&db, 7, vec![BareDbExtrinsic::Indexed { hash, header: vec![0x45, 0x00] }]);

	let random = [0x11u8; 32];
	let unchecked = compute_storage_proof(&db, 7, random, None).unwrap().unwrap();
	assert!(
		unchecked.verified,
		"a proof built from the on-disk bytes verifies against its own root"
	);
	assert_eq!(unchecked.agrees_with_chain(), None, "nothing to compare against");
	assert!(unchecked.is_good());

	// The root the chain would have recorded for an unmodified value is the one we just derived.
	let matching = compute_storage_proof(&db, 7, random, Some(unchecked.tx_chunk_root))
		.unwrap()
		.unwrap();
	assert_eq!(matching.agrees_with_chain(), Some(true));
	assert!(matching.is_good());
	assert!(matching.to_string().contains("chain agreement:          OK"));

	// Any other root means the bytes on disk are not the stored ones — the proof still verifies
	// locally, but the runtime would reject it.
	let stale = compute_storage_proof(&db, 7, random, Some(h(0xEE))).unwrap().unwrap();
	assert!(stale.verified);
	assert_eq!(stale.agrees_with_chain(), Some(false));
	assert!(!stale.is_good(), "a chain mismatch is not a clean bill of health");
	assert!(stale.to_string().contains("chain agreement:          MISMATCH"));
}

// --- reference trace ---------------------------------------------------------------------

/// Three blocks each referencing the value once, and a counter that agrees.
#[test]
fn trace_builds_the_ledger_over_referring_blocks() {
	let db = create(NUM_COLUMNS);
	let data = payload(300);
	let hash = put_value(&db, HashAlgo::Blake2b256, &data);
	seed_counter(&db, &hash, 3);
	for n in [10u32, 20, 30] {
		put_block(&db, n, vec![BareDbExtrinsic::Indexed { hash, header: vec![0x45] }]);
	}
	// A block that references something else must not appear in the ledger.
	put_block(&db, 40, vec![BareDbExtrinsic::Indexed { hash: h(9), header: vec![] }]);

	let report = trace_hash(&db, hash).unwrap();
	assert_eq!(report.value_size, Some(300));
	assert_eq!(report.algo, Some(HashAlgo::Blake2b256));
	assert_eq!(report.referring_blocks(), vec![10, 20, 30]);
	assert_eq!(report.alive_total, 3);
	assert_eq!(report.verdict(), Verdict::Consistent { total: 3 });
	// The rendered ledger carries the running total, not just the per-block delta.
	let text = report.to_string();
	assert!(text.contains("Alive references:      3"));
	assert!(text.contains("CONSISTENT"));
}

/// One `MultiRenew` naming the hash twice contributes two references, not one — the
/// distinction the polkadot-sdk#12106 collapse got wrong.
#[test]
fn trace_counts_occurrences_not_blocks() {
	let db = create(NUM_COLUMNS);
	let data = payload(64);
	let hash = put_value(&db, HashAlgo::Blake2b256, &data);
	seed_counter(&db, &hash, 2);
	put_block(
		&db,
		7,
		vec![BareDbExtrinsic::MultiRenew { hashes: vec![hash, h(4), hash], extrinsic: vec![] }],
	);

	let report = trace_hash(&db, hash).unwrap();
	assert_eq!(report.referring_blocks(), vec![7]);
	assert_eq!(report.alive_total, 2, "one block, two occurrences");
	assert_eq!(report.verdict(), Verdict::Consistent { total: 2 });
}

#[test]
fn trace_flags_a_short_counter() {
	let db = create(NUM_COLUMNS);
	let data = payload(64);
	let hash = put_value(&db, HashAlgo::Blake2b256, &data);
	seed_counter(&db, &hash, 1); // collapsed: one per block where it should be one per ref
	put_block(
		&db,
		7,
		vec![
			BareDbExtrinsic::Indexed { hash, header: vec![] },
			BareDbExtrinsic::Indexed { hash, header: vec![] },
		],
	);

	let report = trace_hash(&db, hash).unwrap();
	assert_eq!(report.verdict(), Verdict::Short { expected: 2, actual: 1 });
	assert!(report.verdict().is_finding());
	assert!(report.to_string().contains("COUNTER SHORT"));
}

#[test]
fn trace_flags_an_excess_counter() {
	let db = create(NUM_COLUMNS);
	let data = payload(64);
	let hash = put_value(&db, HashAlgo::Blake2b256, &data);
	seed_counter(&db, &hash, 5);
	put_block(&db, 7, vec![BareDbExtrinsic::Indexed { hash, header: vec![] }]);

	let report = trace_hash(&db, hash).unwrap();
	assert_eq!(report.verdict(), Verdict::Excess { expected: 1, actual: 5 });
	assert!(report.to_string().contains("COUNTER EXCESS"));
}

/// Alive blocks reference a hash whose value and counter are both gone, which a renewal
/// cannot repair.
#[test]
fn trace_flags_a_dangling_reference() {
	let db = create(NUM_COLUMNS);
	let hash = h(0xAB);
	put_block(&db, 189038, vec![BareDbExtrinsic::Indexed { hash, header: vec![0x45] }]);

	let report = trace_hash(&db, hash).unwrap();
	assert_eq!(report.value_size, None);
	assert_eq!(report.counter, None);
	assert_eq!(report.verdict(), Verdict::Dangling { referring_blocks: 1 });
	let text = report.to_string();
	assert!(text.contains("DANGLING"));
	assert!(text.contains("cannot repair this"), "must say renewals will not fix it");
}

#[test]
fn trace_reports_absent_when_nothing_knows_the_hash() {
	let db = create(NUM_COLUMNS);
	let report = trace_hash(&db, h(0x11)).unwrap();
	assert_eq!(report.verdict(), Verdict::Absent);
	assert!(report.rows.is_empty());
	assert!(report.to_string().contains("No block references this hash"));
}

/// Chain rows for blocks this database has no reference for become their own ledger lines,
/// labelled as releases; blocks the chain has no entry for are flagged the other way.
#[test]
fn merge_chain_adds_released_and_spurious_rows() {
	use crate::chain::{BlockFacts, ChainFacts};

	let db = create(NUM_COLUMNS);
	let data = payload(64);
	let hash = put_value(&db, HashAlgo::Blake2b256, &data);
	seed_counter(&db, &hash, 1);
	put_block(&db, 200, vec![BareDbExtrinsic::Indexed { hash, header: vec![] }]);
	let mut report = trace_hash(&db, hash).unwrap();

	let mut per_block = std::collections::BTreeMap::new();
	// The chain recorded a renewal at #100 that this database no longer references.
	per_block
		.insert(100u32, BlockFacts { state_readable: true, has_entry: true, ..Default::default() });
	// And says nothing happened at #200, which this database does reference.
	per_block.insert(
		200u32,
		BlockFacts { state_readable: true, has_entry: false, ..Default::default() },
	);
	merge_chain(
		&mut report,
		ChainFacts {
			url: "http://x".into(),
			head: 300,
			finalized: 299,
			retention_period: Some(100),
			latest_location: Some((100, 0)),
			per_block,
			unresolved: Vec::new(),
		},
	);

	assert_eq!(report.rows.len(), 2, "the chain-only block became its own row");
	assert_eq!(report.released(), vec![100]);
	assert_eq!(report.spurious(), vec![200]);
	// Merging must not change what the database itself carries.
	assert_eq!(report.alive_total, 1);
	let text = report.to_string();
	assert!(text.contains("reference released here"));
	assert!(text.contains("chain has no record of this"));
	assert!(text.contains("next proof due at #200"), "RP + location = when the proof is due");
}

/// Exercises the subxt layer against a real node. Ignored by default; nothing else proves the
/// storage paths and metadata lookups still resolve against a live runtime. Needs
/// `TX_INDEX_TOOL_TEST_RPC`, `TX_INDEX_TOOL_TEST_HASH` and `TX_INDEX_TOOL_TEST_BLOCK`.
#[test]
#[ignore]
fn chain_fetch_reaches_a_live_node() {
	let Ok(url) = std::env::var("TX_INDEX_TOOL_TEST_RPC") else {
		panic!("set TX_INDEX_TOOL_TEST_RPC to a ws:// or wss:// endpoint");
	};
	let hash: DbHash = std::env::var("TX_INDEX_TOOL_TEST_HASH")
		.expect("set TX_INDEX_TOOL_TEST_HASH to a stored content hash")
		.parse()
		.expect("valid 32-byte hash");
	let block: u32 = std::env::var("TX_INDEX_TOOL_TEST_BLOCK")
		.expect("set TX_INDEX_TOOL_TEST_BLOCK to a block that references it")
		.parse()
		.expect("valid block number");

	let facts = crate::chain::fetch(&url, hash, &[block], false, 16).expect("chain fetch");
	println!("head #{}, finalized #{}", facts.head, facts.finalized);
	println!("retention period: {:?}", facts.retention_period);
	println!("latest location:  {:?}", facts.latest_location);
	for (n, bf) in &facts.per_block {
		println!("  #{n}: {}", bf.summary());
		println!("      chunk_root {:?}", bf.chunk_root);
	}
	assert!(facts.head > 0, "node reported no best block");
	assert!(facts.retention_period.is_some(), "RetentionPeriod did not decode");

	// The cadence walk finds renewals the database no longer references.
	let probed = crate::chain::fetch(&url, hash, &[block], true, 12).expect("cadence probe");
	println!("cadence probe touched {} height(s)", probed.per_block.len());
	for (n, bf) in &probed.per_block {
		println!("  #{n}: {}", bf.summary());
	}
	assert!(probed.per_block.len() > 1, "probing should reach beyond the anchor block");
}
