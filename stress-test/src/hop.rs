use anyhow::{Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use jsonrpsee::{core::client::ClientT, rpc_params, ws_client::WsClient};
use rand::rngs::OsRng;
use serde::Deserialize;
use std::time::Instant;

use crate::client;

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
// RPC helpers
// ---------------------------------------------------------------------------

/// Submit data to HOP pool. Returns (content_hash, submit_result, latency).
pub async fn hop_submit(
	ws: &WsClient,
	data: &[u8],
	recipients: &[RecipientKeypair],
) -> Result<([u8; 32], SubmitResult, std::time::Duration)> {
	let data_hex = format!("0x{}", hex::encode(data));
	let recipient_hexes: Vec<String> = recipients
		.iter()
		.map(|r| format!("0x{}", hex::encode(r.scale_multi_signer())))
		.collect();
	let proof_hex = "0x"; // NoopVerifier accepts empty proof

	let start = Instant::now();
	let result: SubmitResult = ws
		.request("hop_submit", rpc_params![data_hex, recipient_hexes, proof_hex])
		.await
		.map_err(|e| anyhow::anyhow!("hop_submit: {e}"))?;
	let latency = start.elapsed();

	let hash = client::blake2b_256(data);
	Ok((hash, result, latency))
}

/// Claim data from HOP pool. Returns (data, latency).
pub async fn hop_claim(
	ws: &WsClient,
	hash: &[u8; 32],
	recipient: &RecipientKeypair,
) -> Result<(Vec<u8>, std::time::Duration)> {
	let hash_hex = format!("0x{}", hex::encode(hash));
	let signature = recipient.sign_multi_signature(hash);
	let sig_hex = format!("0x{}", hex::encode(&signature));

	let start = Instant::now();
	let data_hex: String = ws
		.request("hop_claim", rpc_params![hash_hex, sig_hex])
		.await
		.map_err(|e| anyhow::anyhow!("hop_claim: {e}"))?;
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

/// Try an RPC call that should fail, return the error code (or None if it succeeded).
pub async fn try_hop_submit(
	ws: &WsClient,
	data: &[u8],
	recipients: &[RecipientKeypair],
) -> Option<i32> {
	let data_hex = format!("0x{}", hex::encode(data));
	let recipient_hexes: Vec<String> = recipients
		.iter()
		.map(|r| format!("0x{}", hex::encode(r.scale_multi_signer())))
		.collect();

	let result: Result<SubmitResult, _> =
		ws.request("hop_submit", rpc_params![data_hex, recipient_hexes, "0x"]).await;
	match result {
		Err(e) => extract_error_code(&e),
		Ok(_) => None,
	}
}

pub async fn try_hop_claim(
	ws: &WsClient,
	hash: &[u8],
	recipient: &RecipientKeypair,
) -> Option<i32> {
	let hash_hex = format!("0x{}", hex::encode(hash));
	let signature = recipient.sign_multi_signature(hash);
	let sig_hex = format!("0x{}", hex::encode(&signature));

	let result: Result<String, _> = ws.request("hop_claim", rpc_params![hash_hex, sig_hex]).await;
	match result {
		Err(e) => extract_error_code(&e),
		Ok(_) => None,
	}
}

fn extract_error_code(err: &jsonrpsee::core::ClientError) -> Option<i32> {
	if let jsonrpsee::core::ClientError::Call(obj) = err {
		Some(obj.code())
	} else {
		None
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
	let salt = *SALT.get_or_init(|| rand::random());
	let seed = index ^ salt ^ (size as u64).wrapping_mul(0x9E3779B97F4A7C15);
	let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
	let mut data = vec![0u8; size];
	rng.fill(&mut data[..]);
	data
}
