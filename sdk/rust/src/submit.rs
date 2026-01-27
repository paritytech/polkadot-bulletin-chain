// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Transaction submission traits and implementations.
//!
//! This module provides traits for submitting transactions to Bulletin Chain.
//! Users can implement these traits with their preferred signing/submission method.

#![cfg(feature = "std")]

use crate::{cid::ContentHash, types::Result};
use alloc::vec::Vec;
use sp_runtime::AccountId32;

/// Trait for submitting transactions to Bulletin Chain.
///
/// Implement this trait to integrate with your preferred
/// signing and submission method (subxt, PAPI, etc.).
#[async_trait::async_trait]
pub trait TransactionSubmitter: Send + Sync {
	/// Submit a store transaction.
	async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt>;

	/// Submit an authorize_account transaction.
	async fn submit_authorize_account(
		&self,
		who: AccountId32,
		transactions: u32,
		bytes: u64,
	) -> Result<TransactionReceipt>;

	/// Submit an authorize_preimage transaction.
	async fn submit_authorize_preimage(
		&self,
		content_hash: ContentHash,
		max_size: u64,
	) -> Result<TransactionReceipt>;

	/// Submit a renew transaction.
	async fn submit_renew(&self, block: u32, index: u32) -> Result<TransactionReceipt>;

	/// Submit a refresh_account_authorization transaction.
	async fn submit_refresh_account_authorization(
		&self,
		who: AccountId32,
	) -> Result<TransactionReceipt>;

	/// Submit a refresh_preimage_authorization transaction.
	async fn submit_refresh_preimage_authorization(
		&self,
		content_hash: ContentHash,
	) -> Result<TransactionReceipt>;

	/// Submit a remove_expired_account_authorization transaction.
	async fn submit_remove_expired_account_authorization(
		&self,
		who: AccountId32,
	) -> Result<TransactionReceipt>;

	/// Submit a remove_expired_preimage_authorization transaction.
	async fn submit_remove_expired_preimage_authorization(
		&self,
		content_hash: ContentHash,
	) -> Result<TransactionReceipt>;
}

/// Receipt from a successful transaction.
#[derive(Debug, Clone)]
pub struct TransactionReceipt {
	/// Hash of the block containing the transaction.
	pub block_hash: String,
	/// Hash of the extrinsic.
	pub extrinsic_hash: String,
	/// Block number (if known).
	pub block_number: Option<u32>,
}

/// Helper to build transaction calls for manual submission.
///
/// Use this if you want to build the transaction payload yourself
/// and submit it using your own method.
pub struct TransactionBuilder;

impl TransactionBuilder {
	/// Build a store call.
	pub fn store(data: Vec<u8>) -> Call {
		Call::Store { data }
	}

	/// Build an authorize_account call.
	pub fn authorize_account(who: AccountId32, transactions: u32, bytes: u64) -> Call {
		Call::AuthorizeAccount { who, transactions, bytes }
	}

	/// Build an authorize_preimage call.
	pub fn authorize_preimage(content_hash: ContentHash, max_size: u64) -> Call {
		Call::AuthorizePreimage { content_hash, max_size }
	}

	/// Build a renew call.
	pub fn renew(block: u32, index: u32) -> Call {
		Call::Renew { block, index }
	}

	/// Build a refresh_account_authorization call.
	pub fn refresh_account_authorization(who: AccountId32) -> Call {
		Call::RefreshAccountAuthorization { who }
	}

	/// Build a refresh_preimage_authorization call.
	pub fn refresh_preimage_authorization(content_hash: ContentHash) -> Call {
		Call::RefreshPreimageAuthorization { content_hash }
	}

	/// Build a remove_expired_account_authorization call.
	pub fn remove_expired_account_authorization(who: AccountId32) -> Call {
		Call::RemoveExpiredAccountAuthorization { who }
	}

	/// Build a remove_expired_preimage_authorization call.
	pub fn remove_expired_preimage_authorization(content_hash: ContentHash) -> Call {
		Call::RemoveExpiredPreimageAuthorization { content_hash }
	}
}

/// A TransactionStorage pallet call.
#[derive(Debug, Clone)]
pub enum Call {
	Store { data: Vec<u8> },
	Renew { block: u32, index: u32 },
	AuthorizeAccount { who: AccountId32, transactions: u32, bytes: u64 },
	AuthorizePreimage { content_hash: ContentHash, max_size: u64 },
	RefreshAccountAuthorization { who: AccountId32 },
	RefreshPreimageAuthorization { content_hash: ContentHash },
	RemoveExpiredAccountAuthorization { who: AccountId32 },
	RemoveExpiredPreimageAuthorization { content_hash: ContentHash },
}

impl Call {
	/// Get the call name.
	pub fn name(&self) -> &'static str {
		match self {
			Call::Store { .. } => "store",
			Call::Renew { .. } => "renew",
			Call::AuthorizeAccount { .. } => "authorize_account",
			Call::AuthorizePreimage { .. } => "authorize_preimage",
			Call::RefreshAccountAuthorization { .. } => "refresh_account_authorization",
			Call::RefreshPreimageAuthorization { .. } => "refresh_preimage_authorization",
			Call::RemoveExpiredAccountAuthorization { .. } =>
				"remove_expired_account_authorization",
			Call::RemoveExpiredPreimageAuthorization { .. } =>
				"remove_expired_preimage_authorization",
		}
	}

	/// Get the pallet name.
	pub fn pallet(&self) -> &'static str {
		"TransactionStorage"
	}
}
