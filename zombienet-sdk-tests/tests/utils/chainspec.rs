// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Chain spec modification utilities for code_substitutes-based recovery.
//!
//! After a parachain stalls due to a broken runtime, recovery requires:
//! 1. `force_set_current_code` on the relay chain (so validators use the fix code)
//! 2. Adding `codeSubstitutes` to the collator's chain spec (so the client uses the fix code)
//! 3. Restarting the collator (picks up the modified chain spec)

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// Find ALL parachain chain spec files in the zombienet network base directory.
///
/// Searches recursively for JSON files whose `"name"` or `"id"` field contains `search_name`.
/// Returns all matches because zombienet may store copies in multiple locations (base dir,
/// node cfg dir). The collator reads from its cfg dir copy, so all must be modified.
pub fn find_all_parachain_chain_specs(base_dir: &str, search_name: &str) -> Result<Vec<PathBuf>> {
	log::info!("Searching for parachain chain specs containing '{}' in {}", search_name, base_dir);
	let mut results = Vec::new();
	find_chain_specs_recursive(Path::new(base_dir), search_name, &mut results)?;
	if results.is_empty() {
		anyhow::bail!(
			"No parachain chain spec with name/id containing '{}' found in {}",
			search_name,
			base_dir
		);
	}
	for path in &results {
		log::info!("Found parachain chain spec: {}", path.display());
	}
	Ok(results)
}

fn find_chain_specs_recursive(
	dir: &Path,
	search_name: &str,
	results: &mut Vec<PathBuf>,
) -> Result<()> {
	if !dir.is_dir() {
		return Ok(());
	}

	let search_lower = search_name.to_lowercase();

	for entry in std::fs::read_dir(dir)? {
		let entry = entry?;
		let path = entry.path();

		if path.is_dir() {
			find_chain_specs_recursive(&path, search_name, results)?;
		} else if path.extension().map_or(false, |ext| ext == "json") {
			if let Ok(contents) = std::fs::read_to_string(&path) {
				if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
					let name_matches = json
						.get("name")
						.and_then(|n| n.as_str())
						.map_or(false, |n| n.to_lowercase().contains(&search_lower));
					let id_matches = json
						.get("id")
						.and_then(|n| n.as_str())
						.map_or(false, |n| n.to_lowercase().contains(&search_lower));

					if name_matches || id_matches {
						results.push(path);
					}
				}
			}
		}
	}

	Ok(())
}

/// Add a `codeSubstitutes` entry to a chain spec file.
///
/// The substrate client reads `codeSubstitutes` during initialization. Each entry maps a
/// block number to WASM code that replaces the on-chain `:code` for execution. The substitute
/// is used starting at the given block number until the on-chain `spec_version` changes
/// (i.e., a proper runtime upgrade is enacted).
///
/// This allows recovering a stalled chain: the substitute code is used instead of the
/// broken on-chain runtime, enabling block production to resume.
pub fn add_code_substitute(spec_path: &Path, block_number: u64, wasm_path: &str) -> Result<()> {
	let wasm_bytes = std::fs::read(wasm_path)
		.map_err(|e| anyhow!("Failed to read WASM from '{}': {}", wasm_path, e))?;

	log::info!(
		"Adding code substitute at block {} (WASM: {} bytes from {})",
		block_number,
		wasm_bytes.len(),
		wasm_path
	);

	let contents = std::fs::read_to_string(spec_path)
		.map_err(|e| anyhow!("Failed to read chain spec '{}': {}", spec_path.display(), e))?;

	let mut json: serde_json::Value = serde_json::from_str(&contents)
		.map_err(|e| anyhow!("Failed to parse chain spec JSON: {}", e))?;

	let wasm_hex = format!("0x{}", hex::encode(&wasm_bytes));

	let obj = json
		.as_object_mut()
		.ok_or_else(|| anyhow!("Chain spec root is not a JSON object"))?;

	let code_subs = obj.entry("codeSubstitutes").or_insert_with(|| serde_json::json!({}));

	code_subs
		.as_object_mut()
		.ok_or_else(|| anyhow!("codeSubstitutes field is not a JSON object"))?
		.insert(block_number.to_string(), serde_json::json!(wasm_hex));

	let output = serde_json::to_string_pretty(&json)?;
	std::fs::write(spec_path, output)
		.map_err(|e| anyhow!("Failed to write chain spec '{}': {}", spec_path.display(), e))?;

	log::info!("Code substitute added at block {} in {}", block_number, spec_path.display());
	Ok(())
}
