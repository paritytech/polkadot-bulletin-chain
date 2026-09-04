// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Shared types and helpers: on-disk layout, hashing, formatting, and how the database is
//! opened.

use codec::{Decode, Encode};
use kvdb::KeyValueDB;
use kvdb_rocksdb::{Database, DatabaseConfig};
use std::{fmt::Write as _, path::PathBuf};

/// Block hash / content hash, matching `sc_client_db::DbHash`.
pub type DbHash = sp_core::H256;

/// Column indices, mirroring the private `sc_client_db::columns`. They are part of the on-disk
/// format, so they only change with a database version bump.
pub mod columns {
	/// Chain metadata, including the best-block pointer.
	pub const META: u32 = 0;
	/// Maps block numbers to lookup keys.
	pub const KEY_LOOKUP: u32 = 3;
	/// Indexed transaction data: `hash` -> value, `hash||0x00` -> LE u32 refcount.
	pub const TRANSACTION: u32 = 11;
	/// Per-block `Vec<DbExtrinsic>` describing which transactions a body indexes.
	pub const BODY_INDEX: u32 = 12;
}

/// Key holding the best-block lookup key in `columns::META`.
pub const BEST_BLOCK_KEY: &[u8; 4] = b"best";

/// Column count a substrate full node's database is created with.
pub const NUM_COLUMNS: u32 = 13;

/// Schema version this tool's column layout was written against, as substrate records it in
/// the `db_version` file beside the data.
pub const SUPPORTED_DB_VERSION: &str = "4";

/// Mirror of the private `DbExtrinsic<B>` whose `Full` variant carries opaque bytes instead
/// of `B::Extrinsic`. SCALE-decoding is bytewise identical because the standard substrate
/// extrinsic encoding is a compact-length-prefixed byte blob — exactly what `Vec<u8>`
/// decodes as. Keeps the scanner runtime-agnostic.
#[derive(Encode, Decode)]
pub enum BareDbExtrinsic {
	Indexed { hash: DbHash, header: Vec<u8> },
	Full(Vec<u8>),
	MultiRenew { hashes: Vec<DbHash>, extrinsic: Vec<u8> },
}

/// Per-hash occurrence breakdown within a single block, separating standalone `Indexed`
/// entries from per-`MultiRenew` inner counts. `Indexed` is used by sc-client-db for BOTH
/// initial stores *and* single renewals — they're byte-indistinguishable in `BODY_INDEX`
/// without decoding the runtime-specific extrinsic call.
#[derive(Debug, Clone, Default)]
pub struct Occurrences {
	pub indexed: u32,
	/// One entry per `MultiRenew` that referenced this hash; the value is how many times
	/// the hash appeared in *that* MultiRenew's `hashes` vec.
	pub multirenew_inner: Vec<u32>,
}

impl Occurrences {
	/// Total references this shape contributes — one per occurrence, not one per extrinsic,
	/// which is what the refcount must agree with.
	pub fn total(&self) -> u32 {
		self.indexed + self.multirenew_inner.iter().sum::<u32>()
	}
}

pub fn fmt_occurrences(occ: &Occurrences) -> String {
	let mr_extrinsics = occ.multirenew_inner.len();
	let mr_total: u32 = occ.multirenew_inner.iter().sum();
	match (occ.indexed, mr_extrinsics) {
		(1, 0) => "Indexed".to_string(),
		(n, 0) => format!("{n}×Indexed"),
		(0, 1) => format!("MultiRenew(×{mr_total})"),
		(0, m) => format!("{m}×MultiRenew(total ×{mr_total})"),
		(i, 1) => format!("{i}×Indexed + MultiRenew(×{mr_total})"),
		(i, m) => format!("{i}×Indexed + {m}×MultiRenew(total ×{mr_total})"),
	}
}

