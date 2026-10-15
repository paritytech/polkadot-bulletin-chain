// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Authorization management for storing data on Bulletin Chain.
//!
//! The Bulletin Chain supports two authorization models:
//! 1. **Account-based**: Authorize an account to store N transactions with max total size
//! 2. **Preimage-based**: Authorize anyone to store a specific preimage (by content hash)

use crate::types::AuthorizationScope;

/// Client-side authorization settings and requirement estimates.
///
/// Note: This is a client-side helper. Actual authorization calls
/// must be made through the blockchain runtime using `authorize_account`
/// or `authorize_preimage` extrinsics.
#[derive(Debug, Clone)]
pub struct AuthorizationManager {
	/// Whether to use account or preimage authorization by default.
	pub default_scope: AuthorizationScope,
	/// Auto-refresh authorization when it's close to expiring.
	pub auto_refresh: bool,
}

impl Default for AuthorizationManager {
	fn default() -> Self {
		Self { default_scope: AuthorizationScope::Account, auto_refresh: false }
	}
}

impl AuthorizationManager {
	/// Create a new authorization manager with default settings.
	pub fn new() -> Self {
		Self::default()
	}

	/// Create an authorization manager with account-based authorization.
	pub fn with_account_auth() -> Self {
		Self { default_scope: AuthorizationScope::Account, auto_refresh: false }
	}

	/// Create an authorization manager with preimage-based authorization.
	pub fn with_preimage_auth() -> Self {
		Self { default_scope: AuthorizationScope::Preimage, auto_refresh: false }
	}

	/// Enable auto-refresh of authorizations.
	pub fn with_auto_refresh(mut self, enabled: bool) -> Self {
		self.auto_refresh = enabled;
		self
	}

	/// Calculate authorization requirements for storing chunked data.
	pub fn calculate_requirements(
		&self,
		total_size: u64,
		num_chunks: usize,
		include_manifest: bool,
	) -> (u32, u64) {
		// Each chunk requires one transaction (saturate to u32::MAX if overflow)
		let mut transactions = u32::try_from(num_chunks).unwrap_or(u32::MAX);
		let mut total_bytes = total_size;

		// If creating a manifest, add one more transaction
		if include_manifest {
			transactions = transactions.saturating_add(1);
			// Manifest is typically small, estimate ~1KB per 100 chunks
			let manifest_estimate = (num_chunks as u64).saturating_mul(10).saturating_add(1000);
			total_bytes = total_bytes.saturating_add(manifest_estimate);
		}

		(transactions, total_bytes)
	}

	/// Estimate authorization needed for storing data of given size.
	///
	/// This calculates based on default chunk size (1 MiB).
	pub fn estimate_authorization(&self, data_size: u64, create_manifest: bool) -> (u32, u64) {
		use crate::chunker::DEFAULT_CHUNK_SIZE;

		let num_chunks =
			if data_size == 0 { 1 } else { data_size.div_ceil(DEFAULT_CHUNK_SIZE as u64) as usize };

		self.calculate_requirements(data_size, num_chunks, create_manifest)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_calculate_requirements() {
		let manager = AuthorizationManager::new();

		// 5 chunks, no manifest
		let (txs, bytes) = manager.calculate_requirements(5_000_000, 5, false);
		assert_eq!(txs, 5);
		assert_eq!(bytes, 5_000_000);

		// 5 chunks, with manifest
		let (txs, bytes) = manager.calculate_requirements(5_000_000, 5, true);
		assert_eq!(txs, 6);
		assert!(bytes > 5_000_000);
	}

	#[test]
	fn test_estimate_authorization() {
		let manager = AuthorizationManager::new();

		// 10 MB data = 10 chunks (1 MB each)
		let (txs, bytes) = manager.estimate_authorization(10_000_000, false);
		assert_eq!(txs, 10);

		// With manifest
		let (txs_with_manifest, bytes_with_manifest) =
			manager.estimate_authorization(10_000_000, true);
		assert_eq!(txs_with_manifest, 11);
		assert!(bytes_with_manifest > bytes);
	}

	#[test]
	fn test_default_scope() {
		let manager = AuthorizationManager::new();
		assert!(matches!(manager.default_scope, AuthorizationScope::Account));

		let account_manager = AuthorizationManager::with_account_auth();
		assert!(matches!(account_manager.default_scope, AuthorizationScope::Account));

		let preimage_manager = AuthorizationManager::with_preimage_auth();
		assert!(matches!(preimage_manager.default_scope, AuthorizationScope::Preimage));
	}

	#[test]
	fn test_auto_refresh() {
		let manager = AuthorizationManager::new().with_auto_refresh(true);
		assert!(manager.auto_refresh);
	}

	#[test]
	fn test_calculate_requirements_overflow_protection() {
		let manager = AuthorizationManager::new();

		// Very large number of chunks that would overflow u32
		let (txs, bytes) = manager.calculate_requirements(u64::MAX, usize::MAX, true);

		// Should saturate instead of overflow
		assert_eq!(txs, u32::MAX);
		assert!(bytes > 0); // Should have saturated but not wrapped to zero
	}

	#[test]
	fn test_estimate_authorization_zero_size() {
		let manager = AuthorizationManager::new();

		// Zero size should still require 1 transaction
		let (txs, _bytes) = manager.estimate_authorization(0, false);
		assert_eq!(txs, 1);
	}
}
