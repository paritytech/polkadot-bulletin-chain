// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! RocksDB LDB tool integration: column dump, entry parsing, and verification.

use super::{config::*, network::env_or_default};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

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

pub fn get_ldb_path() -> String {
	env_or_default(LDB_PATH_ENV, DEFAULT_LDB_PATH)
}

#[derive(Debug, Clone)]
pub struct LdbEntry {
	pub key: String,   // hex without 0x prefix
	pub value: String, // hex without 0x prefix
}

impl LdbEntry {
	/// Key ends with "00" indicates refcount entry (RocksDB storage convention).
	pub fn is_refcount(&self) -> bool {
		self.key.ends_with("00")
	}

	pub fn content_hash(&self) -> &str {
		if self.is_refcount() {
			&self.key[..self.key.len() - 2]
		} else {
			&self.key
		}
	}

	/// Parse little-endian u32 refcount value.
	pub fn parse_refcount(&self) -> Option<u32> {
		if !self.is_refcount() {
			return None;
		}
		let bytes = hex::decode(&self.value).ok()?;
		if bytes.len() >= 4 {
			Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
		} else {
			None
		}
	}
}

#[derive(Debug, Clone)]
pub struct LdbColumnDump {
	pub entries: Vec<LdbEntry>,
	pub key_count: usize,
}

impl LdbColumnDump {
	pub fn is_empty(&self) -> bool {
		self.key_count == 0
	}

	pub fn data_entries(&self) -> Vec<&LdbEntry> {
		self.entries.iter().filter(|e| !e.is_refcount()).collect()
	}

	pub fn get_refcount(&self, content_hash: &str) -> Option<u32> {
		let hash = content_hash.strip_prefix("0x").unwrap_or(content_hash);
		self.entries
			.iter()
			.find(|e| e.is_refcount() && e.content_hash().eq_ignore_ascii_case(hash))
			.and_then(|e| e.parse_refcount())
	}

	pub fn log(&self, label: &str) {
		log::info!("{}: {} keys in column", label, self.key_count);
		for entry in &self.entries {
			if entry.is_refcount() {
				log::info!(
					"  [REFCOUNT] {} => {} (count: {})",
					entry.key,
					entry.value,
					entry.parse_refcount().unwrap_or(0)
				);
			} else {
				let data_preview = if entry.value.len() > 40 {
					format!("{}...", &entry.value[..40])
				} else {
					entry.value.clone()
				};
				log::info!("  [DATA] {} => {}", entry.key, data_preview);
			}
		}
	}
}

pub fn ldb_dump_column(db_path: &Path, column: &str) -> Result<LdbColumnDump> {
	use std::process::Command;

	let ldb_path = get_ldb_path();
	let db_path_str = db_path.to_string_lossy();

	log::debug!("Running: {} --db={} --column_family={} dump --hex", ldb_path, db_path_str, column);

	let output = Command::new(&ldb_path)
		.args([
			&format!("--db={}", db_path_str),
			&format!("--column_family={}", column),
			"dump",
			"--hex",
		])
		.output()
		.context(format!(
			"Failed to execute ldb tool at '{}'. Set {} env var if needed.",
			ldb_path, LDB_PATH_ENV
		))?;

	if !output.status.success() {
		let stderr = String::from_utf8_lossy(&output.stderr);
		anyhow::bail!("ldb command failed: {}", stderr);
	}

	let stdout = String::from_utf8_lossy(&output.stdout);
	parse_ldb_dump_output(&stdout)
}

fn parse_ldb_dump_output(output: &str) -> Result<LdbColumnDump> {
	let mut entries = Vec::new();
	let mut key_count = 0;

	for line in output.lines() {
		let line = line.trim();

		// Parse "Keys in range: N" line
		if line.starts_with("Keys in range:") {
			if let Some(count_str) = line.strip_prefix("Keys in range:") {
				key_count = count_str.trim().parse().unwrap_or(0);
			}
			continue;
		}

		// Parse "0xKEY ==> 0xVALUE" lines
		if line.contains(" ==> ") {
			let parts: Vec<&str> = line.split(" ==> ").collect();
			if parts.len() == 2 {
				let key = parts[0].strip_prefix("0x").unwrap_or(parts[0]).to_uppercase();
				let value = parts[1].strip_prefix("0x").unwrap_or(parts[1]).to_uppercase();
				entries.push(LdbEntry { key, value });
			}
		}
	}

	Ok(LdbColumnDump { entries, key_count })
}

pub fn verify_col11(db_path: &std::path::Path, label: &str) -> Result<LdbColumnDump> {
	let dump = ldb_dump_column(db_path, TRANSACTION_STORAGE_COLUMN).context(format!(
		"LDB verification failed ({}). Ensure rocksdb_ldb tool is available and {} env var is set.",
		label, LDB_PATH_ENV
	))?;
	dump.log(label);
	Ok(dump)
}