/// Hash algorithm options matching bulletin's `HashingAlgorithm` enum (Blake2b256 is the
/// default `store(data)` path; the others come from `store_with_cid_config`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgo {
	/// Blake2b-256 — the default `store(data)` path.
	Blake2b256,
	/// SHA2-256, selected via `store_with_cid_config`.
	Sha2_256,
	/// Keccak-256, selected via `store_with_cid_config`.
	Keccak256,
}

impl HashAlgo {
	/// Every algorithm a stored value may have been hashed under, in the order they are tried.
	/// Blake2b256 is first because it is the default `store(data)` path.
	pub const ALL: [HashAlgo; 3] = [HashAlgo::Blake2b256, HashAlgo::Sha2_256, HashAlgo::Keccak256];

	/// The integrity check: which algorithm reproduces `key` from `value`, if any. Stops at the
	/// first match, so a healthy entry costs one hash rather than three.
	pub fn identify(key: DbHash, value: &[u8]) -> Option<HashAlgo> {
		Self::ALL.into_iter().find(|a| DbHash::from(a.hash(value)) == key)
	}

	/// What `value` hashes to under every algorithm — the evidence to print once `identify`
	/// has come back empty.
	pub fn hash_all(value: &[u8]) -> Vec<(HashAlgo, DbHash)> {
		Self::ALL.into_iter().map(|a| (a, DbHash::from(a.hash(value)))).collect()
	}

	/// Hash `data` under this algorithm.
	pub fn hash(&self, data: &[u8]) -> [u8; 32] {
		match self {
			HashAlgo::Blake2b256 => sp_crypto_hashing::blake2_256(data),
			HashAlgo::Sha2_256 => sp_crypto_hashing::sha2_256(data),
			HashAlgo::Keccak256 => sp_crypto_hashing::keccak_256(data),
		}
	}

	/// Canonical lowercase name, as printed in reports.
	pub fn name(&self) -> &'static str {
		match self {
			HashAlgo::Blake2b256 => "blake2b256",
			HashAlgo::Sha2_256 => "sha2_256",
			HashAlgo::Keccak256 => "keccak256",
		}
	}

	/// Parse an algorithm from its CLI spelling.
	pub fn parse(s: &str) -> Result<Self, String> {
		match s.to_lowercase().as_str() {
			"blake2b256" | "blake2_256" | "blake2b" | "blake2" => Ok(HashAlgo::Blake2b256),
			"sha2_256" | "sha2-256" | "sha256" => Ok(HashAlgo::Sha2_256),
			"keccak256" | "keccak_256" | "keccak-256" | "keccak" => Ok(HashAlgo::Keccak256),
			other => Err(format!("unknown hash algorithm: {other}")),
		}
	}
}

/// Parse a 32-byte hash, with or without the `0x` prefix.
pub fn parse_hex32(s: &str) -> Result<DbHash, String> {
	s.parse::<DbHash>().map_err(|e| format!("invalid 32-byte hash {s}: {e}"))
}

/// `0x`-prefixed lowercase hex. `DbHash` renders itself this way through `{:#x}`; this is for
/// the byte slices that aren't hashes.
pub fn hex(bytes: &[u8]) -> String {
	let mut out = String::with_capacity(2 + bytes.len() * 2);
	out.push_str("0x");
	for b in bytes {
		let _ = write!(out, "{b:02x}");
	}
	out
}

/// Render up to `max` items, comma-separated, noting how many were left out.
pub fn joined<T>(items: &[T], max: usize, render: impl Fn(&T) -> String) -> String {
	let mut out = items.iter().take(max).map(render).collect::<Vec<_>>().join(", ");
	if items.len() > max {
		let _ = write!(out, ", … (+{} more)", items.len() - max);
	}
	out
}

/// `joined` for the common case of plain block numbers.
pub fn joined_blocks(blocks: &[u32], max: usize) -> String {
	joined(blocks, max, |n| format!("#{n}"))
}

