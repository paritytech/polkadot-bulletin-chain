// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Transaction submission for Bulletin Chain operations.
//!
//! This module provides the actual blockchain interaction layer using subxt.

use crate::{
	authorization::{Authorization, AuthorizationManager},
	cid::ContentHash,
	types::{AuthorizationScope, Error, ProgressCallback, ProgressEvent, Result, WaitFor},
};
use subxt::{blocks::BlockRef, utils::AccountId32, OnlineClient, PolkadotConfig};
use subxt_signer::sr25519::Keypair;

// Subxt metadata for TransactionStorage pallet
#[subxt::subxt(runtime_metadata_path = "../metadata.scale")]
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
			.map_err(|e| Error::NetworkError(format!("Failed to connect: {e:?}")))?;

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

	/// Query the current authorization for an account.
	///
	/// Returns `Some((transactions, bytes))` if authorization exists and is not expired,
	/// `None` if no authorization exists or it has expired.
	pub async fn query_account_authorization(
		&self,
		who: &AccountId32,
	) -> Result<Option<(u32, u64)>> {
		use bulletin::runtime_types::pallet_transaction_storage::AuthorizationScope as OnChainScope;

		let storage_query = bulletin::storage()
			.transaction_storage()
			.authorizations(OnChainScope::Account(who.clone()));

		let latest_block = self
			.api
			.blocks()
			.at_latest()
			.await
			.map_err(|e| Error::NetworkError(format!("Failed to get latest block: {e:?}")))?;

		let current_block_number = latest_block.number();

		let maybe_auth =
			latest_block.storage().fetch(&storage_query).await.map_err(|e| {
				Error::NetworkError(format!("Failed to query authorization: {e:?}"))
			})?;

		match maybe_auth {
			Some(auth) if auth.expiration > current_block_number =>
				Ok(Some((auth.extent.transactions, auth.extent.bytes))),
			Some(_) => Ok(None), // expired
			None => Ok(None),
		}
	}

	/// Check that sufficient authorization exists for a store operation.
	///
	/// Queries the chain for the account's current authorization and validates
	/// that it has enough transactions and bytes remaining.
	///
	/// This is a best-effort check — if the query fails (e.g., network error),
	/// the error is returned so the caller can decide whether to proceed.
	pub async fn check_authorization_for_store(
		&self,
		who: &AccountId32,
		required_transactions: u32,
		required_bytes: u64,
	) -> Result<()> {
		let auth_data = self.query_account_authorization(who).await?;

		match auth_data {
			Some((transactions, bytes)) => {
				let auth = Authorization {
					scope: AuthorizationScope::Account,
					transactions,
					max_size: bytes,
					expires_at: None, // already filtered out expired
				};
				let manager = AuthorizationManager::new();
				manager.check_authorization(&auth, required_bytes, required_transactions)
			},
			None => Err(Error::AuthorizationNotFound(format!("{who}"))),
		}
	}

	/// Store data on-chain.
	///
	/// Submits a `TransactionStorage.store` extrinsic.
	pub async fn store(
		&self,
		data: Vec<u8>,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<StoreReceipt> {
		self.store_with_progress(data, signer, wait_for, None).await
	}

	/// Store data on-chain with progress callbacks.
	///
	/// Submits a `TransactionStorage.store` extrinsic and emits progress
	/// events as the transaction moves through the network.
	///
	/// Before submitting, checks the account's on-chain authorization.
	/// Returns an error immediately if authorization is missing, expired,
	/// or insufficient (avoiding a wasted transaction submission).
	///
	/// The `wait_for` parameter controls when the method returns:
	/// - `WaitFor::InBlock` - Returns after transaction is included in a best block (faster)
	/// - `WaitFor::Finalized` - Returns after transaction is finalized (safer)
	///
	/// Progress events emitted:
	/// - `TransactionStatusEvent::Validated` - Transaction validated in pool
	/// - `TransactionStatusEvent::Broadcasted` - Transaction sent to peers
	/// - `TransactionStatusEvent::InBestBlock` - Transaction in a best block
	/// - `TransactionStatusEvent::Finalized` - Transaction finalized
	pub async fn store_with_progress(
		&self,
		data: Vec<u8>,
		signer: &Keypair,
		wait_for: WaitFor,
		progress_callback: Option<ProgressCallback>,
	) -> Result<StoreReceipt> {
		let data_size = data.len() as u64;

		// Authorization check before submission
		let account = AccountId32::from(signer.public_key().0);
		self.check_authorization_for_store(&account, 1, data_size).await?;

		let tx = bulletin::tx().transaction_storage().store(data);

		let mut progress = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&tx, signer)
			.await
			.map_err(|e| Error::StorageFailed(format!("Transaction submission failed: {e:?}")))?;

		let mut final_block_hash = None;
		let mut final_extrinsic_hash = None;

		// Stream transaction status events
		while let Some(status) = progress.next().await {
			match status {
				Ok(status) => {
					use subxt::tx::TxStatus;
					match status {
						TxStatus::Validated =>
							if let Some(ref callback) = progress_callback {
								callback(ProgressEvent::tx_validated());
							},
						TxStatus::Broadcasted =>
							if let Some(ref callback) = progress_callback {
								callback(ProgressEvent::tx_broadcasted());
							},
						TxStatus::InBestBlock(in_block) => {
							let block_hash = format!("{:?}", in_block.block_hash());
							let extrinsic_hash = format!("{:?}", in_block.extrinsic_hash());

							if let Some(ref callback) = progress_callback {
								// Only fetch block number when callback needs it
								let block_number =
									self.get_block_number(in_block.block_hash()).await.ok();
								callback(ProgressEvent::tx_in_best_block(
									block_hash.clone(),
									block_number,
									None, // extrinsic index not easily available here
								));
							}

							final_block_hash = Some(block_hash);
							final_extrinsic_hash = Some(extrinsic_hash);

							if wait_for == WaitFor::InBlock {
								in_block.wait_for_success().await.map_err(|e| {
									Error::TransactionFailed(format!("Transaction failed: {e:?}"))
								})?;
								break;
							}
						},
						TxStatus::InFinalizedBlock(in_block) => {
							let block_hash = format!("{:?}", in_block.block_hash());
							let extrinsic_hash = format!("{:?}", in_block.extrinsic_hash());

							// Only fetch block number when callback needs it
							let block_number = if progress_callback.is_some() {
								self.get_block_number(in_block.block_hash()).await.ok()
							} else {
								None
							};

							if let Some(ref callback) = progress_callback {
								callback(ProgressEvent::tx_finalized(
									block_hash.clone(),
									block_number,
									None,
								));
							}

							final_block_hash = Some(block_hash);
							final_extrinsic_hash = Some(extrinsic_hash);

							// Check for success
							in_block.wait_for_success().await.map_err(|e| {
								Error::TransactionFailed(format!("Transaction failed: {e:?}"))
							})?;

							break;
						},
						TxStatus::NoLongerInBestBlock => {
							if let Some(ref callback) = progress_callback {
								callback(ProgressEvent::Transaction(
									crate::types::TransactionStatusEvent::NoLongerInBestBlock,
								));
							}
						},
						TxStatus::Invalid { message } => {
							if let Some(ref callback) = progress_callback {
								callback(ProgressEvent::Transaction(
									crate::types::TransactionStatusEvent::Invalid {
										error: message.clone(),
									},
								));
							}
							return Err(Error::TransactionFailed(format!(
								"Transaction invalid: {message}"
							)));
						},
						TxStatus::Dropped { message } => {
							if let Some(ref callback) = progress_callback {
								callback(ProgressEvent::Transaction(
									crate::types::TransactionStatusEvent::Dropped {
										error: message.clone(),
									},
								));
							}
							return Err(Error::TransactionFailed(format!(
								"Transaction dropped: {message}"
							)));
						},
						TxStatus::Error { message } => {
							return Err(Error::TransactionFailed(format!(
								"Transaction error: {message}"
							)));
						},
					}
				},
				Err(e) => {
					return Err(Error::TransactionFailed(format!("Status error: {e:?}")));
				},
			}
		}

		Ok(StoreReceipt {
			block_hash: final_block_hash.unwrap_or_default(),
			extrinsic_hash: final_extrinsic_hash.unwrap_or_default(),
			data_size,
		})
	}

	/// Helper to get block number from block hash.
	async fn get_block_number<H: Into<BlockRef<subxt::config::substrate::H256>>>(
		&self,
		block_hash: H,
	) -> Result<u32> {
		let block = self
			.api
			.blocks()
			.at(block_hash)
			.await
			.map_err(|e| Error::NetworkError(format!("Failed to get block: {e:?}")))?;

		Ok(block.number())
	}

	/// Submit a transaction, wait for inclusion or finalization, and return the block hash.
	///
	/// The `wait_for` parameter controls when the method returns:
	/// - `WaitFor::InBlock` - Returns after the transaction is in a best block
	/// - `WaitFor::Finalized` - Returns after the transaction is finalized
	///
	/// The `make_error` closure maps a formatted message into the appropriate
	/// `Error` variant for the calling context (e.g. `Error::StorageFailed`,
	/// `Error::RenewalFailed`).
	async fn submit_and_wait(
		&self,
		tx: &impl subxt::tx::Payload,
		signer: &Keypair,
		context: &str,
		wait_for: WaitFor,
		make_error: impl Fn(String) -> Error,
	) -> Result<String> {
		let mut progress = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(tx, signer)
			.await
			.map_err(|e| make_error(format!("{context} failed: {e:?}")))?;

		match wait_for {
			WaitFor::Finalized => {
				let in_block = progress
					.wait_for_finalized()
					.await
					.map_err(|e| make_error(format!("{context} failed: {e:?}")))?;

				let block_hash = format!("{:?}", in_block.block_hash());

				in_block
					.wait_for_success()
					.await
					.map_err(|e| make_error(format!("{context} failed: {e:?}")))?;

				Ok(block_hash)
			},
			WaitFor::InBlock => {
				while let Some(status) = progress.next().await {
					match status {
						Ok(subxt::tx::TxStatus::InBestBlock(in_block)) => {
							let block_hash = format!("{:?}", in_block.block_hash());
							in_block
								.wait_for_success()
								.await
								.map_err(|e| make_error(format!("{context} failed: {e:?}")))?;
							return Ok(block_hash);
						},
						Ok(subxt::tx::TxStatus::Invalid { message }) =>
							return Err(make_error(format!(
								"{context} failed: transaction invalid: {message}"
							))),
						Ok(subxt::tx::TxStatus::Dropped { message }) =>
							return Err(make_error(format!(
								"{context} failed: transaction dropped: {message}"
							))),
						Ok(subxt::tx::TxStatus::Error { message }) =>
							return Err(make_error(format!(
								"{context} failed: transaction error: {message}"
							))),
						Err(e) => return Err(make_error(format!("{context} failed: {e:?}"))),
						_ => continue,
					}
				}
				Err(make_error(format!(
					"{context} failed: transaction stream ended without block inclusion"
				)))
			},
		}
	}

	/// Authorize an account to store data.
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn authorize_account(
		&self,
		who: AccountId32,
		transactions: u32,
		bytes: u64,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<AuthorizationReceipt> {
		let tx = bulletin::tx().transaction_storage().authorize_account(
			who.clone(),
			transactions,
			bytes,
		);

		let block_hash = self
			.submit_and_wait(&tx, signer, "Authorization", wait_for, Error::TransactionFailed)
			.await?;

		Ok(AuthorizationReceipt { account: who, transactions, bytes, block_hash })
	}

	/// Authorize a preimage (by content hash) to be stored.
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn authorize_preimage(
		&self,
		content_hash: ContentHash,
		max_size: u64,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<PreimageAuthorizationReceipt> {
		let tx = bulletin::tx().transaction_storage().authorize_preimage(content_hash, max_size);

		let block_hash = self
			.submit_and_wait(&tx, signer, "Authorization", wait_for, Error::TransactionFailed)
			.await?;

		Ok(PreimageAuthorizationReceipt { content_hash, max_size, block_hash })
	}

	/// Renew/extend the retention period for stored data.
	pub async fn renew(
		&self,
		block: u32,
		index: u32,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<RenewReceipt> {
		let tx = bulletin::tx().transaction_storage().renew(block, index);

		let block_hash = self
			.submit_and_wait(&tx, signer, "Renew", wait_for, Error::RenewalFailed)
			.await?;

		Ok(RenewReceipt { original_block: block, transaction_index: index, block_hash })
	}

	/// Refresh an account authorization (extends expiry).
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn refresh_account_authorization(
		&self,
		who: AccountId32,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<()> {
		let tx = bulletin::tx().transaction_storage().refresh_account_authorization(who);
		self.submit_and_wait(&tx, signer, "Refresh", wait_for, Error::TransactionFailed)
			.await?;
		Ok(())
	}

	/// Refresh a preimage authorization (extends expiry).
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn refresh_preimage_authorization(
		&self,
		content_hash: ContentHash,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<()> {
		let tx = bulletin::tx()
			.transaction_storage()
			.refresh_preimage_authorization(content_hash);
		self.submit_and_wait(&tx, signer, "Refresh", wait_for, Error::TransactionFailed)
			.await?;
		Ok(())
	}

	/// Remove an expired account authorization.
	pub async fn remove_expired_account_authorization(
		&self,
		who: AccountId32,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<()> {
		let tx = bulletin::tx().transaction_storage().remove_expired_account_authorization(who);
		self.submit_and_wait(&tx, signer, "Removal", wait_for, Error::TransactionFailed)
			.await?;
		Ok(())
	}

	/// Remove an expired preimage authorization.
	pub async fn remove_expired_preimage_authorization(
		&self,
		content_hash: ContentHash,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<()> {
		let tx = bulletin::tx()
			.transaction_storage()
			.remove_expired_preimage_authorization(content_hash);
		self.submit_and_wait(&tx, signer, "Removal", wait_for, Error::TransactionFailed)
			.await?;
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
