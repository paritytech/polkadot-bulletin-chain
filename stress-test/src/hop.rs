use anyhow::{Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use jsonrpsee::{core::client::ClientT, rpc_params, ws_client::WsClient};
use rand::rngs::OsRng;
use serde::Deserialize;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use subxt_signer::sr25519::Keypair;

use crate::client;

/// Domain-separator prefix the runtime uses when verifying `hop_submit` signatures.
const HOP_SUBMIT_CONTEXT: &[u8] = b"hop-submit-v1:";

/// Domain-separator prefix the runtime uses when verifying `hop_claim` signatures.
const HOP_CLAIM_CONTEXT: &[u8] = b"hop-claim-v1:";

/// `blake2_256(context || hash)` — recipients sign this for claim/ack operations.
fn op_signing_payload(context: &[u8], hash: &[u8]) -> [u8; 32] {
	let mut buf = Vec::with_capacity(context.len() + hash.len());
	buf.extend_from_slice(context);
	buf.extend_from_slice(hash);
	client::blake2b_256(&buf)
}

// ---------------------------------------------------------------------------
// Types matching the HOP RPC responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolStatus {
	pub entry_count: usize,
	pub total_bytes: u64,
	pub max_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitResult {
	pub pool_status: PoolStatus,
}

// ---------------------------------------------------------------------------
// Ed25519 keypair helpers
// ---------------------------------------------------------------------------

/// An ephemeral ed25519 keypair used as a HOP recipient.
#[derive(Debug, Clone)]
pub struct RecipientKeypair {
	pub signing_key: SigningKey,
}

impl RecipientKeypair {
	pub fn generate() -> Self {
		Self { signing_key: SigningKey::generate(&mut OsRng) }
	}

	/// 32-byte public key.
	pub fn public_bytes(&self) -> [u8; 32] {
		self.signing_key.verifying_key().to_bytes()
	}

	/// SCALE-encoded `MultiSigner::Ed25519(pubkey)`.
	/// MultiSigner enum variant 0 = Ed25519, so: `[0x00] ++ pubkey[32]`.
	pub fn scale_multi_signer(&self) -> Vec<u8> {
		let mut buf = Vec::with_capacity(33);
		buf.push(0x00); // Ed25519 variant
		buf.extend_from_slice(&self.public_bytes());
		buf
	}

	/// Sign `msg` and return SCALE-encoded `MultiSignature::Ed25519(sig)`.
	/// MultiSignature enum variant 0 = Ed25519, so: `[0x00] ++ sig[64]`.
	pub fn sign_multi_signature(&self, msg: &[u8]) -> Vec<u8> {
		let sig = self.signing_key.sign(msg);
		let mut buf = Vec::with_capacity(65);
		buf.push(0x00); // Ed25519 variant
		buf.extend_from_slice(&sig.to_bytes());
		buf
	}
}

// ---------------------------------------------------------------------------
// Submitter helpers (sr25519, must be authorized by the runtime)
// ---------------------------------------------------------------------------

/// SCALE-encoded `MultiSigner::Sr25519(pubkey)`. Variant index 1, then 32-byte key.
fn submitter_multi_signer(submitter: &Keypair) -> Vec<u8> {
	let mut buf = Vec::with_capacity(33);
	buf.push(0x01);
	buf.extend_from_slice(&submitter.public_key().0);
	buf
}

/// SCALE-encoded `MultiSignature::Sr25519(sig)`. Variant index 1, then 64-byte sig.
fn submitter_multi_signature(submitter: &Keypair, msg: &[u8]) -> Vec<u8> {
	let sig = submitter.sign(msg).0;
	let mut buf = Vec::with_capacity(65);
	buf.push(0x01);
	buf.extend_from_slice(&sig);
	buf
}

/// `blake2_256(HOP_SUBMIT_CONTEXT || blake2_256(data) || submit_timestamp.to_le_bytes())`
/// — must match the runtime pallet's reconstruction byte-for-byte.
fn submit_signing_payload(data: &[u8], submit_timestamp: u64) -> [u8; 32] {
	let data_hash = client::blake2b_256(data);
	let mut buf = Vec::with_capacity(HOP_SUBMIT_CONTEXT.len() + 32 + 8);
	buf.extend_from_slice(HOP_SUBMIT_CONTEXT);
	buf.extend_from_slice(&data_hash);
	buf.extend_from_slice(&submit_timestamp.to_le_bytes());
	client::blake2b_256(&buf)
}

fn now_ms() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.expect("clock is past UNIX_EPOCH")
		.as_millis() as u64
}

