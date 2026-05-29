// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Raw `hop_submit` JSON-RPC client. Mirrors the on-chain construction of
//! `MultiSigner`/`MultiSignature` + the submit signing payload so the test
//! driver can stand in for `sc-hop`'s SDK side without pulling it in.

use super::crypto::blake2_256;
use anyhow::{anyhow, Result};
use subxt::{
	backend::rpc::RpcClient,
	ext::subxt_rpcs::client::{rpc_params, RpcParams},
};
use subxt_signer::sr25519::Keypair;

/// Must remain byte-identical to `pallet-bulletin-hop-promotion::HOP_SUBMIT_CONTEXT`.
const HOP_SUBMIT_CONTEXT: &[u8] = b"hop-submit-v1:";

/// `blake2_256(HOP_SUBMIT_CONTEXT || blake2_256(data) || submit_timestamp.to_le_bytes())` —
/// must remain byte-identical to the pallet's reconstruction; the runtime re-verifies the
/// user's signature against this exact byte sequence.
pub fn submit_signing_payload(data_hash: &[u8; 32], submit_timestamp_ms: u64) -> [u8; 32] {
	let mut buf = Vec::with_capacity(HOP_SUBMIT_CONTEXT.len() + 32 + 8);
	buf.extend_from_slice(HOP_SUBMIT_CONTEXT);
	buf.extend_from_slice(data_hash);
	buf.extend_from_slice(&submit_timestamp_ms.to_le_bytes());
	blake2_256(&buf)
}

/// SCALE-encoded `MultiSigner::Sr25519(pub_key)` — variant index 1 + 32 raw bytes.
pub fn encode_multi_signer_sr25519(pub_key: &[u8; 32]) -> Vec<u8> {
	let mut out = Vec::with_capacity(1 + 32);
	out.push(1u8);
	out.extend_from_slice(pub_key);
	out
}

/// SCALE-encoded `MultiSignature::Sr25519(sig)` — variant index 1 + 64 raw bytes.
pub fn encode_multi_signature_sr25519(sig: &[u8; 64]) -> Vec<u8> {
	let mut out = Vec::with_capacity(1 + 64);
	out.push(1u8);
	out.extend_from_slice(sig);
	out
}

/// Submit `data` to a collator's HOP data pool via the `hop_submit` JSON-RPC.
/// The submission is signed by `signer` and lists `recipients` as the intended
/// claimants. Returns the pool's reported entry count after insertion.
pub async fn hop_submit(
	ws_uri: &str,
	signer: &Keypair,
	data: &[u8],
	recipients: &[[u8; 32]],
	submit_timestamp_ms: u64,
) -> Result<u64> {
	let rpc = RpcClient::from_insecure_url(ws_uri)
		.await
		.map_err(|e| anyhow!("connect {ws_uri}: {e}"))?;

	let data_hash = blake2_256(data);
	let payload = submit_signing_payload(&data_hash, submit_timestamp_ms);
	let sig_bytes = signer.sign(&payload).0;

	let signer_encoded = encode_multi_signer_sr25519(&signer.public_key().0);
	let signature_encoded = encode_multi_signature_sr25519(&sig_bytes);

	let recipients_encoded: Vec<String> = recipients
		.iter()
		.map(|r| format!("0x{}", hex::encode(encode_multi_signer_sr25519(r))))
		.collect();

	let mut params: RpcParams = rpc_params![format!("0x{}", hex::encode(data)), recipients_encoded];
	params
		.push(format!("0x{}", hex::encode(&signature_encoded)))
		.map_err(|e| anyhow!("encode signature param: {e}"))?;
	params
		.push(format!("0x{}", hex::encode(&signer_encoded)))
		.map_err(|e| anyhow!("encode signer param: {e}"))?;
	params
		.push(submit_timestamp_ms)
		.map_err(|e| anyhow!("encode timestamp param: {e}"))?;

	let value: serde_json::Value = rpc
		.request("hop_submit", params)
		.await
		.map_err(|e| anyhow!("hop_submit RPC: {e}"))?;
	let entry_count = value
		.get("poolStatus")
		.and_then(|p| p.get("entryCount"))
		.and_then(|n| n.as_u64())
		.ok_or_else(|| anyhow!("hop_submit response missing poolStatus.entryCount: {value}"))?;
	Ok(entry_count)
}

/// Wall-clock now in milliseconds since the unix epoch. Used as the `submit_timestamp`
/// bound into the user's signature; the runtime rejects promotions whose timestamp
/// falls outside `SubmitTimestampTolerance` of on-chain time.
pub fn now_ms() -> u64 {
	use std::time::{SystemTime, UNIX_EPOCH};
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_millis() as u64)
		.unwrap_or(0)
}