/// Key of the refcount row that accompanies a stored value: the content hash with a zero byte
/// appended.
pub fn counter_key(hash: &DbHash) -> [u8; 33] {
	let mut key = [0u8; 33];
	key[..32].copy_from_slice(hash.as_ref());
	key
}

/// Read the refcount for `hash`. `None` means the row is absent or not a 4-byte LE integer.
pub fn read_counter(db: &dyn KeyValueDB, hash: &DbHash) -> std::io::Result<Option<u32>> {
	Ok(db
		.get(columns::TRANSACTION, &counter_key(hash))?
		.and_then(|v| <[u8; 4]>::try_from(&v[..]).ok().map(u32::from_le_bytes)))
}

/// Split a canonical lookup key — 4 big-endian number bytes then the 32-byte block hash —
/// which is how both `KEY_LOOKUP` values and `BODY_INDEX` keys are shaped.
pub fn split_lookup_key(key: &[u8]) -> Option<(u32, DbHash)> {
	if key.len() != 36 {
		return None;
	}
	let number = u32::from_be_bytes([key[0], key[1], key[2], key[3]]);
	let mut hash = [0u8; 32];
	hash.copy_from_slice(&key[4..36]);
	Some((number, DbHash::from(hash)))
}

/// The lookup key for a block number, alongside the block hash it embeds.
pub fn block_lookup_key(
	db: &dyn KeyValueDB,
	number: u32,
) -> std::io::Result<Option<(DbHash, Vec<u8>)>> {
	let Some(key) = db.get(columns::KEY_LOOKUP, &number.to_be_bytes())? else { return Ok(None) };
	Ok(split_lookup_key(&key).map(|(_, hash)| (hash, key)))
}

/// Read the best (highest) block recorded in the META column, as `(number, hash)`.
///
/// The value under `meta_keys::BEST_BLOCK` is a lookup key: a 4-byte big-endian block
/// number followed by the 32-byte block hash.
pub fn best_block(db: &dyn KeyValueDB) -> std::io::Result<Option<(u32, DbHash)>> {
	let Some(lookup) = db.get(columns::META, BEST_BLOCK_KEY)? else {
		return Ok(None);
	};
	if lookup.len() < 36 {
		return Ok(None);
	}
	let number = u32::from_be_bytes([lookup[0], lookup[1], lookup[2], lookup[3]]);
	let mut hash = [0u8; 32];
	hash.copy_from_slice(&lookup[4..36]);
	Ok(Some((number, DbHash::from(hash))))
}

/// How the RocksDB directory should be opened.
///
/// RocksDB's exclusive `LOCK` is taken by the *primary* open mode, so the repair
/// path needs the node stopped. Read-only work doesn't: a **secondary** instance attaches to
/// the same files without touching the lock, so it can run against a live node. The secondary
/// keeps its own small state directory (its `LOG`/`CURRENT`), which is what `--secondary`
/// names and `--live` puts in the system temp directory.
///
/// A secondary sees the primary's state as of the last `MANIFEST`/WAL replay: rows the node
/// has only in its memtable are not visible yet. `Database::try_catch_up_with_primary` pulls
/// it forward, and both binaries call it right after opening.
#[derive(Debug, Clone, Default)]
pub struct OpenMode {
	/// State directory for the secondary instance. `None` opens the database exclusively.
	pub secondary: Option<std::path::PathBuf>,
	/// Whether that directory was generated (and so should be removed afterwards).
	pub ephemeral: bool,
}

impl OpenMode {
	/// `live` uses a generated state directory under the system temp dir; an explicit
	/// `secondary` directory wins over it. Neither takes the primary's lock.
	pub fn new(live: bool, secondary: Option<PathBuf>) -> Self {
		match secondary {
			Some(dir) => Self { secondary: Some(dir), ephemeral: false },
			None if live => Self {
				secondary: Some(
					std::env::temp_dir().join(format!("tx-index-tool-{}", std::process::id())),
				),
				ephemeral: true,
			},
			None => Self::default(),
		}
	}

