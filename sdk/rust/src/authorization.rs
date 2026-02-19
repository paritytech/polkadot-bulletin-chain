// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Authorization management for storing data on Bulletin Chain.
//!
//! The Bulletin Chain supports two authorization models:
//! 1. **Account-based**: Authorize an account to store N transactions with max total size
//! 2. **Preimage-based**: Authorize anyone to store a specific preimage (by content hash)

extern crate alloc;

use crate::types::{AuthorizationScope, Error, Result};

/// Authorization information.
#[derive(Debug, Clone)]
pub struct Authorization {
	/// The authorization scope.
	pub scope: AuthorizationScope,
	/// Number of transactions authorized.
	pub transactions: u32,
	/// Maximum total size in bytes.
	pub max_size: u64,
	/// Block number when authorization expires (if known).
	pub expires_at: Option<u32>,
}

/// Authorization manager for checking and tracking authorization state.
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

	/// Check if sufficient authorization exists for storing data.
	///
	/// This is a client-side helper. The actual authorization check
	/// is performed on-chain when submitting the transaction.
	pub fn check_authorization(
		&self,
		available: &Authorization,
		required_size: u64,
		num_transactions: u32,
	) -> Result<()> {
		if available.transactions < num_transactions {
			return Err(Error::InsufficientAuthorization {
				need: num_transactions as u64,
				available: available.transactions as u64,
			});
		}

		if available.max_size < required_size {
			return Err(Error::InsufficientAuthorization {
				need: required_size,
				available: available.max_size,
			});
		}

		Ok(())
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

/// Helper functions for authorization (requires std for subxt integration).
#[cfg(feature = "std")]
pub mod helpers {
	use super::*;

	/// Build authorization request parameters for account-based auth.
	///
	/// Returns: (transactions, max_size)
	pub fn build_account_auth_params(
		data_size: u64,
		num_chunks: usize,
		include_manifest: bool,
	) -> (u32, u64) {
		let manager = AuthorizationManager::new();
		manager.calculate_requirements(data_size, num_chunks, include_manifest)
	}

	/// Build authorization request parameters for preimage-based auth.
	///
	/// Returns: (content_hash, max_size)
	pub fn build_preimage_auth_params(content_hash: [u8; 32], data_size: u64) -> ([u8; 32], u64) {
		(content_hash, data_size)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_check_authorization_sufficient() {
		let manager = AuthorizationManager::new();
		let auth = Authorization {
			scope: AuthorizationScope::Account,
			transactions: 10,
			max_size: 10_000_000,
			expires_at: None,
		};

		let result = manager.check_authorization(&auth, 5_000_000, 5);
		assert!(result.is_ok());
	}

	#[test]
	fn test_check_authorization_insufficient_transactions() {
		let manager = AuthorizationManager::new();
		let auth = Authorization {
			scope: AuthorizationScope::Account,
			transactions: 5,
			max_size: 10_000_000,
			expires_at: None,
		};

		let result = manager.check_authorization(&auth, 5_000_000, 10);
		assert!(result.is_err());
	}

	#[test]
	fn test_check_authorization_insufficient_size() {
		let manager = AuthorizationManager::new();
		let auth = Authorization {
			scope: AuthorizationScope::Account,
			transactions: 10,
			max_size: 1_000_000,
			expires_at: None,
		};

		let result = manager.check_authorization(&auth, 5_000_000, 5);
		assert!(result.is_err());
	}

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
}
