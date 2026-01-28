// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Subxt-based transaction submitter implementation.
//!
//! This module provides a [`SubxtSubmitter`] that uses the `subxt` library
//! to submit transactions to Bulletin Chain with full type safety.

use crate::{
	cid::ContentHash,
	submit::{TransactionReceipt, TransactionSubmitter},
	types::{Error, Result},
};
use alloc::vec::Vec;
use sp_runtime::AccountId32;
use subxt::{OnlineClient, PolkadotConfig};

/// Transaction submitter using `subxt` for type-safe blockchain interaction.
///
/// # Example
///
/// ```ignore
/// use bulletin_sdk_rust::submitters::SubxtSubmitter;
/// use bulletin_sdk_rust::async_client::AsyncBulletinClient;
/// use subxt::{OnlineClient, PolkadotConfig};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Create subxt client
///     let api = OnlineClient::<PolkadotConfig>::new().await?;
///     let signer = // ... your signer (PairSigner, etc.)
///
///     // Create submitter
///     let submitter = SubxtSubmitter::new(api, signer);
///
///     // Create bulletin client with submitter
///     let client = AsyncBulletinClient::new(submitter);
///
///     // Use the client
///     let data = b"Hello, Bulletin!".to_vec();
///     let result = client.store(data, Default::default()).await?;
///
///     Ok(())
/// }
/// ```
pub struct SubxtSubmitter<Signer> {
	/// Subxt online client.
	api: OnlineClient<PolkadotConfig>,
	/// Transaction signer.
	signer: Signer,
}

impl<Signer> SubxtSubmitter<Signer> {
	/// Create a new SubxtSubmitter.
	///
	/// # Arguments
	///
	/// * `api` - The subxt `OnlineClient` connected to a Bulletin Chain node
	/// * `signer` - A signer that implements `subxt::tx::Signer` (e.g., `PairSigner`)
	pub fn new(api: OnlineClient<PolkadotConfig>, signer: Signer) -> Self {
		Self { api, signer }
	}

	/// Get a reference to the API client.
	pub fn api(&self) -> &OnlineClient<PolkadotConfig> {
		&self.api
	}

	/// Get a reference to the signer.
	pub fn signer(&self) -> &Signer {
		&self.signer
	}
}

#[async_trait::async_trait]
impl<Signer> TransactionSubmitter for SubxtSubmitter<Signer>
where
	Signer: subxt::tx::Signer<PolkadotConfig> + Send + Sync,
{
	async fn submit_store(&self, data: Vec<u8>) -> Result<TransactionReceipt> {
		// NOTE: This is a placeholder implementation.
		// The actual implementation requires:
		// 1. Generated code from subxt metadata for TransactionStorage pallet
		// 2. Proper CID configuration handling
		// 3. Transaction building with the correct call
		//
		// Example (requires generated metadata):
		// ```ignore
		// use bulletin_metadata::transaction_storage;
		//
		// let tx = transaction_storage::calls::TransactionRoot::new(
		//     transaction_storage::calls::Store {
		//         data: data.into(),
		//         cid_config: None, // or Some(cid_config)
		//     }
		// );
		//
		// let result = self.api
		//     .tx()
		//     .sign_and_submit_then_watch_default(&tx, &self.signer)
		//     .await
		//     .map_err(|e| Error::SubmissionFailed(format!("Subxt error: {e:?}")))?;
		//
		// let events = result
		//     .wait_for_finalized_success()
		//     .await
		//     .map_err(|e| Error::SubmissionFailed(format!("Finalization error: {e:?}")))?;
		//
		// Ok(TransactionReceipt {
		//     block_hash: format!("{:?}", events.block_hash()),
		//     extrinsic_hash: format!("{:?}", events.extrinsic_hash()),
		//     block_number: None, // Could be extracted from block header
		// })
		// ```

		let _ = data; // Silence unused warning
		Err(Error::SubmissionFailed(
			"SubxtSubmitter requires metadata generation. See subxt documentation for generating metadata code.".into()
		))
	}

	async fn submit_authorize_account(
		&self,
		who: AccountId32,
		transactions: u32,
		bytes: u64,
	) -> Result<TransactionReceipt> {
		// Placeholder - requires generated metadata
		let _ = (who, transactions, bytes);
		Err(Error::SubmissionFailed(
			"SubxtSubmitter requires metadata generation. See subxt documentation.".into(),
		))
	}

	async fn submit_authorize_preimage(
		&self,
		content_hash: ContentHash,
		max_size: u64,
	) -> Result<TransactionReceipt> {
		// Placeholder - requires generated metadata
		let _ = (content_hash, max_size);
		Err(Error::SubmissionFailed(
			"SubxtSubmitter requires metadata generation. See subxt documentation.".into(),
		))
	}

	async fn submit_renew(&self, block: u32, index: u32) -> Result<TransactionReceipt> {
		// Placeholder - requires generated metadata
		let _ = (block, index);
		Err(Error::SubmissionFailed(
			"SubxtSubmitter requires metadata generation. See subxt documentation.".into(),
		))
	}

	async fn submit_refresh_account_authorization(
		&self,
		who: AccountId32,
	) -> Result<TransactionReceipt> {
		// Placeholder - requires generated metadata
		let _ = who;
		Err(Error::SubmissionFailed(
			"SubxtSubmitter requires metadata generation. See subxt documentation.".into(),
		))
	}

	async fn submit_refresh_preimage_authorization(
		&self,
		content_hash: ContentHash,
	) -> Result<TransactionReceipt> {
		// Placeholder - requires generated metadata
		let _ = content_hash;
		Err(Error::SubmissionFailed(
			"SubxtSubmitter requires metadata generation. See subxt documentation.".into(),
		))
	}

	async fn submit_remove_expired_account_authorization(
		&self,
		who: AccountId32,
	) -> Result<TransactionReceipt> {
		// Placeholder - requires generated metadata
		let _ = who;
		Err(Error::SubmissionFailed(
			"SubxtSubmitter requires metadata generation. See subxt documentation.".into(),
		))
	}

	async fn submit_remove_expired_preimage_authorization(
		&self,
		content_hash: ContentHash,
	) -> Result<TransactionReceipt> {
		// Placeholder - requires generated metadata
		let _ = content_hash;
		Err(Error::SubmissionFailed(
			"SubxtSubmitter requires metadata generation. See subxt documentation.".into(),
		))
	}
}

// Note: For a complete implementation, users should:
// 1. Generate metadata using `subxt metadata` command from a running node
// 2. Use `#[subxt::subxt(runtime_metadata_path = "metadata.scale")]` to generate types
// 3. Implement the methods above using the generated types
//
// See the TypeScript SDK examples for reference on the expected transaction structure.
