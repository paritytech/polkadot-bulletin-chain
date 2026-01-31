// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Simple Rust example for authorize and store on Bulletin Chain using bulletin-sdk-rust.
//!
//! This example demonstrates using the SDK's AsyncBulletinClient with a subxt-based
//! TransactionSubmitter to:
//! 1. Authorize an account to store data
//! 2. Store data on the Bulletin Chain
//!
//! ## Setup
//!
//! Before running this example, generate metadata from a running node:
//!   ./fetch_metadata.sh ws://localhost:10000
//!
//! ## Usage
//!
//!   cargo run --release -- --ws ws://localhost:10000 --seed "//Alice"

use std::{str::FromStr, sync::Arc};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bulletin_sdk_rust::{
	async_client::AsyncBulletinClient,
	cid::ContentHash,
	submit::{TransactionReceipt, TransactionSubmitter},
	types::Error as SdkError,
	Result as SdkResult,
};
use clap::Parser;
use codec::Encode;
use scale_info::PortableRegistry;
use subxt::{
	client::ClientState,
	config::{
		signed_extensions::{self, SignedExtension},
		Config, DefaultExtrinsicParamsBuilder, ExtrinsicParams, ExtrinsicParamsEncoder,
		ExtrinsicParamsError, SubstrateConfig,
	},
	utils::AccountId32,
	OnlineClient,
};
use subxt_signer::sr25519::Keypair;

#[derive(Parser, Debug)]
#[command(name = "authorize-and-store")]
#[command(about = "Authorize and store data on Bulletin Chain using bulletin-sdk-rust")]
struct Args {
	/// WebSocket URL of the Bulletin Chain node
	#[arg(long, default_value = "ws://localhost:10000")]
	ws: String,

	/// Seed phrase or dev seed (e.g., "//Alice" or mnemonic)
	#[arg(long, default_value = "//Alice")]
	seed: String,
}

// Generate types from metadata using subxt codegen
// This reads bulletin_metadata.scale and generates all the necessary types at compile time
#[subxt::subxt(runtime_metadata_path = "bulletin_metadata.scale")]
pub mod bulletin {}

/// Custom Config for Bulletin Chain extending SubstrateConfig with ProvideCidConfig extension
pub enum BulletinConfig {}

impl Config for BulletinConfig {
	type Hash = <SubstrateConfig as Config>::Hash;
	type AccountId = <SubstrateConfig as Config>::AccountId;
	type Address = <SubstrateConfig as Config>::Address;
	type Signature = <SubstrateConfig as Config>::Signature;
	type Hasher = <SubstrateConfig as Config>::Hasher;
	type Header = <SubstrateConfig as Config>::Header;
	type ExtrinsicParams = signed_extensions::AnyOf<
		Self,
		(
			signed_extensions::CheckSpecVersion,
			signed_extensions::CheckTxVersion,
			signed_extensions::CheckNonce,
			signed_extensions::CheckGenesis<Self>,
			signed_extensions::CheckMortality<Self>,
			signed_extensions::ChargeAssetTxPayment<Self>,
			signed_extensions::ChargeTransactionPayment,
			signed_extensions::CheckMetadataHash,
			ProvideCidConfigExt, // Bulletin Chain's custom extension
		),
	>;
	type AssetId = <SubstrateConfig as Config>::AssetId;
}

/// Custom signed extension for Bulletin Chain's ProvideCidConfig.
///
/// This extension is required by the TransactionStorage pallet to configure CID codec
/// and hash algorithm. For non-store calls, this encodes as Option::None.
pub struct ProvideCidConfigExt;

impl<T: Config> SignedExtension<T> for ProvideCidConfigExt {
	type Decoded = ();

	fn matches(identifier: &str, _type_id: u32, _types: &PortableRegistry) -> bool {
		identifier == "ProvideCidConfig"
	}
}

impl<T: Config> ExtrinsicParams<T> for ProvideCidConfigExt {
	type Params = ();

	fn new(_client: &ClientState<T>, _params: Self::Params) -> Result<Self, ExtrinsicParamsError> {
		Ok(ProvideCidConfigExt)
	}
}

impl ExtrinsicParamsEncoder for ProvideCidConfigExt {
	fn encode_extra_to(&self, v: &mut Vec<u8>) {
		// Encode Option::None for non-store calls (no CID config needed)
		Option::<()>::None.encode_to(v);
	}

	fn encode_additional_to(&self, _v: &mut Vec<u8>) {
		// No additional signed data
	}
}

/// Helper to build extrinsic params with our custom extension
fn bulletin_params(
	params: DefaultExtrinsicParamsBuilder<BulletinConfig>,
) -> <<BulletinConfig as Config>::ExtrinsicParams as ExtrinsicParams<BulletinConfig>>::Params {
	let (a, b, c, d, e, f, g, h) = params.build();
	(a, b, c, d, e, f, g, h, ())
}

