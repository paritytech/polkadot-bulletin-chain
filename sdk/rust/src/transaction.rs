// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Transaction submission for Bulletin Chain operations.
//!
//! This module provides the actual blockchain interaction layer using subxt.

#![cfg(feature = "std")]

use crate::{
	cid::ContentHash,
	types::{Error, Result},
};
use subxt::{
	OnlineClient, PolkadotConfig,
	tx::{PairSigner, TxPayload},
};
use sp_core::sr25519::Pair;
use sp_runtime::AccountId32;

// Subxt metadata for TransactionStorage pallet
#[subxt::subxt(runtime_metadata_path = "../metadata.scale", substitute_type_path = "sp_core::crypto::AccountId32")]
pub mod bulletin {}

/// Transaction submission client for Bulletin Chain.
///
/// This wraps a subxt OnlineClient and provides high-level methods
/// for all TransactionStorage pallet operations.
pub struct TransactionClient {
	api: OnlineClient<PolkadotConfig>,
}

impl TransactionClient {
	/// Create a new transaction client by connecting to the specified endpoint.
	pub async fn new(endpoint: &str) -> Result<Self> {
		let api = OnlineClient::<PolkadotConfig>::from_url(endpoint)
			.await
			.map_err(|e| Error::NetworkError(format!("Failed to connect: {:?}", e)))?;

		Ok(Self { api })
	}

	/// Create a transaction client from an existing subxt client.
	pub fn from_client(api: OnlineClient<PolkadotConfig>) -> Self {
		Self { api }
	}

	/// Get the underlying subxt client.
	pub fn api(&self) -> &OnlineClient<PolkadotConfig> {
		&self.api
	}

	/// Store data on-chain.
	///
	/// Submits a `TransactionStorage.store` extrinsic.
	pub async fn store(
		&self,
		data: Vec<u8>,
		signer: &PairSigner<PolkadotConfig, Pair>,
	) -> Result<StoreReceipt> {
		let tx = bulletin::tx()
			.transaction_storage()
			.store(data.clone());

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction submission failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction failed: {:?}", e)))?;

		let block_hash = result.block_hash();
		let extrinsic_hash = result.extrinsic_hash();

		Ok(StoreReceipt {
			block_hash: format!("{:?}", block_hash),
			extrinsic_hash: format!("{:?}", extrinsic_hash),
			data_size: data.len() as u64,
		})
	}

	/// Authorize an account to store data.
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn authorize_account(
		&self,
		who: AccountId32,
		transactions: u32,
		bytes: u64,
		signer: &PairSigner<PolkadotConfig, Pair>,
	) -> Result<AuthorizationReceipt> {
		let tx = bulletin::tx()
			.transaction_storage()
			.authorize_account(who.clone(), transactions, bytes);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Authorization failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Authorization failed: {:?}", e)))?;

		Ok(AuthorizationReceipt {
			account: who,
			transactions,
			bytes,
			block_hash: format!("{:?}", result.block_hash()),
		})
	}

	/// Authorize a preimage (by content hash) to be stored.
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn authorize_preimage(
		&self,
		content_hash: ContentHash,
		max_size: u64,
		signer: &PairSigner<PolkadotConfig, Pair>,
	) -> Result<PreimageAuthorizationReceipt> {
		let tx = bulletin::tx()
			.transaction_storage()
			.authorize_preimage(content_hash, max_size);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Authorization failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Authorization failed: {:?}", e)))?;

		Ok(PreimageAuthorizationReceipt {
			content_hash,
			max_size,
			block_hash: format!("{:?}", result.block_hash()),
		})
	}

	/// Renew/extend the retention period for stored data.
	pub async fn renew(
		&self,
		block: u32,
		index: u32,
		signer: &PairSigner<PolkadotConfig, Pair>,
	) -> Result<RenewReceipt> {
		let tx = bulletin::tx()
			.transaction_storage()
			.renew(block, index);

		let result = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Renew failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Renew failed: {:?}", e)))?;

		Ok(RenewReceipt {
			original_block: block,
			transaction_index: index,
			block_hash: format!("{:?}", result.block_hash()),
		})
	}

	/// Refresh an account authorization (extends expiry).
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn refresh_account_authorization(
		&self,
		who: AccountId32,
		signer: &PairSigner<PolkadotConfig, Pair>,
	) -> Result<()> {
		let tx = bulletin::tx()
			.transaction_storage()
			.refresh_account_authorization(who);

		self.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Refresh failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Refresh failed: {:?}", e)))?;

		Ok(())
	}

	/// Refresh a preimage authorization (extends expiry).
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn refresh_preimage_authorization(
		&self,
		content_hash: ContentHash,
		signer: &PairSigner<PolkadotConfig, Pair>,
	) -> Result<()> {
		let tx = bulletin::tx()
			.transaction_storage()
			.refresh_preimage_authorization(content_hash);

		self.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Refresh failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Refresh failed: {:?}", e)))?;

		Ok(())
	}

	/// Remove an expired account authorization.
	pub async fn remove_expired_account_authorization(
		&self,
		who: AccountId32,
		signer: &PairSigner<PolkadotConfig, Pair>,
	) -> Result<()> {
		let tx = bulletin::tx()
			.transaction_storage()
			.remove_expired_account_authorization(who);

		self.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Removal failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Removal failed: {:?}", e)))?;

		Ok(())
	}

	/// Remove an expired preimage authorization.
	pub async fn remove_expired_preimage_authorization(
		&self,
		content_hash: ContentHash,
		signer: &PairSigner<PolkadotConfig, Pair>,
	) -> Result<()> {
		let tx = bulletin::tx()
			.transaction_storage()
			.remove_expired_preimage_authorization(content_hash);

		self.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Removal failed: {:?}", e)))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| Error::StorageFailed(format!("Removal failed: {:?}", e)))?;

		Ok(())
	}
}

/// Receipt from a successful store operation.
#[derive(Debug, Clone)]
pub struct StoreReceipt {
	pub block_hash: String,
	pub extrinsic_hash: String,
	pub data_size: u64,
}

/// Receipt from a successful authorization.
#[derive(Debug, Clone)]
pub struct AuthorizationReceipt {
	pub account: AccountId32,
	pub transactions: u32,
	pub bytes: u64,
	pub block_hash: String,
}

/// Receipt from a successful preimage authorization.
#[derive(Debug, Clone)]
pub struct PreimageAuthorizationReceipt {
	pub content_hash: ContentHash,
	pub max_size: u64,
	pub block_hash: String,
}

/// Receipt from a successful renew operation.
#[derive(Debug, Clone)]
pub struct RenewReceipt {
	pub original_block: u32,
	pub transaction_index: u32,
	pub block_hash: String,
}
