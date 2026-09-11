// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Recomputation of the storage proof the transaction-storage inherent provider emits.

use crate::common::*;
use codec::Decode;
use kvdb::KeyValueDB;
use std::fmt;

/// Result of a storage-proof computation, with diagnostic info about which chunk got picked.
#[derive(Debug, Clone)]
pub struct ProofResult {
	/// The target block whose indexed body is being proved.
	pub target_block: u32,
	/// Block hash of the target block.
	pub target_block_hash: DbHash,
	/// Randomness used to select the chunk (typically the parent hash of the proof-emitting
	/// block).
	pub random_hash: [u8; 32],
	/// Total number of chunks across all indexed transactions in the block.
	pub total_chunks: u32,
	/// Chunk index (in the flattened block-wide enumeration) selected by `random_chunk`.
	pub selected_chunk_index: u32,
	/// Position of the transaction in the indexed body containing the selected chunk.
	pub tx_index_in_body: usize,
	/// Content hash of the transaction containing the selected chunk.
	pub tx_content_hash: DbHash,
	/// Per-tx chunk index (offset within the transaction's chunk list).
	pub chunk_index_within_tx: u32,
	/// The chunk bytes that the proof attests to.
	pub chunk: Vec<u8>,
	/// SCALE-encoded trie nodes forming the Merkle proof.
	pub proof: Vec<Vec<u8>>,
	/// Per-transaction trie root that the proof is rooted at — useful for external verification.
	pub tx_chunk_root: DbHash,
	/// True iff the proof verifies against the recomputed tx_chunk_root locally.
	pub verified: bool,
	/// The chunk root the chain committed to for this transaction, when the caller supplied it
	/// (`TransactionInfo::chunk_root` from on-chain state).
	pub expected_root: Option<DbHash>,
}

impl ProofResult {
	/// Whether the locally recomputed root matches what the chain recorded. `None` when no
	/// expected root was given — in which case `verified` only says the proof is internally
	/// consistent with the bytes currently on disk.
	pub fn agrees_with_chain(&self) -> Option<bool> {
		self.expected_root.map(|expected| expected == self.tx_chunk_root)
	}

	/// Whether this result is a clean bill of health: the proof verifies and, if an expected
	/// root was supplied, it matches.
	pub fn is_good(&self) -> bool {
		self.verified && self.agrees_with_chain() != Some(false)
	}
}

/// Compute the storage proof the same way `transaction-storage` inherent provider does:
/// read the block's indexed body, flatten Indexed + MultiRenew entries in submission order
/// to a `Vec<Vec<u8>>` of transaction blobs, then call
/// `sp_transaction_storage_proof::registration::build_proof(random_hash, transactions)`.
///
/// Returns `Ok(None)` when the block has no indexed body (no `BODY_INDEX` entry, or only
/// `Full` extrinsics, or zero total chunks).
/// `expected_root` is the chunk root the chain committed to when the data was stored
/// (`TransactionInfo::chunk_root`). Supply it to turn the local, self-consistent check into a
/// real one: without it, the proof and the root are both derived from the same on-disk bytes, so
/// a value that has since changed still verifies.
pub fn compute_storage_proof(
	db: &dyn KeyValueDB,
	target_block: u32,
	random_hash: [u8; 32],
	expected_root: Option<DbHash>,
) -> std::io::Result<Option<ProofResult>> {
	let Some((target_block_hash, lookup_key)) = block_lookup_key(db, target_block)? else {
		return Ok(None);
	};

	let Some(body_index_bytes) = db.get(columns::BODY_INDEX, &lookup_key)? else {
		return Ok(None);
	};

	let Ok(index) = Vec::<BareDbExtrinsic>::decode(&mut &body_index_bytes[..]) else {
		return Err(std::io::Error::new(
			std::io::ErrorKind::InvalidData,
			"BODY_INDEX decode failed for target block",
		));
	};

	// Mirror sc-client-db's `block_indexed_body`: flatten Indexed + MultiRenew in submission
	// order, dropping Full. Track each blob's content hash for cross-referencing.
	let mut hashes_ordered: Vec<DbHash> = Vec::new();
	let mut transactions: Vec<Vec<u8>> = Vec::new();
	for ex in index {
		match ex {
			BareDbExtrinsic::Indexed { hash, .. } => {
				let value = db.get(columns::TRANSACTION, hash.as_ref())?.ok_or_else(|| {
					std::io::Error::new(
						std::io::ErrorKind::NotFound,
						format!("col11 missing value for hash {hash:?}"),
					)
				})?;
				hashes_ordered.push(hash);
				transactions.push(value);
			},
			BareDbExtrinsic::MultiRenew { hashes, .. } =>
				for h in hashes {
					let value = db.get(columns::TRANSACTION, h.as_ref())?.ok_or_else(|| {
						std::io::Error::new(
							std::io::ErrorKind::NotFound,
							format!("col11 missing value for hash {h:?}"),
						)
					})?;
					hashes_ordered.push(h);
					transactions.push(value);
				},
			BareDbExtrinsic::Full(_) => {},
		}
	}

	if transactions.is_empty() {
		return Ok(None);
	}

	// Compute the total chunks and which one would be selected — needed for both the
	// pretty-printing AND to find which transaction the chunk falls into. (build_proof
	// internally does the same selection but doesn't expose the resolved tx index.)
	use sp_transaction_storage_proof::{num_chunks, random_chunk, CHUNK_SIZE};
	let total_chunks: u32 = transactions.iter().map(|t| num_chunks(t.len() as u32)).sum();
	if total_chunks == 0 {
		return Ok(None);
	}
	let selected_chunk_index = random_chunk(&random_hash, total_chunks);

	let mut cumulative: u32 = 0;
	let mut tx_index_in_body = 0;
	let mut chunk_index_within_tx = 0u32;
	for (i, tx) in transactions.iter().enumerate() {
		let n = num_chunks(tx.len() as u32);
		if selected_chunk_index < cumulative + n {
			tx_index_in_body = i;
			chunk_index_within_tx = selected_chunk_index - cumulative;
			break;
		}
		cumulative += n;
	}
	let tx_content_hash = hashes_ordered[tx_index_in_body];

	// Recompute the target transaction's chunk root locally first, so `transactions` can then
	// be moved into `build_proof` rather than cloned — that vec holds every indexed value in
	// the block, which is megabytes on a busy one.
	use sp_trie::TrieMut;
	type Hasher = sp_core::Blake2Hasher;
	type Layout = sp_trie::LayoutV1<Hasher>;
	let mut db_mem = sp_trie::MemoryDB::<Hasher>::default();
	let mut tx_chunk_root = sp_trie::empty_trie_root::<Layout>();
	{
		let mut trie =
			sp_trie::TrieDBMutBuilder::<Layout>::new(&mut db_mem, &mut tx_chunk_root).build();
		for (i, chunk) in transactions[tx_index_in_body].chunks(CHUNK_SIZE).enumerate() {
			let _ = trie.insert(&sp_transaction_storage_proof::encode_index(i as u32), chunk);
		}
		trie.commit();
	}

	// Build the actual proof via the upstream function — guarantees byte-identical output
	// to what the runtime's inherent provider would emit.
	let proof = sp_transaction_storage_proof::registration::build_proof(&random_hash, transactions)
		.map_err(|e| std::io::Error::other(format!("build_proof: {e}")))?
		.ok_or_else(|| std::io::Error::other("build_proof returned None unexpectedly"))?;

	let verified = sp_trie::verify_trie_proof::<Layout, _, _, _>(
		&tx_chunk_root,
		&proof.proof,
		&[(
			sp_transaction_storage_proof::encode_index(chunk_index_within_tx),
			Some(proof.chunk.clone()),
		)],
	)
	.is_ok();

	Ok(Some(ProofResult {
		target_block,
		target_block_hash,
		random_hash,
		total_chunks,
		selected_chunk_index,
		tx_index_in_body,
		tx_content_hash,
		chunk_index_within_tx,
		chunk: proof.chunk,
		proof: proof.proof,
		tx_chunk_root,
		verified,
		expected_root,
	}))
}

