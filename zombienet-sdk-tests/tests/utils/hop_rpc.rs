// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Raw HOP JSON-RPC client (`hop_submit`, `hop_claim`, `hop_ack`, `hop_poolStatus`).
//! Mirrors the on-chain construction of `MultiSigner`/`MultiSignature` + the signing
//! payloads so the test driver can stand in for `sc-hop`'s SDK side without pulling
//! it in.

use super::crypto::blake2_256;
use anyhow::{anyhow, Context, Result};
use subxt::{
	backend::rpc::RpcClient,
	ext::subxt_rpcs::{
		client::{rpc_params, RpcParams},
		Error as RpcError,
	},
};
use subxt_signer::sr25519::Keypair;

/// Must remain byte-identical to `pallet-bulletin-hop-promotion::HOP_SUBMIT_CONTEXT`.
const HOP_SUBMIT_CONTEXT: &[u8] = b"hop-submit-v1:";
/// Must remain byte-identical to `sc_hop::types::HOP_CLAIM_CONTEXT`.
const HOP_CLAIM_CONTEXT: &[u8] = b"hop-claim-v1:";
/// Must remain byte-identical to `sc_hop::types::HOP_ACK_CONTEXT`.
const HOP_ACK_CONTEXT: &[u8] = b"hop-ack-v1:";

pub const HOP_POOL_FULL_ERROR_CODE: i32 = 1002;
pub const HOP_USER_QUOTA_ERROR_CODE: i32 = 1011;
pub const HOP_RATE_LIMIT_ERROR_CODE: i32 = 1020;

/// `blake2_256(HOP_SUBMIT_CONTEXT || blake2_256(data) || submit_timestamp_ms.to_le_bytes())` —
/// must remain byte-identical to the pallet's reconstruction.
pub(super) fn submit_signing_payload(data_hash: &[u8; 32], submit_timestamp_ms: u64) -> [u8; 32] {
	let mut buf = Vec::with_capacity(HOP_SUBMIT_CONTEXT.len() + 32 + 8);
	buf.extend_from_slice(HOP_SUBMIT_CONTEXT);
	buf.extend_from_slice(data_hash);
	buf.extend_from_slice(&submit_timestamp_ms.to_le_bytes());
	blake2_256(&buf)
}

/// `blake2_256(context || content_hash)` — `sc-hop`'s claim/ack signing payload. The
/// signer must be one of the recipients named at submit time.
fn hash_signing_payload(context: &[u8], content_hash: &[u8; 32]) -> [u8; 32] {
	let mut buf = Vec::with_capacity(context.len() + 32);
	buf.extend_from_slice(context);
	buf.extend_from_slice(content_hash);
	blake2_256(&buf)
}

/// SCALE: variant index 1 + raw bytes. Used for both `MultiSigner::Sr25519`
/// (1 + 32 bytes) and `MultiSignature::Sr25519` (1 + 64 bytes).
fn sr25519_scale(bytes: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(1 + bytes.len());
	out.push(1u8);
	out.extend_from_slice(bytes);
	out
}

fn hex0x(bytes: &[u8]) -> String {
	format!("0x{}", hex::encode(bytes))
}

fn decode_hex(what: &str, hex: &str) -> Result<Vec<u8>> {
	let hex_digits = hex.strip_prefix("0x").unwrap_or(hex);
	hex::decode(hex_digits).with_context(|| format!("{what} returned non-hex payload"))
}

pub fn hop_rpc_error_code(error: &anyhow::Error) -> Option<i32> {
	match error.downcast_ref::<RpcError>()? {
		RpcError::User(user_error) => Some(user_error.code),
		_ => None,
	}
}

