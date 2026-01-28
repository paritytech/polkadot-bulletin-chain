// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Simple Rust example for authorize and store on Bulletin Chain using bulletin-sdk-rust.
//!
//! This example demonstrates using the SDK's AsyncBulletinClient with a subxt-based
//! TransactionSubmitter to:
//! 1. Authorize an account to store data
//! 2. Store data on the Bulletin Chain
//!
//! Usage:
//!   authorize-and-store --ws ws://localhost:10000 --seed "//Alice"

use std::str::FromStr;
use std::sync::Arc;

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
use subxt::{dynamic::Value, utils::AccountId32, OnlineClient, SubstrateConfig};
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

/// Subxt-based implementation of TransactionSubmitter for the SDK.
struct SubxtSubmitter {
    api: OnlineClient<SubstrateConfig>,
    sudo_keypair: Arc<Keypair>,
    storage_keypair: Arc<Keypair>,
}

impl SubxtSubmitter {
    fn new(
        api: OnlineClient<SubstrateConfig>,
        sudo_keypair: Keypair,
        storage_keypair: Keypair,
    ) -> Self {
        Self {
            api,
            sudo_keypair: Arc::new(sudo_keypair),
            storage_keypair: Arc::new(storage_keypair),
        }
    }
}

#[async_trait]
impl TransactionSubmitter for SubxtSubmitter {
    async fn submit_store(&self, data: Vec<u8>) -> SdkResult<TransactionReceipt> {
        println!("  SDK: Storing {} bytes of data...", data.len());

        let store_tx = subxt::dynamic::tx(
            "TransactionStorage",
            "store",
            vec![Value::from_bytes(&data)],
        );

        let tx_progress = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&store_tx, self.storage_keypair.as_ref())
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit store tx: {e}")))?;

        println!("  SDK: Store tx submitted, waiting for finalization...");

        let finalized = tx_progress
            .wait_for_finalized_success()
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Store tx failed: {e}")))?;

        println!("  SDK: Store finalized, extrinsic hash: {:?}", finalized.extrinsic_hash());

