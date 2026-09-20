// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Shared HOP JSON-RPC signing payloads and `MultiSigner` / `MultiSignature` SCALE
//! encoding used by Bulletin stress and zombienet test drivers.

pub use pallet_bulletin_hop_promotion::HOP_SUBMIT_CONTEXT;

/// Domain separator for `hop_claim` / `hop_ack` recipient signatures.
pub const HOP_CLAIM_CONTEXT: &[u8] = b"hop-claim-v1:";

pub fn blake2_256(data: &[u8]) -> [u8; 32] {
	use blake2::{digest::consts::U32, Blake2b, Digest};
	let mut hasher = Blake2b::<U32>::new();
	hasher.update(data);
	let result = hasher.finalize();
	let mut output = [0u8; 32];
	output.copy_from_slice(&result);
	output
}

/// `blake2_256(HOP_SUBMIT_CONTEXT || blake2_256(data) || submit_timestamp.to_le_bytes())`.
pub fn submit_signing_payload(data: &[u8], submit_timestamp_ms: u64) -> [u8; 32] {
	let data_hash = blake2_256(data);
	pallet_bulletin_hop_promotion::signing_payload(&data_hash, submit_timestamp_ms)
}

/// `blake2_256(context || hash)` — recipients sign this for claim/ack operations.
pub fn op_signing_payload(context: &[u8], hash: &[u8]) -> [u8; 32] {
	let mut buf = Vec::with_capacity(context.len() + hash.len());
	buf.extend_from_slice(context);
	buf.extend_from_slice(hash);
	blake2_256(&buf)
}

/// SCALE-encoded `MultiSigner::Ed25519(pubkey)`.
pub fn scale_multi_signer_ed25519(public_key: &[u8; 32]) -> Vec<u8> {
	let mut buf = Vec::with_capacity(33);
	buf.push(0x00);
	buf.extend_from_slice(public_key);
	buf
}

/// SCALE-encoded `MultiSignature::Ed25519(sig)`.
pub fn scale_multi_signature_ed25519(signature: &[u8; 64]) -> Vec<u8> {
	let mut buf = Vec::with_capacity(65);
	buf.push(0x00);
	buf.extend_from_slice(signature);
	buf
}

/// SCALE-encoded `MultiSigner::Sr25519(pubkey)`.
pub fn scale_multi_signer_sr25519(public_key: &[u8; 32]) -> Vec<u8> {
	sr25519_scale(public_key)
}

/// SCALE-encoded `MultiSignature::Sr25519(sig)`.
pub fn scale_multi_signature_sr25519(signature: &[u8; 64]) -> Vec<u8> {
	sr25519_scale(signature)
}

/// SCALE variant index 1 + raw bytes (Sr25519 signer or signature).
pub fn sr25519_scale(bytes: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(1 + bytes.len());
	out.push(1u8);
	out.extend_from_slice(bytes);
	out
}

pub fn hex0x(bytes: &[u8]) -> String {
	format!("0x{}", hex::encode(bytes))
}

pub fn now_ms() -> u64 {
	use std::time::{SystemTime, UNIX_EPOCH};
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_millis() as u64)
		.unwrap_or(0)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn submit_signing_payload_matches_pallet() {
		let data = b"hop-payload-test";
		let ts = 1_700_000_000_123u64;
		let data_hash = blake2_256(data);
		assert_eq!(
			submit_signing_payload(data, ts),
			pallet_bulletin_hop_promotion::signing_payload(&data_hash, ts),
		);
	}

	#[test]
	fn sr25519_scale_prefixes_variant_index() {
		let key = [7u8; 32];
		let encoded = scale_multi_signer_sr25519(&key);
		assert_eq!(encoded[0], 1);
		assert_eq!(&encoded[1..], &key);
	}
}
