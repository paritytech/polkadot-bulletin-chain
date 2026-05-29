use anyhow::{anyhow, Result};
use std::time::Duration;
use subxt::{
	dynamic::{tx, Value},
	ext::scale_value::value,
	OnlineClient,
};
use subxt_signer::sr25519::Keypair;

use crate::{
	accounts::NonceTracker,
	client::{BulletinConfig, BulletinExtrinsicParamsBuilder},
	store::wait_for_in_best_block,
};

/// Maximum accounts per `Utility::batch_all` authorize call (block weight).
///
/// Kept **moderately small** on purpose: smaller batches mean more frequent authorize extrinsics,
/// so accounts become usable on-chain sooner and store submitters are less likely to run out of
/// authorized signers while blocks still have capacity.
pub const AUTHORIZE_BATCH_SIZE: usize = 2048;
const AUTHORIZE_TIMEOUT_SECS: u64 = 60;
const MAX_NONCE_RETRIES: u32 = 10;

/// Authorize a single batch of accounts (at most `AUTHORIZE_BATCH_SIZE` recommended).
///
/// Same semantics as one iteration inside [`authorize_accounts`]: waits for inclusion
/// in a best block and retries on nonce errors.
pub async fn authorize_account_batch(
	client: &OnlineClient<BulletinConfig>,
	authorizer: &Keypair,
	nonce_tracker: &NonceTracker,
	accounts: &[subxt::utils::AccountId32],
	transactions_per_account: u32,
	bytes_per_account: u64,
) -> Result<()> {
	if accounts.is_empty() {
		return Ok(());
	}

	let authorizer_id = authorizer.public_key().to_account_id();
	let mut attempts = 0u32;
	loop {
		let call = build_authorize_call(accounts, transactions_per_account, bytes_per_account);

		let nonce = nonce_tracker.next_nonce(&authorizer_id);
		let params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).build();

		tracing::info!("Authorizing batch of {} accounts (nonce={})", accounts.len(), nonce);

		let result = tokio::time::timeout(Duration::from_secs(AUTHORIZE_TIMEOUT_SECS), async {
			let progress =
				client.tx().sign_and_submit_then_watch(&call, authorizer, params).await?;
			let (block_hash, _events) = wait_for_in_best_block(progress).await?;
			Ok::<_, anyhow::Error>(block_hash)
		})
		.await;

		match result {
			Ok(Ok(block_hash)) => {
				tracing::info!(
					"Batch of {} accounts included in best block {block_hash:?}",
					accounts.len()
				);
				return Ok(());
			},
			Ok(Err(e)) if is_nonce_error(&e) && attempts < MAX_NONCE_RETRIES => {
				attempts += 1;
				tracing::warn!(
					"Authorization failed (attempt {attempts}/{MAX_NONCE_RETRIES}), \
					 waiting for block then refreshing nonce: {e}"
				);
				tokio::time::sleep(Duration::from_secs(6)).await;
				nonce_tracker.refresh(client, &authorizer_id).await?;
				tracing::info!("Nonce refreshed from chain after retry delay");
			},
			Ok(Err(e)) => return Err(e),
			Err(_) => return Err(anyhow!("authorize_account_batch timed out")),
		}
	}
}

/// Authorize multiple accounts for transaction storage.
///
/// The signer must be a member of the runtime's `Authorizer` origin (e.g. Alice
/// is in `TestAccounts` on the bulletin-westend runtime).
/// No sudo wrapping is needed — `authorize_account` is called directly.
///
/// Splits into batches of AUTHORIZE_BATCH_SIZE to stay within block weight limits.
/// Waits for each batch to appear in a best block (not finalization) before
/// submitting the next batch.
///
/// On nonce/invalid-transaction errors (common on live networks where the authorizer
/// account may be used concurrently), refreshes the nonce from chain and retries.
pub async fn authorize_accounts(
	client: &OnlineClient<BulletinConfig>,
	authorizer: &Keypair,
	nonce_tracker: &NonceTracker,
	accounts: &[subxt::utils::AccountId32],
	transactions_per_account: u32,
	bytes_per_account: u64,
) -> Result<()> {
	for batch in accounts.chunks(AUTHORIZE_BATCH_SIZE) {
		authorize_account_batch(
			client,
			authorizer,
			nonce_tracker,
			batch,
			transactions_per_account,
			bytes_per_account,
		)
		.await?;
	}

	Ok(())
}

fn build_authorize_call(
	batch: &[subxt::utils::AccountId32],
	transactions_per_account: u32,
	bytes_per_account: u64,
) -> subxt::tx::DefaultPayload<subxt::ext::scale_value::Composite<()>> {
	let authorize_calls: Vec<Value> = batch
		.iter()
		.map(|account| {
			value! {
				TransactionStorage(authorize_account {
					who: Value::from_bytes(account.0),
					transactions: transactions_per_account,
					bytes: bytes_per_account,
				})
			}
		})
		.collect();

	if authorize_calls.len() == 1 {
		tx(
			"TransactionStorage",
			"authorize_account",
			vec![
				Value::from_bytes(batch[0].0),
				Value::u128(transactions_per_account as u128),
				Value::u128(bytes_per_account as u128),
			],
		)
	} else {
		let items = Value::unnamed_composite(authorize_calls);
		tx("Utility", "batch_all", vec![items])
	}
}

/// True only for errors where refreshing the account nonce and resubmitting can help.
///
/// Do **not** match the substring `"invalid"` — every `Invalid Transaction (1010)` would qualify,
/// including bad signer, oversized batch, payment, and exhausts-resources.
fn is_nonce_error(e: &anyhow::Error) -> bool {
	let msg = format!("{e:#}").to_lowercase();
	msg.contains("stale") ||
		msg.contains("future") ||
		msg.contains("outdated") ||
		msg.contains("priority is too low") ||
		msg.contains("nonce too low") ||
		msg.contains("nonce too high") ||
		msg.contains("transaction is outdated") ||
		(msg.contains("1010") && (msg.contains("stale") || msg.contains("future")))
}
