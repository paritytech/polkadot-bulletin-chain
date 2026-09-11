// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Transaction submission for Bulletin Chain operations.
//!
//! This module provides the actual blockchain interaction layer using subxt.

use crate::{
	authorization::{Authorization, AuthorizationManager},
	cid::ContentHash,
	types::{AuthorizationScope, Error, ProgressCallback, ProgressEvent, Result, WaitFor},
};
use bulletin_transaction_storage_primitives::TransactionRef;
use subxt::{
	blocks::BlockRef, config::DefaultExtrinsicParamsBuilder, utils::AccountId32, OnlineClient,
	PolkadotConfig,
};
use subxt_signer::sr25519::Keypair;

// Subxt metadata for TransactionStorage pallet
#[subxt::subxt(runtime_metadata_path = "../metadata.scale")]
pub mod bulletin {}

/// Runtime name of the pallet holding the renewal extrinsics. Absent on chains that ship
/// without `pallet-bulletin-data-renewal`.
const RENEWAL_PALLET: &str = "DataRenewal";

/// Runtime API exposing the authorization summary, and the method on it that
/// [`TransactionClient::query_account_authorization`] calls. Absent on runtimes that predate them.
const AUTHORIZATION_API: &str = "BulletinTransactionStorageApi";
const AUTHORIZATION_METHOD: &str = "account_authorization";

/// Convert the primitives `TransactionRef` into the subxt-generated one.
fn to_runtime_ref(
	entry: &TransactionRef<u32>,
) -> bulletin::runtime_types::bulletin_transaction_storage_primitives::TransactionRef<u32> {
	use bulletin::runtime_types::bulletin_transaction_storage_primitives::TransactionRef as Gen;
	match entry {
		TransactionRef::Position { block, index } => Gen::Position { block: *block, index: *index },
		TransactionRef::ContentHash(hash) => Gen::ContentHash(*hash),
	}
}