        Ok(TransactionReceipt {
            block_hash: format!("{:?}", finalized.extrinsic_hash()),
            extrinsic_hash: format!("{:?}", finalized.extrinsic_hash()),
            block_number: None,
        })
    }

    async fn submit_authorize_account(
        &self,
        who: sp_runtime::AccountId32,
        transactions: u32,
        bytes: u64,
    ) -> SdkResult<TransactionReceipt> {
        println!(
            "  SDK: Authorizing account {} for {} transactions and {} bytes...",
            who, transactions, bytes
        );

        // Build the inner call as a variant for the RuntimeCall enum
        let inner_call = Value::unnamed_variant(
            "TransactionStorage",
            [Value::unnamed_variant(
                "authorize_account",
                [
                    Value::from_bytes(AsRef::<[u8]>::as_ref(&who)),
                    Value::u128(transactions as u128),
                    Value::u128(bytes as u128),
                ],
            )],
        );

        // Wrap in Sudo.sudo
        let sudo_tx = subxt::dynamic::tx("Sudo", "sudo", vec![inner_call]);

        let tx_progress = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&sudo_tx, self.sudo_keypair.as_ref())
            .await
            .map_err(|e| {
                SdkError::SubmissionFailed(format!("Failed to submit authorize tx: {e}"))
            })?;

        println!("  SDK: Authorization tx submitted, waiting for finalization...");

        let finalized = tx_progress
            .wait_for_finalized_success()
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Authorize tx failed: {e}")))?;

        println!(
            "  SDK: Authorization finalized, extrinsic hash: {:?}",
            finalized.extrinsic_hash()
        );

        Ok(TransactionReceipt {
            block_hash: format!("{:?}", finalized.extrinsic_hash()),
            extrinsic_hash: format!("{:?}", finalized.extrinsic_hash()),
            block_number: None,
        })
    }

    async fn submit_authorize_preimage(
        &self,
        content_hash: ContentHash,
        max_size: u64,
    ) -> SdkResult<TransactionReceipt> {
        println!(
            "  SDK: Authorizing preimage {:?} with max size {}...",
            content_hash, max_size
        );

        // Build the inner call as a variant for the RuntimeCall enum
        let inner_call = Value::unnamed_variant(
            "TransactionStorage",
            [Value::unnamed_variant(
                "authorize_preimage",
                [
                    Value::from_bytes(content_hash),
                    Value::u128(max_size as u128),
                ],
            )],
        );

        // Wrap in Sudo.sudo
        let sudo_tx = subxt::dynamic::tx("Sudo", "sudo", vec![inner_call]);

        let tx_progress = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&sudo_tx, self.sudo_keypair.as_ref())
            .await
            .map_err(|e| {
                SdkError::SubmissionFailed(format!("Failed to submit authorize_preimage tx: {e}"))
            })?;

        let finalized = tx_progress
            .wait_for_finalized_success()
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Authorize preimage tx failed: {e}")))?;

        Ok(TransactionReceipt {
            block_hash: format!("{:?}", finalized.extrinsic_hash()),
            extrinsic_hash: format!("{:?}", finalized.extrinsic_hash()),
            block_number: None,
        })
    }

    async fn submit_renew(&self, block: u32, index: u32) -> SdkResult<TransactionReceipt> {
        let renew_tx = subxt::dynamic::tx(
            "TransactionStorage",
            "renew",
            vec![Value::u128(block as u128), Value::u128(index as u128)],
        );

        let tx_progress = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&renew_tx, self.storage_keypair.as_ref())
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit renew tx: {e}")))?;

        let finalized = tx_progress
            .wait_for_finalized_success()
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Renew tx failed: {e}")))?;

        Ok(TransactionReceipt {
            block_hash: format!("{:?}", finalized.extrinsic_hash()),
            extrinsic_hash: format!("{:?}", finalized.extrinsic_hash()),
            block_number: None,
        })
    }

    async fn submit_refresh_account_authorization(
        &self,
        who: sp_runtime::AccountId32,
    ) -> SdkResult<TransactionReceipt> {
        // Build the inner call as a variant for the RuntimeCall enum
        let inner_call = Value::unnamed_variant(
            "TransactionStorage",
            [Value::unnamed_variant(
                "refresh_account_authorization",
                [Value::from_bytes(AsRef::<[u8]>::as_ref(&who))],
            )],
        );

        // Wrap in Sudo.sudo
        let sudo_tx = subxt::dynamic::tx("Sudo", "sudo", vec![inner_call]);

        let tx_progress = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&sudo_tx, self.sudo_keypair.as_ref())
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit tx: {e}")))?;

        let finalized = tx_progress
            .wait_for_finalized_success()
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Tx failed: {e}")))?;

        Ok(TransactionReceipt {
            block_hash: format!("{:?}", finalized.extrinsic_hash()),
            extrinsic_hash: format!("{:?}", finalized.extrinsic_hash()),
            block_number: None,
        })
    }

    async fn submit_refresh_preimage_authorization(
        &self,
        content_hash: ContentHash,
    ) -> SdkResult<TransactionReceipt> {
        // Build the inner call as a variant for the RuntimeCall enum
        let inner_call = Value::unnamed_variant(
            "TransactionStorage",
            [Value::unnamed_variant(
                "refresh_preimage_authorization",
                [Value::from_bytes(content_hash)],
            )],
        );

        // Wrap in Sudo.sudo
        let sudo_tx = subxt::dynamic::tx("Sudo", "sudo", vec![inner_call]);

        let tx_progress = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&sudo_tx, self.sudo_keypair.as_ref())
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit tx: {e}")))?;

        let finalized = tx_progress
            .wait_for_finalized_success()
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Tx failed: {e}")))?;

        Ok(TransactionReceipt {
            block_hash: format!("{:?}", finalized.extrinsic_hash()),
            extrinsic_hash: format!("{:?}", finalized.extrinsic_hash()),
            block_number: None,
        })
    }

    async fn submit_remove_expired_account_authorization(
        &self,
        who: sp_runtime::AccountId32,
    ) -> SdkResult<TransactionReceipt> {
        let call = subxt::dynamic::tx(
            "TransactionStorage",
            "remove_expired_account_authorization",
            vec![Value::from_bytes(AsRef::<[u8]>::as_ref(&who))],
        );

        let tx_progress = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&call, self.storage_keypair.as_ref())
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit tx: {e}")))?;

        let finalized = tx_progress
            .wait_for_finalized_success()
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Tx failed: {e}")))?;

        Ok(TransactionReceipt {
            block_hash: format!("{:?}", finalized.extrinsic_hash()),
            extrinsic_hash: format!("{:?}", finalized.extrinsic_hash()),
            block_number: None,
        })
    }

    async fn submit_remove_expired_preimage_authorization(
        &self,
        content_hash: ContentHash,
    ) -> SdkResult<TransactionReceipt> {
        let call = subxt::dynamic::tx(
            "TransactionStorage",
            "remove_expired_preimage_authorization",
            vec![Value::from_bytes(content_hash)],
        );

        let tx_progress = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&call, self.storage_keypair.as_ref())
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Failed to submit tx: {e}")))?;

        let finalized = tx_progress
            .wait_for_finalized_success()
            .await
            .map_err(|e| SdkError::SubmissionFailed(format!("Tx failed: {e}")))?;

        Ok(TransactionReceipt {
            block_hash: format!("{:?}", finalized.extrinsic_hash()),
            extrinsic_hash: format!("{:?}", finalized.extrinsic_hash()),
            block_number: None,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    println!("Connecting to: {}", args.ws);
    println!("Using seed: {}", args.seed);

    // Connect to the node
    let api = OnlineClient::<SubstrateConfig>::from_url(&args.ws).await?;
    println!("Connected to chain");

    // Create keypairs
    let sudo_keypair = keypair_from_seed(&args.seed)?;
    let sudo_account: AccountId32 = sudo_keypair.public_key().into();
    println!("Sudo account: {}", sudo_account);

    let storage_keypair = keypair_from_seed("//Rustsigner")?;
    let storage_account: AccountId32 = storage_keypair.public_key().into();
    println!("Storage account: {}", storage_account);

    // Create the SDK client with our subxt-based submitter
    let submitter = SubxtSubmitter::new(api, sudo_keypair, storage_keypair);
    let client = AsyncBulletinClient::new(submitter);

    // Data to store
    let data_to_store = format!("Hello, Bulletin from Rust SDK - {}", chrono_lite());
    println!("Data to store: {}", data_to_store);

    // Convert subxt AccountId32 to sp_runtime::AccountId32 for the SDK
    let who = sp_runtime::AccountId32::from(storage_account.0);

    // Step 1: Authorize account using the SDK
    println!("\nStep 1: Authorizing account...");
    client
        .authorize_account(who, 100, 100 * 1024 * 1024)
        .await
        .map_err(|e| anyhow!("Failed to authorize account: {e:?}"))?;
    println!("Account authorized successfully!");

    // Step 2: Store data using the SDK
    println!("\nStep 2: Storing data...");
    let result = client
        .store(data_to_store.as_bytes().to_vec(), Default::default())
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