// ---------------------------------------------------------------------------
// RPC helpers
// ---------------------------------------------------------------------------

/// Submit data to HOP pool. Returns (content_hash, submit_result, latency).
///
/// `submitter` must be an authorized sr25519 account (see
/// `pallet-bulletin-transaction-storage::authorize_account`).
pub async fn hop_submit(
	ws: &WsClient,
	data: &[u8],
	recipients: &[RecipientKeypair],
	submitter: &Keypair,
) -> Result<([u8; 32], SubmitResult, std::time::Duration)> {
	let data_hex = format!("0x{}", hex::encode(data));
	let recipient_hexes: Vec<String> = recipients
		.iter()
		.map(|r| format!("0x{}", hex::encode(r.scale_multi_signer())))
		.collect();

	let submit_timestamp = now_ms();
	let payload = submit_signing_payload(data, submit_timestamp);
	let signature_hex =
		format!("0x{}", hex::encode(submitter_multi_signature(submitter, &payload)));
	let signer_hex = format!("0x{}", hex::encode(submitter_multi_signer(submitter)));

	let start = Instant::now();
	let result: SubmitResult = ws
		.request(
			"hop_submit",
			rpc_params![data_hex, recipient_hexes, signature_hex, signer_hex, submit_timestamp],
		)
		.await?;
	let latency = start.elapsed();

	let hash = client::blake2b_256(data);
	Ok((hash, result, latency))
}

/// Claim data from HOP pool. Returns (data, latency).
pub async fn hop_claim(
	ws: &WsClient,
	hash: &[u8],
	recipient: &RecipientKeypair,
) -> Result<(Vec<u8>, std::time::Duration)> {
	let hash_hex = format!("0x{}", hex::encode(hash));
	let payload = op_signing_payload(HOP_CLAIM_CONTEXT, hash);
	let signature = recipient.sign_multi_signature(&payload);
	let sig_hex = format!("0x{}", hex::encode(&signature));

	let start = Instant::now();
	let data_hex: String = ws.request("hop_claim", rpc_params![hash_hex, sig_hex]).await?;
	let latency = start.elapsed();

	let data = hex::decode(data_hex.strip_prefix("0x").unwrap_or(&data_hex))
		.context("decoding claimed data")?;
	Ok((data, latency))
}

/// Get pool status.
pub async fn hop_pool_status(ws: &WsClient) -> Result<PoolStatus> {
	let status: PoolStatus = ws
		.request("hop_poolStatus", rpc_params![])
		.await
		.context("hop_poolStatus RPC")?;
	Ok(status)
}

/// Error-scenario tests assert against the runtime's numeric codes, which only call errors carry;
/// transport and decode failures have none, hence `None`.
pub fn error_code(err: &anyhow::Error) -> Option<i32> {
	match err.downcast_ref::<jsonrpsee::core::ClientError>() {
		Some(jsonrpsee::core::ClientError::Call(obj)) => Some(obj.code()),
		_ => None,
	}
}

// ---------------------------------------------------------------------------
// Payload generation (deterministic, unique per index)
// ---------------------------------------------------------------------------

/// Generate a unique payload of `size` bytes for index `i`.
/// Uses a combination of index, size, and a per-process random salt so
/// successive runs never produce duplicate content hashes.
pub fn generate_payload(size: usize, index: u64) -> Vec<u8> {
	use rand::{Rng, SeedableRng};
	use std::sync::OnceLock;
	static SALT: OnceLock<u64> = OnceLock::new();
	let salt = *SALT.get_or_init(rand::random);
	let seed = index ^ salt ^ (size as u64).wrapping_mul(0x9E3779B97F4A7C15);
	let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
	let mut data = vec![0u8; size];
	rng.fill(&mut data[..]);
	data
}