/// Submit `data` to a collator's HOP data pool via the `hop_submit` JSON-RPC. Returns
/// the pool's entry count after insertion.
pub async fn hop_submit(
	rpc: &RpcClient,
	signer: &Keypair,
	data: &[u8],
	recipients: &[[u8; 32]],
	submit_timestamp_ms: u64,
) -> Result<u64> {
	let data_hash = blake2_256(data);
	let payload = submit_signing_payload(&data_hash, submit_timestamp_ms);
	let signature_scale = sr25519_scale(&signer.sign(&payload).0);
	let signer_scale = sr25519_scale(&signer.public_key().0);
	let recipients_hex: Vec<String> = recipients.iter().map(|r| hex0x(&sr25519_scale(r))).collect();

	let mut params: RpcParams = rpc_params![hex0x(data), recipients_hex];
	for p in [hex0x(&signature_scale), hex0x(&signer_scale)] {
		params.push(p).map_err(|e| anyhow!("encode param: {e}"))?;
	}
	params.push(submit_timestamp_ms).map_err(|e| anyhow!("encode ts param: {e}"))?;

	let value: serde_json::Value =
		rpc.request("hop_submit", params).await.context("hop_submit RPC")?;
	value
		.get("poolStatus")
		.and_then(|p| p.get("entryCount"))
		.and_then(|n| n.as_u64())
		.ok_or_else(|| anyhow!("hop_submit response missing poolStatus.entryCount: {value}"))
}

/// `hop_poolStatus` response. Field names are camelCase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HopPoolStatus {
	pub entry_count: u64,
	pub total_bytes: u64,
	pub max_bytes: u64,
}

/// Read a collator's HOP pool status via `hop_poolStatus`.
pub async fn hop_pool_status(rpc: &RpcClient) -> Result<HopPoolStatus> {
	let value: serde_json::Value = rpc
		.request("hop_poolStatus", rpc_params![])
		.await
		.context("hop_poolStatus RPC")?;

	let field = |name: &str| {
		value
			.get(name)
			.and_then(serde_json::Value::as_u64)
			.ok_or_else(|| anyhow!("hop_poolStatus response missing {name}: {value}"))
	};
	Ok(HopPoolStatus {
		entry_count: field("entryCount")?,
		total_bytes: field("totalBytes")?,
		max_bytes: field("maxBytes")?,
	})
}

/// Claim a blob by content hash via `hop_claim`, signing as `recipient`. Read-only:
/// the recipient is not marked claimed until [`hop_ack`]. Returns the blob bytes.
pub async fn hop_claim(
	rpc: &RpcClient,
	recipient: &Keypair,
	content_hash: &[u8; 32],
) -> Result<Vec<u8>> {
	let payload = hash_signing_payload(HOP_CLAIM_CONTEXT, content_hash);
	let signature_scale = sr25519_scale(&recipient.sign(&payload).0);

	let hex: String = rpc
		.request("hop_claim", rpc_params![hex0x(content_hash), hex0x(&signature_scale)])
		.await
		.context("hop_claim RPC")?;
	decode_hex("hop_claim", &hex)
}

/// `hop_claim` with an arbitrary-length `raw_hash`, to exercise the RPC's hash decode.
/// The signature is irrelevant: `decode_hash` runs first.
pub async fn hop_claim_raw(rpc: &RpcClient, raw_hash: &[u8]) -> Result<Vec<u8>> {
	let hex: String = rpc
		.request("hop_claim", rpc_params![hex0x(raw_hash), hex0x(&sr25519_scale(&[0u8; 64]))])
		.await
		.context("hop_claim RPC")?;
	decode_hex("hop_claim", &hex)
}

/// Acknowledge receipt via `hop_ack`, signing as `recipient`. Marks the recipient
/// claimed; the entry is removed once every recipient has acked.
pub async fn hop_ack(rpc: &RpcClient, recipient: &Keypair, content_hash: &[u8; 32]) -> Result<()> {
	let payload = hash_signing_payload(HOP_ACK_CONTEXT, content_hash);
	let signature_scale = sr25519_scale(&recipient.sign(&payload).0);

	rpc.request::<serde_json::Value>(
		"hop_ack",
		rpc_params![hex0x(content_hash), hex0x(&signature_scale)],
	)
	.await
	.context("hop_ack RPC")?;
	Ok(())
}

/// Wall-clock now in milliseconds since the unix epoch. Bound into the submit
/// signing payload; the runtime rejects promotions whose timestamp falls outside
/// `SubmitTimestampTolerance` of on-chain time.
pub fn now_ms() -> u64 {
	use std::time::{SystemTime, UNIX_EPOCH};
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_millis() as u64)
		.unwrap_or(0)
}
