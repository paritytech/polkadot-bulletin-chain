use anyhow::Result;
use std::{
	collections::HashMap,
	sync::{Arc, Mutex},
};
use subxt::OnlineClient;
use subxt_signer::sr25519::Keypair;

use crate::client::BulletinConfig;

/// Thread-safe nonce tracker for parallel transaction submission.
/// Uses `[u8; 32]` as key because subxt's `AccountId32` doesn't implement `Hash`.
#[derive(Clone)]
pub struct NonceTracker {
	nonces: Arc<Mutex<HashMap<[u8; 32], u64>>>,
}

impl NonceTracker {
	pub fn new() -> Self {
		Self { nonces: Arc::new(Mutex::new(HashMap::new())) }
	}

	/// Initialize nonce for an account by querying the chain at best block.
	pub async fn init_from_chain(
		&self,
		client: &OnlineClient<BulletinConfig>,
		account_id: &subxt::utils::AccountId32,
	) -> Result<()> {
		let nonce = account_nonce_at_best(client, account_id).await?;
		self.nonces.lock().unwrap().insert(account_id.0, nonce);
		Ok(())
	}

	/// Get the next nonce for an account and increment it.
	pub fn next_nonce(&self, account_id: &subxt::utils::AccountId32) -> u64 {
		let mut nonces = self.nonces.lock().unwrap();
		let nonce = nonces.entry(account_id.0).or_insert(0);
		let current = *nonce;
		*nonce += 1;
		current
	}

	/// Roll back the nonce by 1 (undo a `next_nonce` that was not consumed).
	pub fn rollback(&self, account_id: &subxt::utils::AccountId32) {
		let mut nonces = self.nonces.lock().unwrap();
		if let Some(nonce) = nonces.get_mut(&account_id.0) {
			*nonce = nonce.saturating_sub(1);
		}
	}

	/// Reset nonce for an account by re-querying the chain at best block.
	pub async fn refresh(
		&self,
		client: &OnlineClient<BulletinConfig>,
		account_id: &subxt::utils::AccountId32,
	) -> Result<()> {
		let nonce = account_nonce_at_best(client, account_id).await?;
		self.nonces.lock().unwrap().insert(account_id.0, nonce);
		Ok(())
	}
}

/// Query account nonce at the best (not finalized) block.
///
/// Subxt's `client.tx().account_nonce()` uses the finalized block, which can
/// lag behind best by many blocks. This causes stale nonce errors when txs
/// have been included in best blocks but not yet finalized.
async fn account_nonce_at_best(
	client: &OnlineClient<BulletinConfig>,
	account_id: &subxt::utils::AccountId32,
) -> Result<u64> {
	let best_hash = crate::client::best_block_hash(client).await?;
	crate::client::get_account_nonce_at(client, account_id, best_hash).await
}

impl Default for NonceTracker {
	fn default() -> Self {
		Self::new()
	}
}

/// Derive a single sr25519 keypair (same scheme as [`generate_keypairs`]).
pub fn keypair_at_derivation_prefix(prefix: &str, index: u32) -> Keypair {
	let uri = format!("//{prefix}/{index}");
	let secret_uri: subxt_signer::SecretUri = uri.parse().expect("valid secret URI");
	Keypair::from_uri(&secret_uri).expect("valid derivation path")
}

/// Generate N keypairs for stress testing using derivation paths.
pub fn generate_keypairs(count: u32, prefix: &str) -> Vec<Keypair> {
	(0..count).map(|i| keypair_at_derivation_prefix(prefix, i)).collect()
}

/// Batch-initialize nonces for many accounts using concurrent RPC queries.
///
/// Uses the provided connection pool to run up to `concurrency` nonce queries
/// in parallel. This is much faster than sequential init for thousands of
/// accounts (~50ms per RPC call → 4000 accounts @ concurrency=64 ≈ 3s vs 200s).
pub async fn batch_init_nonces(
	pool: &[std::sync::Arc<OnlineClient<BulletinConfig>>],
	nonce_tracker: &NonceTracker,
	keypairs: &[Keypair],
	concurrency: usize,
) -> (u64, u64) {
	use futures::stream::{self, StreamExt};

	let concurrency = concurrency.max(1);
	let nonce_tracker = nonce_tracker.clone();

	let results: Vec<bool> = stream::iter(keypairs.iter().enumerate())
		.map(|(i, kp)| {
			let client = pool[i % pool.len()].clone();
			let nonce_tracker = nonce_tracker.clone();
			async move {
				let account_id = kp.public_key().to_account_id();
				match client.tx().account_nonce(&account_id).await {
					Ok(nonce) => {
						nonce_tracker.nonces.lock().unwrap().insert(account_id.0, nonce);
						true
					},
					Err(e) => {
						log::warn!("batch_init_nonces: failed for account {i}: {e}");
						false
					},
				}
			}
		})
		.buffer_unordered(concurrency)
		.collect()
		.await;

	let ok = results.iter().filter(|&&r| r).count() as u64;
	let failed = results.iter().filter(|&&r| !r).count() as u64;
	(ok, failed)
}
