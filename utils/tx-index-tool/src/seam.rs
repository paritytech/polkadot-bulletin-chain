// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Classification of the `BODY_INDEX.header` ↔ col11 seam.
//!
//! A body is reassembled as exactly `header ++ value` (sc-client-db `body_uncached`). When the
//! extrinsic that produced an entry had fields *after* its indexed data — the shape
//! polkadot-bulletin-chain#574 introduced in `HopPromotion::promote` — the two halves cannot
//! both be right:
//!
//! - the value hashes to its key **and** the reassembled body matches what was authored is
//!   impossible, because the authored bytes were `header ++ data ++ trailing-fields`;
//! - so a database sits in one of two broken states, and they are told apart by whether the data
//!   window the call declares actually holds the value.
//!
//! Both states have identical length, and the node decodes bodies as `OpaqueExtrinsic` (a
//! length-prefixed blob), so neither is caught client-side — the failure only surfaces when a
//! runtime tries to decode the extrinsic.

use crate::common::*;
use codec::Decode;
use kvdb::KeyValueDB;
use std::{
	fmt,
	time::{Duration, Instant},
};

/// What state one indexed entry's seam is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamState {
	/// The value hashes to its key and sits exactly where the call declares it. The pair
	/// round-trips, so the body reassembles to what was authored.
	Healthy,
	/// The value does not hash to its key, but it is the trailing window of the authored
	/// extrinsic: the original #574 mis-split. Bodies still reassemble correctly, so blocks
	/// remain executable; bitswap and storage proofs for this entry are broken.
	OriginalMisaligned,
	/// The value hashes to its key but is not what the call's data window holds: a col11-only
	/// repair. Integrity looks clean while every block referencing it is unexecutable, and the
	/// authored trailing fields are gone.
	HalfRepaired,
	/// A single renewal, stored as `Indexed { hash, header: <the whole extrinsic> }` (see
	/// sc-client-db's "Single renewal: backwards-compatible Indexed variant"). The value was
	/// stored by an earlier block, so `header ++ value` is not meant to reassemble and none of
	/// the seam reasoning applies.
	SingleRenewal,
	/// Neither the hash nor the declared window matches, and no data window of the value's
	/// length was found at all.
	Unknown,
}

impl SeamState {
	/// Whether a body containing this entry can still be executed by a runtime.
	pub fn body_executable(&self) -> bool {
		matches!(
			self,
			SeamState::Healthy | SeamState::OriginalMisaligned | SeamState::SingleRenewal
		)
	}

	/// Short label for the report.
	pub fn label(&self) -> &'static str {
		match self {
			SeamState::Healthy => "healthy",
			SeamState::OriginalMisaligned => "original mis-split (executable, hash broken)",
			SeamState::HalfRepaired => "col11-only repair (hash ok, NOT executable)",
			SeamState::SingleRenewal => "single renewal (header is the whole extrinsic)",
			SeamState::Unknown => "unknown",
		}
	}
}

/// One indexed entry's seam.
#[derive(Debug, Clone)]
pub struct SeamEntry {
	/// Block whose body indexes this entry.
	pub block: u32,
	/// The col11 slot key.
	pub content_hash: DbHash,
	/// Size of the stored value.
	pub value_size: usize,
	/// Size of the `BODY_INDEX` header beside it.
	pub header_size: usize,
	/// Whether the value hashes to its key.
	pub hashes: bool,
	/// Whether the reassembled extrinsic declares a data window holding exactly the value.
	pub window_matches: bool,
	/// Bytes between the end of the declared data window and the end of the extrinsic — the
	/// trailing fields, when they are present.
	pub trailing: Option<usize>,
	/// The verdict.
	pub state: SeamState,
}

/// Result of `verify_seams`.
#[derive(Debug, Default)]
pub struct SeamReport {
	/// Indexed entries examined.
	pub examined: usize,
	/// Entries whose pair round-trips.
	pub healthy: usize,
	/// Entries still in the original mis-split state.
	pub original_misaligned: usize,
	/// Entries a col11-only repair left unexecutable.
	pub half_repaired: usize,
	/// Single renewals, which the seam reasoning does not apply to.
	pub single_renewals: usize,
	/// Entries that fit neither description.
	pub unknown: usize,
	/// Every entry that is not healthy, block order.
	pub rows: Vec<SeamEntry>,
	/// Wall-clock duration.
	pub elapsed: Duration,
}