impl fmt::Display for ProofResult {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(f, "Storage proof for block #{}", self.target_block)?;
		writeln!(f, "  block hash:               {}", hex(self.target_block_hash.as_ref()))?;
		writeln!(f, "  randomness:               {}", hex(&self.random_hash))?;
		writeln!(f, "  total chunks in block:    {}", self.total_chunks)?;
		writeln!(f, "  selected chunk (global):  {}", self.selected_chunk_index)?;
		writeln!(f, "  inside body tx position:  {}", self.tx_index_in_body)?;
		writeln!(f, "  tx content hash:          {}", hex(self.tx_content_hash.as_ref()))?;
		writeln!(f, "  chunk index within tx:    {}", self.chunk_index_within_tx)?;
		writeln!(f, "  tx chunk root:            {}", hex(self.tx_chunk_root.as_ref()))?;
		writeln!(f, "  chunk size:               {} bytes", self.chunk.len())?;
		writeln!(f, "  proof nodes:              {}", self.proof.len())?;
		let proof_size: usize = self.proof.iter().map(|n| n.len()).sum();
		writeln!(f, "  proof total bytes:        {proof_size}")?;
		writeln!(f, "  local verification:       {}", if self.verified { "OK" } else { "FAILED" })?;
		match (self.expected_root, self.agrees_with_chain()) {
			(Some(expected), Some(true)) => {
				writeln!(f, "  expected chunk root:      {}", hex(expected.as_ref()))?;
				writeln!(f, "  chain agreement:          OK — the stored bytes are what the chain committed to")?;
			},
			(Some(expected), Some(false)) => {
				writeln!(f, "  expected chunk root:      {}", hex(expected.as_ref()))?;
				writeln!(f, "  chain agreement:          MISMATCH")?;
				writeln!(
					f,
					"    the recomputed root differs from the one recorded on chain, so the"
				)?;
				writeln!(
					f,
					"    bytes on disk are not the bytes this entry was stored with — the"
				)?;
				writeln!(f, "    runtime would reject a proof built from them")?;
			},
			_ => {
				writeln!(
					f,
					"  chain agreement:          not checked (pass --expect-root to compare)"
				)?;
			},
		}
		writeln!(
			f,
			"  chunk (first 32 bytes):   {}{}",
			hex(&self.chunk[..self.chunk.len().min(32)]),
			if self.chunk.len() > 32 { "…" } else { "" },
		)?;
		Ok(())
	}
}