/// Subxt-based implementation of TransactionSubmitter for the SDK
struct SubxtSubmitter {
	api: OnlineClient<BulletinConfig>,
	sudo_keypair: Arc<Keypair>,
	storage_keypair: Arc<Keypair>,
}

impl SubxtSubmitter {
	fn new(
		api: OnlineClient<BulletinConfig>,
		sudo_keypair: Keypair,
		storage_keypair: Keypair,
	) -> Self {
		Self {
			api,
			sudo_keypair: Arc::new(sudo_keypair),
			storage_keypair: Arc::new(storage_keypair),
		}
	}

	/// Build a TransactionReceipt from subxt's ExtrinsicEvents
	async fn build_receipt(
		&self,
		events: subxt::blocks::ExtrinsicEvents<BulletinConfig>,
	) -> SdkResult<TransactionReceipt> {
		let block_hash = events.block_hash();
		let extrinsic_hash = events.extrinsic_hash();

		// Get block number if available
		let block = self
			.api
			.blocks()
			.at(block_hash)
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to get block: {e:?}")))?;
		let block_number = block.number();

		Ok(TransactionReceipt {
			block_hash: format!("{:?}", block_hash),
			extrinsic_hash: format!("{:?}", extrinsic_hash),
			block_number: Some(block_number),
		})
	}
}

#[async_trait]
impl TransactionSubmitter for SubxtSubmitter {
	async fn submit_store(&self, data: Vec<u8>) -> SdkResult<TransactionReceipt> {
		let signer = subxt_signer::sr25519::Keypair::from_secret_key(self.storage_keypair.secret_key())
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to create signer: {e:?}")))?;

		// Use generated store call from metadata
		let store_tx = bulletin::tx()
			.transaction_storage()
			.store(data);

		let events = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&store_tx, &signer)
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit: {e:?}")))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Transaction failed: {e:?}")))?;

		self.build_receipt(events).await
	}

	async fn submit_authorize_account(
		&self,
		who: AccountId32,
		transactions: u32,
		bytes: u64,
	) -> SdkResult<TransactionReceipt> {
		let signer = subxt_signer::sr25519::Keypair::from_secret_key(self.sudo_keypair.secret_key())
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to create signer: {e:?}")))?;

		// Use generated authorize_account call from metadata
		let authorize_tx = bulletin::tx()
			.transaction_storage()
			.authorize_account(who, transactions, bytes);

		// Wrap in sudo call
		let sudo_tx = bulletin::tx().sudo().sudo(authorize_tx);

		let events = self
			.api
			.tx()
			.sign_and_submit_then_watch(&sudo_tx, &signer, bulletin_params(Default::default()))
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit: {e:?}")))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Transaction failed: {e:?}")))?;

		self.build_receipt(events).await
	}

	async fn submit_authorize_preimage(
		&self,
		content_hash: ContentHash,
		max_size: u64,
	) -> SdkResult<TransactionReceipt> {
		let signer = subxt_signer::sr25519::Keypair::from_secret_key(self.sudo_keypair.secret_key())
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to create signer: {e:?}")))?;

		let authorize_tx = bulletin::tx()
			.transaction_storage()
			.authorize_preimage(content_hash, max_size);

		let sudo_tx = bulletin::tx().sudo().sudo(authorize_tx);

		let events = self
			.api
			.tx()
			.sign_and_submit_then_watch(&sudo_tx, &signer, bulletin_params(Default::default()))
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit: {e:?}")))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Transaction failed: {e:?}")))?;

		self.build_receipt(events).await
	}

	async fn submit_renew(&self, block: u32, index: u32) -> SdkResult<TransactionReceipt> {
		let signer = subxt_signer::sr25519::Keypair::from_secret_key(self.storage_keypair.secret_key())
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to create signer: {e:?}")))?;

		let renew_tx = bulletin::tx()
			.transaction_storage()
			.renew(block, index);

		let events = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&renew_tx, &signer)
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit: {e:?}")))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Transaction failed: {e:?}")))?;

		self.build_receipt(events).await
	}

	async fn submit_refresh_account_authorization(
		&self,
		who: AccountId32,
	) -> SdkResult<TransactionReceipt> {
		let signer = subxt_signer::sr25519::Keypair::from_secret_key(self.sudo_keypair.secret_key())
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to create signer: {e:?}")))?;

		let refresh_tx = bulletin::tx()
			.transaction_storage()
			.refresh_account_authorization(who);

		let sudo_tx = bulletin::tx().sudo().sudo(refresh_tx);

		let events = self
			.api
			.tx()
			.sign_and_submit_then_watch(&sudo_tx, &signer, bulletin_params(Default::default()))
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit: {e:?}")))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Transaction failed: {e:?}")))?;

		self.build_receipt(events).await
	}

	async fn submit_refresh_preimage_authorization(
		&self,
		content_hash: ContentHash,
	) -> SdkResult<TransactionReceipt> {
		let signer = subxt_signer::sr25519::Keypair::from_secret_key(self.sudo_keypair.secret_key())
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to create signer: {e:?}")))?;

		let refresh_tx = bulletin::tx()
			.transaction_storage()
			.refresh_preimage_authorization(content_hash);

		let sudo_tx = bulletin::tx().sudo().sudo(refresh_tx);

		let events = self
			.api
			.tx()
			.sign_and_submit_then_watch(&sudo_tx, &signer, bulletin_params(Default::default()))
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit: {e:?}")))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Transaction failed: {e:?}")))?;

		self.build_receipt(events).await
	}

	async fn submit_remove_expired_account_authorization(
		&self,
		who: AccountId32,
	) -> SdkResult<TransactionReceipt> {
		let signer = subxt_signer::sr25519::Keypair::from_secret_key(self.storage_keypair.secret_key())
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to create signer: {e:?}")))?;

		let remove_tx = bulletin::tx()
			.transaction_storage()
			.remove_expired_account_authorization(who);

		let events = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&remove_tx, &signer)
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit: {e:?}")))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Transaction failed: {e:?}")))?;

		self.build_receipt(events).await
	}

	async fn submit_remove_expired_preimage_authorization(
		&self,
		content_hash: ContentHash,
	) -> SdkResult<TransactionReceipt> {
		let signer = subxt_signer::sr25519::Keypair::from_secret_key(self.storage_keypair.secret_key())
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to create signer: {e:?}")))?;

		let remove_tx = bulletin::tx()
			.transaction_storage()
			.remove_expired_preimage_authorization(content_hash);

		let events = self
			.api
			.tx()
			.sign_and_submit_then_watch_default(&remove_tx, &signer)
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit: {e:?}")))?
			.wait_for_finalized_success()
			.await
			.map_err(|e| SdkError::SubmissionFailed(format!("Transaction failed: {e:?}")))?;

		self.build_receipt(events).await
	}

	fn signer_account(&self) -> Option<AccountId32> {
		Some(self.storage_keypair.public_key().into())
	}
}