/// Transaction submission client for Bulletin Chain.
///
/// This wraps a subxt OnlineClient and provides high-level methods
/// for all TransactionStorage pallet operations.
///
/// Resolves nonces from the best block (not finalized) before each
/// submission, matching PAPI's approach. This ensures sequential
/// transactions work correctly with `WaitFor::InBlock`.
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

	/// Submit a transaction with the nonce resolved from the best block.
	///
	/// Subxt's default nonce resolution queries the finalized block which
	/// lags behind actual block inclusion for sequential `WaitFor::InBlock` transactions. So we
	/// manually override that by choosing to query at the current best block instead.
	async fn submit_at_best_block(
		&self,
		tx: &impl subxt::tx::Payload,
		signer: &Keypair,
		make_error: &impl Fn(String) -> Error,
	) -> Result<subxt::tx::TxProgress<PolkadotConfig, OnlineClient<PolkadotConfig>>> {
		let account = AccountId32::from(signer.public_key().0);

		// Get the best block and query nonce at that block
		let best_block = self
			.api
			.blocks()
			.subscribe_best()
			.await
			.map_err(|e| make_error(format!("{e:?}")))?
			.next()
			.await
			.ok_or_else(|| make_error("Best block stream ended".into()))?
			.map_err(|e| make_error(format!("{e:?}")))?;

		let nonce = best_block
			.account_nonce(&account)
			.await
			.map_err(|e| make_error(format!("{e:?}")))?;

		let params = DefaultExtrinsicParamsBuilder::<PolkadotConfig>::new().nonce(nonce).build();
		self.api
			.tx()
			.sign_and_submit_then_watch(tx, signer, params)
			.await
			.map_err(|e| make_error(format!("{e:?}")))
	}

	/// Submit a transaction, stream status events, and return when the target
	/// confirmation level is reached.
	///
	/// This is the single submission loop used by all public methods. It:
	/// - Streams all `TxStatus` events, firing the optional progress callback
	/// - Breaks on `InBestBlock` or `InFinalizedBlock` depending on the value of `wait_for`
	/// - Returns block hash and extrinsic hash on success
	async fn submit_and_watch(
		&self,
		tx: &impl subxt::tx::Payload,
		signer: &Keypair,
		wait_for: WaitFor,
		progress_callback: Option<ProgressCallback>,
		make_error: impl Fn(String) -> Error,
	) -> Result<SubmitResult> {
		let mut progress = self.submit_at_best_block(tx, signer, &make_error).await?;

		let mut result_block_hash = None;
		let mut result_extrinsic_hash = None;

		while let Some(status) = progress.next().await {
			match status {
				Ok(status) => {
					use subxt::tx::TxStatus;
					match status {
						TxStatus::Validated =>
							if let Some(ref cb) = progress_callback {
								cb(ProgressEvent::tx_validated());
							},
						TxStatus::Broadcasted =>
							if let Some(ref cb) = progress_callback {
								cb(ProgressEvent::tx_broadcasted());
							},
						TxStatus::InBestBlock(in_block) => {
							let block_hash = in_block.block_hash().to_string();
							let extrinsic_hash = in_block.extrinsic_hash().to_string();

							if let Some(ref cb) = progress_callback {
								let block_number =
									self.get_block_number(in_block.block_hash()).await.ok();
								cb(ProgressEvent::tx_in_best_block(
									block_hash.clone(),
									block_number,
									None,
								));
							}

							result_block_hash = Some(block_hash);
							result_extrinsic_hash = Some(extrinsic_hash);

							if wait_for == WaitFor::InBlock {
								in_block.wait_for_success().await.map_err(|e| {
									make_error(format!("Transaction failed: {e:?}"))
								})?;
								break;
							}
						},
						TxStatus::InFinalizedBlock(in_block) => {
							let block_hash = in_block.block_hash().to_string();
							let extrinsic_hash = in_block.extrinsic_hash().to_string();

							if let Some(ref cb) = progress_callback {
								let block_number =
									self.get_block_number(in_block.block_hash()).await.ok();
								cb(ProgressEvent::tx_finalized(
									block_hash.clone(),
									block_number,
									None,
								));
							}

							result_block_hash = Some(block_hash);
							result_extrinsic_hash = Some(extrinsic_hash);

							in_block
								.wait_for_success()
								.await
								.map_err(|e| make_error(format!("Transaction failed: {e:?}")))?;
							break;
						},
						TxStatus::NoLongerInBestBlock =>
							if let Some(ref cb) = progress_callback {
								cb(ProgressEvent::Transaction(
									crate::types::TransactionStatusEvent::NoLongerInBestBlock,
								));
							},
						TxStatus::Invalid { message } => {
							if let Some(ref cb) = progress_callback {
								cb(ProgressEvent::Transaction(
									crate::types::TransactionStatusEvent::Invalid {
										error: message.clone(),
									},
								));
							}
							return Err(make_error(format!("Transaction invalid: {message}")));
						},
						TxStatus::Dropped { message } => {
							if let Some(ref cb) = progress_callback {
								cb(ProgressEvent::Transaction(
									crate::types::TransactionStatusEvent::Dropped {
										error: message.clone(),
									},
								));
							}
							return Err(make_error(format!("Transaction dropped: {message}")));
						},
						TxStatus::Error { message } =>
							return Err(make_error(format!("Transaction error: {message}"))),
					}
				},
				Err(e) => return Err(make_error(format!("Status error: {e:?}"))),
			}
		}

		match (result_block_hash, result_extrinsic_hash) {
			(Some(block_hash), Some(extrinsic_hash)) =>
				Ok(SubmitResult { block_hash, extrinsic_hash }),
			_ => Err(make_error("Transaction stream ended without block inclusion".into())),
		}
	}

	/// Query the current authorization for an account and return the remaining boost-tier
	/// capacity as `(transactions_remaining, bytes_remaining)`.
	///
	/// `bytes` and `transactions` on `AuthorizationExtent` are *consumed* counters; this
	/// helper subtracts them from the granted caps (saturating to `0` if the holder is
	/// already over-cap on either axis). Note that overshooting the caps no longer rejects
	/// the transaction — it only forfeits the priority boost — so the returned remaining
	/// values are a soft-budget preflight, not a hard precondition.
	///
	/// Returns `None` if no authorization exists or it has expired, and
	/// [`Error::AuthorizationApiUnavailable`] on a runtime without [`AUTHORIZATION_API`].
	pub async fn query_account_authorization(
		&self,
		who: &AccountId32,
	) -> Result<Option<(u32, u64)>> {
		self.ensure_authorization_api_available()?;

		let call = bulletin::apis()
			.bulletin_transaction_storage_api()
			.account_authorization(who.clone());

		let maybe_auth = self
			.api
			.runtime_api()
			.at_latest()
			.await
			.map_err(|e| Error::NetworkError(format!("Failed to get latest block: {e:?}")))?
			.call(call)
			.await
			.map_err(|e| Error::NetworkError(format!("Failed to query authorization: {e:?}")))?;

		// The runtime API already filters out expired authorizations.
		let Some(auth) = maybe_auth else { return Ok(None) };

		let transactions_remaining =
			auth.transactions_allowance.saturating_sub(auth.transactions_used);
		// `bytes_permanent_used` is checked against `bytes_allowance` on its own and never adds to
		// `bytes_used`, so a `store` preflight looks at `bytes_used` alone.
		let bytes_remaining = auth.bytes_allowance.saturating_sub(auth.bytes_used);
		Ok(Some((transactions_remaining, bytes_remaining)))
	}

	/// Reject up front on a runtime that predates [`AUTHORIZATION_API`], where the call would
	/// otherwise fail with subxt's metadata-validation error instead of the actual reason.
	///
	/// Checks the method rather than just the trait: a runtime can carry the trait at an older
	/// version that does not yet declare [`AUTHORIZATION_METHOD`].
	fn ensure_authorization_api_available(&self) -> Result<()> {
		let has_method = self
			.api
			.metadata()
			.runtime_api_trait_by_name(AUTHORIZATION_API)
			.is_some_and(|api| api.method_by_name(AUTHORIZATION_METHOD).is_some());

		if !has_method {
			return Err(Error::AuthorizationApiUnavailable);
		}
		Ok(())
	}

	/// Check that sufficient authorization exists for a store operation.
	///
	/// Queries the chain for the account's current authorization and validates
	/// that it has enough transactions and bytes remaining for the boost tier.
	///
	/// This is a soft preflight: under the soft-cap design, on-chain validation
	/// no longer rejects a `store` for being over-budget — it only drops the
	/// priority boost. Use this check to decide whether to send (likely-boosted)
	/// or to top up the authorization first; do not treat a failure here as
	/// "the chain will reject this tx".
	///
	/// If the query itself fails (e.g., network error), the error is returned
	/// so the caller can decide whether to proceed.
	///
	/// A runtime without [`AUTHORIZATION_API`] is the one exception: there is nothing to
	/// preflight against, and a soft check must not block a `store` the chain would accept,
	/// so the check passes. Matches the TypeScript SDK, which also proceeds when it cannot
	/// read the authorization.
	pub async fn check_authorization_for_store(
		&self,
		who: &AccountId32,
		required_transactions: u32,
		required_bytes: u64,
	) -> Result<()> {
		let auth_data = match self.query_account_authorization(who).await {
			Ok(auth_data) => auth_data,
			Err(Error::AuthorizationApiUnavailable) => return Ok(()),
			Err(e) => return Err(e),
		};

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
		let result = self
			.submit_and_watch(&tx, signer, wait_for, progress_callback, |e| {
				Error::StorageFailed(format!("Store failed: {e}"))
			})
			.await?;

		Ok(StoreReceipt {
			block_hash: result.block_hash,
			extrinsic_hash: result.extrinsic_hash,
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

		let result = self
			.submit_and_watch(&tx, signer, wait_for, None, |e| {
				Error::TransactionFailed(format!("Authorization failed: {e}"))
			})
			.await?;

		Ok(AuthorizationReceipt {
			account: who,
			transactions,
			bytes,
			block_hash: result.block_hash,
		})
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

		let result = self
			.submit_and_watch(&tx, signer, wait_for, None, |e| {
				Error::TransactionFailed(format!("Authorization failed: {e}"))
			})
			.await?;

		Ok(PreimageAuthorizationReceipt { content_hash, max_size, block_hash: result.block_hash })
	}

	/// Reject renewal calls up front on a chain that has no renewal pallet.
	///
	/// The generated calls are built from `sdk/metadata.scale`, which comes from a runtime
	/// that wires `pallet-bulletin-data-renewal`. A runtime that omits it has no
	/// [`RENEWAL_PALLET`] at all, and submitting would surface subxt's metadata-validation
	/// error instead of the actual reason.
	fn ensure_renewal_available(&self) -> Result<()> {
		if self.api.metadata().pallet_by_name(RENEWAL_PALLET).is_none() {
			return Err(Error::RenewalUnavailable);
		}
		Ok(())
	}

	/// Schedule a one-shot renewal of stored data.
	///
	/// The renewal fires once when the data reaches its retention boundary; it
	/// does not renew synchronously. For immediate renewal use
	/// [`force_renew`](Self::force_renew).
	///
	/// Returns [`Error::RenewalUnavailable`] on a chain without the renewal pallet.
	///
	/// `entry` accepts anything convertible to [`TransactionRef`]: a
	/// `(block, index)` tuple or a [`ContentHash`].
	pub async fn renew(
		&self,
		entry: impl Into<TransactionRef<u32>>,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<RenewReceipt> {
		self.ensure_renewal_available()?;
		let entry = entry.into();
		let tx = bulletin::tx().data_renewal().renew(to_runtime_ref(&entry));

		let result = self
			.submit_and_watch(&tx, signer, wait_for, None, |e| {
				Error::RenewalFailed(format!("Renew failed: {e}"))
			})
			.await?;

		Ok(RenewReceipt { entry, block_hash: result.block_hash })
	}

	/// Immediately renew stored data, extending its retention from the current block.
	///
	/// `entry` accepts anything convertible to [`TransactionRef`]: a
	/// `(block, index)` tuple or a [`ContentHash`].
	///
	/// Returns [`Error::RenewalUnavailable`] on a chain without the renewal pallet.
	pub async fn force_renew(
		&self,
		entry: impl Into<TransactionRef<u32>>,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<RenewReceipt> {
		self.ensure_renewal_available()?;
		let entry = entry.into();
		let tx = bulletin::tx().data_renewal().force_renew(to_runtime_ref(&entry));

		let result = self
			.submit_and_watch(&tx, signer, wait_for, None, |e| {
				Error::RenewalFailed(format!("Force renew failed: {e}"))
			})
			.await?;

		Ok(RenewReceipt { entry, block_hash: result.block_hash })
	}

	/// Register recurring auto-renewal for stored content.
	///
	/// The first cycle is prepaid at registration; each later cycle charges
	/// the owner's authorization when it fires.
	///
	/// Returns [`Error::RenewalUnavailable`] on a chain without the renewal pallet.
	pub async fn enable_auto_renew(
		&self,
		content_hash: ContentHash,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<()> {
		self.ensure_renewal_available()?;
		let tx = bulletin::tx().data_renewal().enable_auto_renew(content_hash);
		self.submit_and_watch(&tx, signer, wait_for, None, |e| {
			Error::RenewalFailed(format!("Enable auto-renew failed: {e}"))
		})
		.await?;
		Ok(())
	}

	/// Disable auto-renewal for stored content.
	///
	/// A signed caller must own the registration, and its prepaid cycle must
	/// already have fired; Root bypasses both checks.
	///
	/// Returns [`Error::RenewalUnavailable`] on a chain without the renewal pallet.
	pub async fn disable_auto_renew(
		&self,
		content_hash: ContentHash,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<()> {
		self.ensure_renewal_available()?;
		let tx = bulletin::tx().data_renewal().disable_auto_renew(content_hash);
		self.submit_and_watch(&tx, signer, wait_for, None, |e| {
			Error::RenewalFailed(format!("Disable auto-renew failed: {e}"))
		})
		.await?;
		Ok(())
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
		self.submit_and_watch(&tx, signer, wait_for, None, |e| {
			Error::TransactionFailed(format!("Refresh failed: {e}"))
		})
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
		self.submit_and_watch(&tx, signer, wait_for, None, |e| {
			Error::TransactionFailed(format!("Refresh failed: {e}"))
		})
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
		self.submit_and_watch(&tx, signer, wait_for, None, |e| {
			Error::TransactionFailed(format!("Removal failed: {e}"))
		})
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
		self.submit_and_watch(&tx, signer, wait_for, None, |e| {
			Error::TransactionFailed(format!("Removal failed: {e}"))
		})
		.await?;
		Ok(())
	}
}

/// Internal result from `submit_and_watch`.
struct SubmitResult {
	block_hash: String,
	extrinsic_hash: String,
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
	pub entry: TransactionRef<u32>,
	pub block_hash: String,
}
