// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Raw `hop_submit` JSON-RPC client. Uses [`bulletin_hop_rpc_client`] for signing payloads
//! and SCALE encoding so stress-test and zombienet drivers share one implementation.

use bulletin_hop_rpc_client::{
	hex0x, scale_multi_signature_sr25519, scale_multi_signer_sr25519, submit_signing_payload,
};
use anyhow::{anyhow, Result};
use subxt::{
	backend::rpc::RpcClient,
	ext::subxt_rpcs::client::{rpc_params, RpcParams},
};
use subxt_signer::sr25519::Keypair;

/// Submit `data` to a collator's HOP data pool via the `hop_submit` JSON-RPC. Returns
/// the pool's entry count after insertion.
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

	let payload = submit_signing_payload(data, submit_timestamp_ms);
	let signature_scale = scale_multi_signature_sr25519(&signer.sign(&payload).0);
	let signer_scale = scale_multi_signer_sr25519(&signer.public_key().0);
	let recipients_hex: Vec<String> = recipients.iter().map(|r| hex0x(&scale_multi_signer_sr25519(r))).collect();

	let mut params: RpcParams = rpc_params![hex0x(data), recipients_hex];
	for p in [hex0x(&signature_scale), hex0x(&signer_scale)] {
		params.push(p).map_err(|e| anyhow!("encode param: {e}"))?;
	}
	params.push(submit_timestamp_ms).map_err(|e| anyhow!("encode ts param: {e}"))?;

	let value: serde_json::Value = rpc
		.request("hop_submit", params)
		.await
		.map_err(|e| anyhow!("hop_submit RPC: {e}"))?;
	value
		.get("poolStatus")
		.and_then(|p| p.get("entryCount"))
		.and_then(|n| n.as_u64())
		.ok_or_else(|| anyhow!("hop_submit response missing poolStatus.entryCount: {value}"))
}

pub fn content_hash(data: &[u8]) -> [u8; 32] {
	bulletin_hop_rpc_client::blake2_256(data)
}

pub use bulletin_hop_rpc_client::now_ms;