impl SeamReport {
	/// Blocks that no runtime can execute any more, in ascending order.
	pub fn unexecutable_blocks(&self) -> Vec<u32> {
		let mut blocks: Vec<u32> = self
			.rows
			.iter()
			.filter(|r| !r.state.body_executable())
			.map(|r| r.block)
			.collect();
		blocks.sort_unstable();
		blocks.dedup();
		blocks
	}

	/// True when every entry's pair round-trips.
	pub fn is_clean(&self) -> bool {
		self.healthy + self.single_renewals == self.examined
	}
}

/// Decode a SCALE compact at `offset`, returning `(value, width)`.
fn compact_at(buf: &[u8], offset: usize) -> Option<(u64, usize)> {
	let first = *buf.get(offset)?;
	match first & 0b11 {
		0b00 => Some((u64::from(first >> 2), 1)),
		0b01 => {
			let raw = u16::from_le_bytes(buf.get(offset..offset + 2)?.try_into().ok()?);
			Some((u64::from(raw >> 2), 2))
		},
		0b10 => {
			let raw = u32::from_le_bytes(buf.get(offset..offset + 4)?.try_into().ok()?);
			Some((u64::from(raw >> 2), 4))
		},
		_ => {
			let width = usize::from(first >> 2) + 4;
			let bytes = buf.get(offset + 1..offset + 1 + width)?;
			let mut value = 0u64;
			for (i, b) in bytes.iter().take(8).enumerate() {
				value |= u64::from(*b) << (8 * i);
			}
			Some((value, 1 + width))
		},
	}
}

/// How far the search for the call's data-length prefix reaches into the header. A signed v4
/// preamble is ~116 bytes; a general v5 one is much shorter. 512 covers both with room to spare.
const MAX_PREAMBLE: usize = 512;

/// Find a `Compact(value.len())` in the reassembled extrinsic that is actually followed by
/// `value`, returning the trailing byte count after that window.
///
/// Limitation: if the data repeats with exactly the period of the shift (uniform padding, say),
/// a shifted window is byte-identical to the correct one and this cannot tell them apart. Real
/// payloads — compressed archives, CAR files — do not have that property.
fn declared_window(reassembled: &[u8], value: &[u8]) -> Option<usize> {
	let want = value.len() as u64;
	let limit = reassembled.len().saturating_sub(value.len()).min(MAX_PREAMBLE);
	for offset in 0..=limit {
		let Some((declared, width)) = compact_at(reassembled, offset) else { continue };
		if declared != want {
			continue;
		}
		let start = offset + width;
		let end = start.checked_add(value.len())?;
		if end <= reassembled.len() && &reassembled[start..end] == value {
			return Some(reassembled.len() - end);
		}
	}
	None
}

/// Is `header` on its own a complete extrinsic — a compact length prefix followed by exactly
/// that many bytes? That is how a single renewal is stored, and it never holds for a genuine
/// split, whose header is a prefix declaring a longer payload.
fn header_is_whole_extrinsic(header: &[u8]) -> bool {
	match compact_at(header, 0) {
		Some((declared, width)) => declared == (header.len() - width) as u64,
		None => false,
	}
}

/// Classify one entry from its two halves.
pub fn classify(
	header: &[u8],
	value: &[u8],
	content_hash: DbHash,
) -> (SeamState, bool, bool, Option<usize>) {
	let hashes = HashAlgo::identify(content_hash, value).is_some();

	let mut reassembled = Vec::with_capacity(header.len() + value.len());
	reassembled.extend_from_slice(header);
	reassembled.extend_from_slice(value);
	let trailing = declared_window(&reassembled, value);
	let window_matches = trailing.is_some();

	let state = if header_is_whole_extrinsic(header) {
		SeamState::SingleRenewal
	} else {
		match (hashes, window_matches) {
			(true, true) => SeamState::Healthy,
			(false, _) => SeamState::OriginalMisaligned,
			(true, false) => SeamState::HalfRepaired,
		}
	};
	(state, hashes, window_matches, trailing)
}

