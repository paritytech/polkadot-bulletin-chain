// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Renewal operations for extending data retention.
//!
//! Data stored on Bulletin Chain has a retention period after which it may be pruned.
//! Use the renewal functionality to extend the retention period before expiration.
//!
//! # Example
//!
//! ```rust,ignore
//! use bulletin_sdk_rust::prelude::*;
//!
//! // After storing data, you receive block number and index
//! let storage_ref = StorageRef::new(block_number, index);
//!
//! // Prepare renewal
//! let client = BulletinClient::new();
//! let operation = client.prepare_renew(storage_ref)?;
//!
//! // Submit via subxt:
//! // api.tx().transaction_storage().renew(operation.block, operation.index)
//! ```

extern crate alloc;

use crate::types::{Error, Result, StorageRef};
use codec::{Decode, Encode};
use scale_info::TypeInfo;

/// A renewal operation ready for submission.
///
/// This contains the parameters needed to call `transactionStorage.renew()`.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct RenewalOperation {
	/// Block number containing the original storage or previous renewal.
	pub block: u32,
	/// Transaction index within the block.
	pub index: u32,
}

impl RenewalOperation {
	/// Create a new renewal operation from a storage reference.
	pub fn new(storage_ref: StorageRef) -> Self {
		Self { block: storage_ref.block, index: storage_ref.index }
	}

	/// Create from raw block and index values.
	pub fn from_raw(block: u32, index: u32) -> Self {
		Self { block, index }
	}

	/// Validate the renewal operation.
	pub fn validate(&self) -> Result<()> {
		// Block 0 is typically genesis and unlikely to have user transactions
		if self.block == 0 {
			return Err(Error::RenewalFailed(
				"Block 0 is not a valid renewal target".into(),
			));
		}
		Ok(())
	}

	/// Get the storage reference for this operation.
	pub fn storage_ref(&self) -> StorageRef {
		StorageRef::new(self.block, self.index)
	}
}

/// Tracker for managing multiple storage references that need renewal.
///
/// Use this to keep track of stored data and their expiration.
#[derive(Debug, Clone, Default)]
pub struct RenewalTracker {
	/// Tracked storage references with their expiration block.
	entries: alloc::vec::Vec<TrackedEntry>,
}

/// A tracked entry for renewal.
#[derive(Debug, Clone, Encode, Decode, TypeInfo)]
pub struct TrackedEntry {
	/// Storage reference (block and index).
	pub storage_ref: StorageRef,
	/// Content hash of the data.
	pub content_hash: alloc::vec::Vec<u8>,
	/// Size of the data in bytes.
	pub size: u64,
	/// Block number when this entry expires (needs renewal before this).
	pub expires_at: u32,
}

impl RenewalTracker {
	/// Create a new renewal tracker.
	pub fn new() -> Self {
		Self { entries: alloc::vec::Vec::new() }
	}

	/// Track a new storage entry.
	///
	/// # Arguments
	/// * `storage_ref` - Reference to the stored transaction
	/// * `content_hash` - Hash of the stored content
	/// * `size` - Size of the stored data
	/// * `retention_period` - Number of blocks until expiration
	pub fn track(
		&mut self,
		storage_ref: StorageRef,
		content_hash: alloc::vec::Vec<u8>,
		size: u64,
		retention_period: u32,
	) {
		let expires_at = storage_ref.block.saturating_add(retention_period);
		self.entries.push(TrackedEntry { storage_ref, content_hash, size, expires_at });
	}

	/// Update a tracked entry after renewal.
	///
	/// Call this after a successful renewal to update the storage reference
	/// and expiration block.
	pub fn update_after_renewal(
		&mut self,
		old_ref: StorageRef,
		new_ref: StorageRef,
		retention_period: u32,
	) -> bool {
		for entry in &mut self.entries {
			if entry.storage_ref.block == old_ref.block
				&& entry.storage_ref.index == old_ref.index
			{
				entry.storage_ref = new_ref;
				entry.expires_at = new_ref.block.saturating_add(retention_period);
				return true;
			}
		}
		false
	}

	/// Get entries that expire before a given block.
	pub fn expiring_before(&self, block: u32) -> alloc::vec::Vec<&TrackedEntry> {
		self.entries.iter().filter(|e| e.expires_at <= block).collect()
	}

	/// Get all tracked entries.
	pub fn entries(&self) -> &[TrackedEntry] {
		&self.entries
	}

	/// Remove an entry by content hash.
	pub fn remove_by_content_hash(&mut self, content_hash: &[u8]) -> bool {
		let initial_len = self.entries.len();
		self.entries.retain(|e| e.content_hash != content_hash);
		self.entries.len() != initial_len
	}

	/// Clear all tracked entries.
	pub fn clear(&mut self) {
		self.entries.clear();
	}

	/// Number of tracked entries.
	pub fn len(&self) -> usize {
		self.entries.len()
	}

	/// Check if tracker is empty.
	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_renewal_operation_new() {
		let storage_ref = StorageRef::new(100, 5);
		let op = RenewalOperation::new(storage_ref);
		assert_eq!(op.block, 100);
		assert_eq!(op.index, 5);
	}

	#[test]
	fn test_renewal_operation_validate() {
		let valid = RenewalOperation::from_raw(100, 0);
		assert!(valid.validate().is_ok());

		let invalid = RenewalOperation::from_raw(0, 0);
		assert!(invalid.validate().is_err());
	}

	#[test]
	fn test_tracker_track_and_query() {
		let mut tracker = RenewalTracker::new();

		tracker.track(StorageRef::new(100, 0), vec![1, 2, 3], 1000, 500);
		tracker.track(StorageRef::new(200, 1), vec![4, 5, 6], 2000, 500);

		assert_eq!(tracker.len(), 2);

		// First entry expires at 600
		let expiring = tracker.expiring_before(650);
		assert_eq!(expiring.len(), 1);
		assert_eq!(expiring[0].storage_ref.block, 100);

		// Both expire before 800
		let expiring = tracker.expiring_before(800);
		assert_eq!(expiring.len(), 2);
	}

	#[test]
	fn test_tracker_update_after_renewal() {
		let mut tracker = RenewalTracker::new();
		tracker.track(StorageRef::new(100, 0), vec![1, 2, 3], 1000, 500);

		let updated =
			tracker.update_after_renewal(StorageRef::new(100, 0), StorageRef::new(700, 2), 500);
		assert!(updated);

		let entry = &tracker.entries()[0];
		assert_eq!(entry.storage_ref.block, 700);
		assert_eq!(entry.storage_ref.index, 2);
		assert_eq!(entry.expires_at, 1200);
	}

	#[test]
	fn test_tracker_remove() {
		let mut tracker = RenewalTracker::new();
		tracker.track(StorageRef::new(100, 0), vec![1, 2, 3], 1000, 500);
		tracker.track(StorageRef::new(200, 1), vec![4, 5, 6], 2000, 500);

		assert_eq!(tracker.len(), 2);

		let removed = tracker.remove_by_content_hash(&[1, 2, 3]);
		assert!(removed);
		assert_eq!(tracker.len(), 1);
	}
}