#[tokio::main]
async fn main() -> Result<()> {
	let args = Args::parse();

	// Parse keypair from seed
	let keypair = keypair_from_seed(&args.seed)?;
	println!("Using account: {}", hex::encode(keypair.public_key().0));

	// Connect to Bulletin Chain node
	println!("Connecting to {}...", args.ws);
	let api = OnlineClient::<BulletinConfig>::from_url(&args.ws)
		.await
		.map_err(|e| anyhow!("Failed to connect: {e:?}"))?;
	println!("Connected successfully!");

	// Create submitter (Alice as sudo, same account for storage)
	let submitter = SubxtSubmitter::new(api, keypair.clone(), keypair);

	// Create SDK client with the submitter
	let client = AsyncBulletinClient::new(submitter);

	// Step 1: Authorize the account to store data
	println!("\nStep 1: Authorizing account...");
	client
		.authorize_account(
			keypair_from_seed(&args.seed)?.public_key().into(),
			100,      // 100 transactions
			100 * 1024 * 1024, // 100 MB
		)
		.await
		.map_err(|e| anyhow!("Failed to authorize account: {e:?}"))?;
	println!("Account authorized successfully!");

	// Step 2: Store data using the SDK
	println!("\nStep 2: Storing data...");
	let data_to_store = format!("Hello from Bulletin SDK at {}", chrono_lite());
	let result = client
		.store(data_to_store.as_bytes().to_vec(), None)
		.await
		.map_err(|e| anyhow!("Failed to store data: {e:?}"))?;

	println!("Data stored successfully!");
	println!("  CID: {}", hex::encode(&result.cid));
	println!("  Size: {} bytes", result.size);

	println!("\n\nTest passed!");

	Ok(())
}

fn keypair_from_seed(seed: &str) -> Result<Keypair> {
	if seed.starts_with("//") {
		let uri = subxt_signer::SecretUri::from_str(seed)
			.map_err(|e| anyhow!("Failed to parse secret URI: {e}"))?;
		let keypair =
			Keypair::from_uri(&uri).map_err(|e| anyhow!("Failed to create keypair: {e}"))?;
		Ok(keypair)
	} else {
		let mnemonic = subxt_signer::bip39::Mnemonic::from_str(seed)
			.map_err(|e| anyhow!("Failed to parse mnemonic: {e}"))?;
		let keypair = Keypair::from_phrase(&mnemonic, None)
			.map_err(|e| anyhow!("Failed to create keypair from mnemonic: {e}"))?;
		Ok(keypair)
	}
}

fn chrono_lite() -> String {
	use std::time::{SystemTime, UNIX_EPOCH};
	let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
	format!("{}s", duration.as_secs())
}