/// Walk `BODY_INDEX` and classify every indexed entry's seam. Read-only.
pub fn verify_seams(db: &dyn KeyValueDB) -> std::io::Result<SeamReport> {
	let started = Instant::now();
	let mut report = SeamReport::default();

	for entry in db.iter(columns::BODY_INDEX) {
		let (k, v) = entry?;
		let Some((block, _)) = split_lookup_key(&k) else { continue };
		let Ok(index) = Vec::<BareDbExtrinsic>::decode(&mut &v[..]) else { continue };

		for ex in index {
			let BareDbExtrinsic::Indexed { hash, header } = ex else { continue };
			let Some(value) = db.get(columns::TRANSACTION, hash.as_ref())? else { continue };

			report.examined += 1;
			let (state, hashes, window_matches, trailing) = classify(&header, &value, hash);
			match state {
				SeamState::Healthy => {
					report.healthy += 1;
					continue;
				},
				SeamState::SingleRenewal => {
					report.single_renewals += 1;
					continue;
				},
				SeamState::OriginalMisaligned => report.original_misaligned += 1,
				SeamState::HalfRepaired => report.half_repaired += 1,
				SeamState::Unknown => report.unknown += 1,
			}
			report.rows.push(SeamEntry {
				block,
				content_hash: hash,
				value_size: value.len(),
				header_size: header.len(),
				hashes,
				window_matches,
				trailing,
				state,
			});
		}
	}

	report.rows.sort_by_key(|r| (r.block, r.content_hash));
	report.elapsed = started.elapsed();
	Ok(report)
}

impl fmt::Display for SeamReport {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(f, "BODY_INDEX ↔ col11 seam verification")?;
		writeln!(f, "====================================")?;
		writeln!(f, "Elapsed:                     {:?}", self.elapsed)?;
		writeln!(f, "Indexed entries examined:    {}", self.examined)?;
		writeln!(f, "  healthy (pair round-trips):{:>6}", self.healthy)?;
		writeln!(
			f,
			"  single renewals:           {:>6}   header is the whole extrinsic, not a split",
			self.single_renewals,
		)?;
		writeln!(
			f,
			"  original mis-split:        {:>6}   executable, but the value does not hash",
			self.original_misaligned,
		)?;
		writeln!(
			f,
			"  col11-only repair:         {:>6}   hashes clean, body NOT executable",
			self.half_repaired,
		)?;
		if self.unknown > 0 {
			writeln!(f, "  unknown:                   {:>6}", self.unknown)?;
		}

		if self.is_clean() {
			writeln!(f)?;
			writeln!(f, "Result: CLEAN — every indexed entry reassembles to what was authored.")?;
			return Ok(());
		}

		writeln!(f)?;
		for r in &self.rows {
			writeln!(f, "  #{}  {}", r.block, hex(r.content_hash.as_ref()))?;
			writeln!(
				f,
				"    value {} B, header {} B, hash {}, data window {}{}",
				r.value_size,
				r.header_size,
				if r.hashes { "ok" } else { "MISMATCH" },
				if r.window_matches { "holds the value" } else { "does NOT hold the value" },
				match r.trailing {
					Some(0) => "  (data is the last field)".to_string(),
					Some(n) => format!("  ({n} trailing bytes after the data)"),
					None => String::new(),
				},
			)?;
			writeln!(f, "    {}", r.state.label())?;
		}

		let unexecutable = self.unexecutable_blocks();
		if !unexecutable.is_empty() {
			writeln!(f)?;
			writeln!(
				f,
				"{} block(s) cannot be executed by any runtime: {}",
				unexecutable.len(),
				joined_blocks(&unexecutable, 20),
			)?;
			writeln!(
				f,
				"A col11-only repair overwrote the value and discarded the extrinsic's trailing"
			)?;
			writeln!(
				f,
				"fields, which existed only in that value. They cannot be recovered from this"
			)?;
			writeln!(
				f,
				"database — restoring executability needs the authored extrinsic from elsewhere"
			)?;
			writeln!(f, "(a node that still holds the original pair, or the collator).")?;
		}
		if self.original_misaligned > 0 {
			writeln!(f)?;
			writeln!(
				f,
				"The mis-split entries are still executable. Repairing them with `realign --apply`"
			)?;
			writeln!(f, "would trade that away for a correct hash — see its warning first.")?;
		}
		Ok(())
	}
}
