// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! HOP data pool implementation.

use crate::{
	hop::types::{HopError, HopPoolEntry, PoolStatus, MAX_DATA_SIZE},
	node_primitives::{BlockNumber, Hash},
};
use parking_lot::RwLock;
use sp_core::{hashing::blake2_256, H256};
use std::{
	collections::HashMap,
	sync::{
		atomic::{AtomicU64, Ordering},
		Arc,
	},
};

/// HOP data pool
pub struct HopDataPool {
	/// The actual data
	entries: Arc<RwLock<HashMap<Hash, HopPoolEntry>>>,
	/// Maximum pool size in bytes
	max_size: u64,
	/// Current pool size in bytes
	current_size: AtomicU64,
	/// Data retention period in blocks
	retention_blocks: u32,
}

impl HopDataPool {
	/// Create a new data pool
	pub fn new(max_size: u64, retention_blocks: u32) -> Result<Self, HopError> {
		Ok(Self {
			entries: Arc::new(RwLock::new(HashMap::new())),
			max_size,
			current_size: AtomicU64::new(0),
			retention_blocks,
		})
	}

	/// Insert data into the pool
	///
	/// Returns the hash of the data
	pub fn insert(&self, data: Vec<u8>, current_block: BlockNumber) -> Result<Hash, HopError> {
		// Validate data size
		if data.is_empty() {
			return Err(HopError::EmptyData);
		}

		let data_len = data.len() as u64;
		if data_len > MAX_DATA_SIZE {
			return Err(HopError::DataTooLarge(data.len(), MAX_DATA_SIZE));
		}

		// Check pool capacity
		let current_size = self.current_size.load(Ordering::Relaxed);
		if current_size + data_len > self.max_size {
			return Err(HopError::PoolFull(current_size, self.max_size));
		}

		let hash = H256(blake2_256(&data));

		// Check for duplicates
		{
			let entries = self.entries.read();
			if entries.contains_key(&hash) {
				return Err(HopError::DuplicateEntry);
			}
		}

		// Create entry and add it to the pool
		let entry = HopPoolEntry::new(data, current_block, self.retention_blocks);
		{
			let mut entries = self.entries.write();
			entries.insert(hash, entry);
		}

		// Update size counter
		self.current_size.fetch_add(data_len, Ordering::Relaxed);

		tracing::info!(
			target: "hop",
			hash = ?hex::encode(hash),
			size = data_len,
			expires_at = current_block + self.retention_blocks,
			"Data added to HOP pool"
		);

		Ok(hash)
	}

	/// Get data from the pool by content hash
	pub fn get(&self, hash: &Hash) -> Option<Vec<u8>> {
		let entries = self.entries.read();
		entries.get(hash).map(|entry| entry.data.clone())
	}

	/// Check if data exists in the pool
	pub fn has(&self, hash: &Hash) -> bool {
		let entries = self.entries.read();
		entries.contains_key(hash)
	}

	/// Remove data from the pool
	pub fn remove(&self, hash: &Hash) -> Result<(), HopError> {
		let entry = {
			let mut entries = self.entries.write();
			entries.remove(hash)
		};

		if let Some(entry) = entry {
			// Update size counter
			self.current_size.fetch_sub(entry.size, Ordering::Relaxed);

			tracing::debug!(
				target: "hop",
				hash = ?hex::encode(hash),
				"Data removed from pool"
			);

			Ok(())
		} else {
			Err(HopError::NotFound)
		}
	}

	/// Get pool status
	pub fn status(&self) -> PoolStatus {
		let entries = self.entries.read();
		PoolStatus {
			entry_count: entries.len(),
			total_bytes: self.current_size.load(Ordering::Relaxed),
			max_bytes: self.max_size,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn create_test_pool() -> HopDataPool {
		HopDataPool::new(1024 * 1024, 100).unwrap()
	}

	#[test]
	fn test_insert_and_get() {
		let pool = create_test_pool();
		let data = vec![1, 2, 3, 4, 5];
		let hash = pool.insert(data.clone(), 0).unwrap();

		let retrieved = pool.get(&hash).unwrap();
		assert_eq!(data, retrieved);
	}

	#[test]
	fn test_duplicate_insert() {
		let pool = create_test_pool();
		let data = vec![1, 2, 3, 4, 5];

		pool.insert(data.clone(), 0).unwrap();
		let result = pool.insert(data, 0);

		assert!(matches!(result, Err(HopError::DuplicateEntry)));
	}

	#[test]
	fn test_data_too_large() {
		let pool = create_test_pool();
		let data = vec![0u8; (MAX_DATA_SIZE + 1) as usize];

		let result = pool.insert(data, 0);
		assert!(matches!(result, Err(HopError::DataTooLarge(_, _))));
	}

	#[test]
	fn test_pool_full() {
		let pool = HopDataPool::new(100, 100).unwrap();

		let data1 = vec![0u8; 60];
		let data2 = vec![1u8; 50];

		pool.insert(data1, 0).unwrap();
		let result = pool.insert(data2, 0);

		assert!(matches!(result, Err(HopError::PoolFull(_, _))));
	}

	#[test]
	fn test_remove() {
		let pool = create_test_pool();
		let data = vec![1, 2, 3, 4, 5];
		let hash = pool.insert(data, 0).unwrap();

		assert!(pool.has(&hash));
		pool.remove(&hash).unwrap();
		assert!(!pool.has(&hash));
	}

	#[test]
	fn test_status() {
		let pool = create_test_pool();
		let data1 = vec![1, 2, 3, 4, 5];
		let data2 = vec![6, 7, 8];

		pool.insert(data1.clone(), 0).unwrap();
		pool.insert(data2.clone(), 0).unwrap();

		let status = pool.status();
		assert_eq!(status.entry_count, 2);
		assert_eq!(status.total_bytes, (data1.len() + data2.len()) as u64);
	}
}