	/// Whether this mode forbids writes.
	pub fn is_read_only(&self) -> bool {
		self.secondary.is_some()
	}

	/// A mode for opening a *second* database in the same process. Two secondary instances must
	/// not share a state directory, so the tag is appended to it.
	pub fn sibling(&self, tag: &str) -> Self {
		match &self.secondary {
			Some(dir) => Self {
				secondary: Some(PathBuf::from(format!("{}-{tag}", dir.display()))),
				ephemeral: self.ephemeral,
			},
			None => Self::default(),
		}
	}

	/// Remove the generated secondary state directory, if this mode created one.
	pub fn cleanup(&self) {
		if self.ephemeral {
			if let Some(dir) = &self.secondary {
				let _ = std::fs::remove_dir_all(dir);
			}
		}
	}
}

/// Open the node's RocksDB directory, honouring [`OpenMode`], and bring a secondary instance
/// up to date with the primary.
///
/// `create_if_missing` is off: a mistyped path must fail rather than silently produce an empty
/// database that then reports zero entries.
pub fn open_database(path: &std::path::Path, mode: &OpenMode) -> std::io::Result<Database> {
	let mut cfg = DatabaseConfig::with_columns(NUM_COLUMNS);
	cfg.create_if_missing = false;
	cfg.secondary = mode.secondary.clone();

	// The column indices above are an on-disk contract. Substrate stamps the schema version
	// beside the data, so check it rather than trusting the comment.
	if let Ok(found) = std::fs::read_to_string(path.join("db_version")) {
		let found = found.trim();
		if found != SUPPORTED_DB_VERSION {
			eprintln!(
				"warning: database schema version {found}, expected {SUPPORTED_DB_VERSION} — \
				 column indices may have moved; treat the output with suspicion."
			);
		}
	}

	let db = Database::open(&cfg, path).map_err(|e| {
		if mode.is_read_only() {
			e
		} else {
			std::io::Error::other(format!(
				"{e}\nnote: rocksdb takes an exclusive lock — stop the node, or pass --live to \
				 attach read-only as a secondary instance."
			))
		}
	})?;

	if mode.is_read_only() {
		// Pull in whatever the primary has flushed since it opened.
		if let Err(e) = db.try_catch_up_with_primary() {
			eprintln!("warning: could not catch up with the primary: {e}");
		}
	}
	Ok(db)
}

/// Lower bound for a plausible millisecond UNIX timestamp (2017-01-01).
pub const MIN_PLAUSIBLE_MS: u64 = 1_483_228_800_000;
/// Upper bound for a plausible millisecond UNIX timestamp (2100-01-01).
pub const MAX_PLAUSIBLE_MS: u64 = 4_102_444_800_000;

/// Best-effort recovery of a block's wall-clock time from its `Timestamp::set` inherent.
///
/// That inherent is a bare (unsigned) extrinsic whose call is `(pallet_index, call_index,
/// Compact<u64> moment)` and nothing else, so the first `Full` extrinsic that
/// decodes exactly that way: version byte with the signed bit clear, two index bytes, then a
/// compact that consumes the rest of the buffer and lands in a plausible range. No call
/// indices are hardcoded, so this stays runtime-agnostic — but it *is* a heuristic, and the
/// report labels the timestamps as such.
pub fn block_timestamp_ms(fulls: &[Vec<u8>]) -> Option<u64> {
	for bytes in fulls {
		if bytes.len() < 4 || bytes[0] & 0b1000_0000 != 0 {
			continue;
		}
		let mut rest = &bytes[3..];
		let Ok(moment) = codec::Compact::<u64>::decode(&mut rest) else { continue };
		if !rest.is_empty() {
			continue;
		}
		if (MIN_PLAUSIBLE_MS..=MAX_PLAUSIBLE_MS).contains(&moment.0) {
			return Some(moment.0);
		}
	}
	None
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → `(year, month, day)`.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
	let z = days + 719_468;
	let era = z.div_euclid(146_097);
	let doe = z.rem_euclid(146_097);
	let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
	let y = yoe + era * 400;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
	let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
	(if m <= 2 { y + 1 } else { y }, m, d)
}

