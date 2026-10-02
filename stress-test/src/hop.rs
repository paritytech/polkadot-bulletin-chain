// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use bulletin_hop_rpc_client::{
	blake2_256, hex0x, now_ms, op_signing_payload, scale_multi_signature_ed25519,
	scale_multi_signer_ed25519, scale_multi_signature_sr25519, scale_multi_signer_sr25519,
	submit_signing_payload, HOP_CLAIM_CONTEXT,
};
use ed25519_dalek::{Signer, SigningKey};
use jsonrpsee::{core::client::ClientT, rpc_params, ws_client::WsClient};
use rand::rngs::OsRng;
use serde::Deserialize;
use std::time::Instant;
use subxt_signer::sr25519::Keypair;

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
	pub fn scale_multi_signer(&self) -> Vec<u8> {
		scale_multi_signer_ed25519(&self.public_bytes())
	}

	/// Sign `msg` and return SCALE-encoded `MultiSignature::Ed25519(sig)`.
	pub fn sign_multi_signature(&self, msg: &[u8]) -> Vec<u8> {
		let sig = self.signing_key.sign(msg);
		scale_multi_signature_ed25519(&sig.to_bytes())
	}
}

// ---------------------------------------------------------------------------
// Submitter helpers (sr25519, must be authorized by the runtime)
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
	let data_hex = hex0x(data);
	let recipient_hexes: Vec<String> = recipients
		.iter()
		.map(|r| hex0x(&r.scale_multi_signer()))
		.collect();

	let submit_timestamp = now_ms();
	let payload = submit_signing_payload(data, submit_timestamp);
	let signature_hex =
		hex0x(&scale_multi_signature_sr25519(&submitter.sign(&payload).0));
	let signer_hex = hex0x(&scale_multi_signer_sr25519(&submitter.public_key().0));

	let start = Instant::now();
	let result: SubmitResult = ws
		.request(
			"hop_submit",
			rpc_params![data_hex, recipient_hexes, signature_hex, signer_hex, submit_timestamp],
		)
		.await?;
	let latency = start.elapsed();

	let hash = blake2_256(data);
	Ok((hash, result, latency))
}

/// Claim data from HOP pool. Returns (data, latency).
pub async fn hop_claim(
	ws: &WsClient,
	hash: &[u8],
	recipient: &RecipientKeypair,
) -> Result<(Vec<u8>, std::time::Duration)> {
	let hash_hex = hex0x(hash);
	let payload = op_signing_payload(HOP_CLAIM_CONTEXT, hash);
	let signature = recipient.sign_multi_signature(&payload);
	let sig_hex = hex0x(&signature);

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
