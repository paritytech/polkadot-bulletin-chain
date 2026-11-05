// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Raw HOP JSON-RPC client (`hop_submit`, `hop_claim`, `hop_ack`, `hop_poolStatus`).
//! Uses [`bulletin_hop_rpc_client`] for signing payloads and SCALE encoding so
//! stress-test and zombienet drivers share one implementation.

use anyhow::{anyhow, Context, Result};
use bulletin_hop_rpc_client::{
	hex0x, now_ms, op_signing_payload, scale_multi_signature_sr25519, scale_multi_signer_sr25519,
	sr25519_scale, submit_signing_payload as client_submit_signing_payload,
	submit_signing_payload_from_hash, HOP_ACK_CONTEXT, HOP_CLAIM_CONTEXT,
};
use subxt::{
	backend::rpc::RpcClient,
	ext::subxt_rpcs::{
		client::{rpc_params, RpcParams},
		Error as RpcError,
	},
};
use subxt_signer::sr25519::Keypair;

pub const HOP_POOL_FULL_ERROR_CODE: i32 = 1002;
pub const HOP_USER_QUOTA_ERROR_CODE: i32 = 1011;
pub const HOP_RATE_LIMIT_ERROR_CODE: i32 = 1020;

/// `blake2_256(HOP_SUBMIT_CONTEXT || data_hash || submit_timestamp_ms.to_le_bytes())` —
/// delegates to [`bulletin_hop_rpc_client`].
pub(super) fn submit_signing_payload(data_hash: &[u8; 32], submit_timestamp_ms: u64) -> [u8; 32] {
	submit_signing_payload_from_hash(data_hash, submit_timestamp_ms)
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
	let payload = client_submit_signing_payload(data, submit_timestamp_ms);
	let signature_scale = scale_multi_signature_sr25519(&signer.sign(&payload).0);
	let signer_scale = scale_multi_signer_sr25519(&signer.public_key().0);
	let recipients_hex: Vec<String> = recipients.iter().map(|r| hex0x(&scale_multi_signer_sr25519(r))).collect();

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
	let payload = op_signing_payload(HOP_CLAIM_CONTEXT, content_hash);
	let signature_scale = scale_multi_signature_sr25519(&recipient.sign(&payload).0);

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
	let payload = op_signing_payload(HOP_ACK_CONTEXT, content_hash);
	let signature_scale = scale_multi_signature_sr25519(&recipient.sign(&payload).0);

	rpc.request::<serde_json::Value>(
		"hop_ack",
		rpc_params![hex0x(content_hash), hex0x(&signature_scale)],
	)
	.await
	.context("hop_ack RPC")?;
	Ok(())
}

pub fn content_hash(data: &[u8]) -> [u8; 32] {
	bulletin_hop_rpc_client::blake2_256(data)
}

pub use bulletin_hop_rpc_client::now_ms;