/// Format a millisecond UNIX timestamp as `YYYY-MM-DD HH:MM:SS UTC`.
pub fn format_timestamp_ms(ms: u64) -> String {
	let secs = (ms / 1_000) as i64;
	let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
	let tod = secs.rem_euclid(86_400);
	format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", tod / 3_600, (tod % 3_600) / 60, tod % 60,)
}

/// Render a byte size with a binary-unit suffix.
pub fn human_bytes(n: u64) -> String {
	const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
	let mut value = n as f64;
	let mut unit = 0;
	while value >= 1_024.0 && unit + 1 < UNITS.len() {
		value /= 1_024.0;
		unit += 1;
	}
	if unit == 0 {
		format!("{n} B")
	} else {
		format!("{value:.2} {}", UNITS[unit])
	}
}

/// Every content hash a single block's body references, in `BODY_INDEX` order (deduplicated),
/// together with that block's timestamp. `Ok(None)` when the block isn't in the database or
/// has no indexed body.
/// Canonical hash of a block number, from `KEY_LOOKUP`. `Ok(None)` when the database has no
/// such block.
pub fn block_hash(db: &dyn KeyValueDB, number: u32) -> std::io::Result<Option<DbHash>> {
	let Some(lookup_key) = db.get(columns::KEY_LOOKUP, &number.to_be_bytes())? else {
		return Ok(None);
	};
	if lookup_key.len() != 36 {
		return Ok(None);
	}
	let mut bytes = [0u8; 32];
	bytes.copy_from_slice(&lookup_key[4..36]);
	Ok(Some(DbHash::from(bytes)))
}

pub fn block_referenced_hashes(
	db: &dyn KeyValueDB,
	number: u32,
) -> std::io::Result<Option<(Vec<DbHash>, Option<u64>)>> {
	let Some(lookup_key) = db.get(columns::KEY_LOOKUP, &number.to_be_bytes())? else {
		return Ok(None);
	};
	let Some(body_index) = db.get(columns::BODY_INDEX, &lookup_key)? else { return Ok(None) };
	let Ok(index) = Vec::<BareDbExtrinsic>::decode(&mut &body_index[..]) else { return Ok(None) };

	let mut hashes: Vec<DbHash> = Vec::new();
	let mut fulls: Vec<Vec<u8>> = Vec::new();
	for ex in index {
		match ex {
			BareDbExtrinsic::Indexed { hash, .. } => hashes.push(hash),
			BareDbExtrinsic::MultiRenew { hashes: inner, .. } => hashes.extend(inner),
			BareDbExtrinsic::Full(bytes) => fulls.push(bytes),
		}
	}
	let time_ms = block_timestamp_ms(&fulls);
	let mut seen = std::collections::HashSet::new();
	hashes.retain(|h| seen.insert(*h));
	Ok(Some((hashes, time_ms)))
}

/// Render bytes in `hexdump -C` layout: offset, 16 hex columns split into two groups of
/// eight, then the printable-ASCII gutter.
pub fn hexdump(bytes: &[u8], indent: &str) -> String {
	let mut out = String::new();
	for (row, chunk) in bytes.chunks(16).enumerate() {
		out.push_str(indent);
		out.push_str(&format!("{:08x}  ", row * 16));
		for i in 0..16 {
			match chunk.get(i) {
				Some(b) => out.push_str(&format!("{b:02x} ")),
				None => out.push_str("   "),
			}
			if i == 7 {
				out.push(' ');
			}
		}
		out.push_str(" |");
		for b in chunk {
			out.push(if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '.' });
		}
		out.push_str("|\n");
	}
	out
}
