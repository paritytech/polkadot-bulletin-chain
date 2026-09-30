// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! `sp_hop::HopRuntimeApi` client.
//!
//! Calls go through subxt's runtime API layer on the test's existing `OnlineClient`, so
//! arguments are encoded from the chain's own metadata: an unknown trait or method, or a
//! changed argument list, fails here instead of silently mis-encoding positional bytes.

use super::hop_rpc::submit_signing_payload;
use anyhow::{anyhow, Result};
use subxt::{
	config::substrate::SubstrateConfig,
	dynamic::Value,
	ext::{codec::Decode, scale_value::Composite},
	OnlineClient,
};
use subxt_signer::sr25519::Keypair;

const HOP_API: &str = "HopRuntimeApi";

/// `MultiSigner`/`MultiSignature` are enums; both use variant `Sr25519` over raw bytes.
fn sr25519_variant(bytes: &[u8]) -> Value {
	Value::variant("Sr25519", Composite::unnamed(vec![Value::from_bytes(bytes)]))
}

async fn call(
	client: &OnlineClient<SubstrateConfig>,
	method: &str,
	args: Vec<Value>,
) -> Result<subxt::dynamic::DecodedValueThunk> {
	let payload = subxt::dynamic::runtime_api_call(HOP_API, method, args);
	client
		.runtime_api()
		.at_latest()
		.await
		.map_err(|e| anyhow!("runtime_api at_latest: {e}"))?
		.call(payload)
		.await
		.map_err(|e| anyhow!("{HOP_API}_{method}: {e}"))
}

/// `HopRuntimeApi::can_account_promote`. `data_len` is accepted but ignored by the
/// pallet, which only requires an unexpired authorization to exist.
pub async fn can_account_promote(
	client: &OnlineClient<SubstrateConfig>,
	account: &[u8; 32],
	data_len: u32,
) -> Result<bool> {
	let args = vec![Value::from_bytes(account), Value::u128(data_len as u128)];
	call(client, "can_account_promote", args)
		.await?
		.as_type::<bool>()
		.map_err(|e| anyhow!("decode can_account_promote: {e}"))
}

/// `HopRuntimeApi::max_promotion_size`.
pub async fn max_promotion_size(client: &OnlineClient<SubstrateConfig>) -> Result<u32> {
	call(client, "max_promotion_size", vec![])
		.await?
		.as_type::<u32>()
		.map_err(|e| anyhow!("decode max_promotion_size: {e}"))
}

/// `HopRuntimeApi::is_promoted_on_chain`.
pub async fn is_promoted_on_chain(
	client: &OnlineClient<SubstrateConfig>,
	content_hash: &[u8; 32],
) -> Result<bool> {
	call(client, "is_promoted_on_chain", vec![Value::from_bytes(content_hash)])
		.await?
		.as_type::<bool>()
		.map_err(|e| anyhow!("decode is_promoted_on_chain: {e}"))
}

/// `HopRuntimeApi::create_promotion_extrinsic`, returning the encoded extrinsic.
///
/// The return type is `Block::Extrinsic`, which SCALE-encodes length-prefixed like a
/// `Vec<u8>`; decoding the raw bytes avoids depending on how the opaque extrinsic is
/// shaped in metadata.
pub async fn create_promotion_extrinsic(
	client: &OnlineClient<SubstrateConfig>,
	signer: &Keypair,
	data: &[u8],
	submit_timestamp_ms: u64,
) -> Result<Vec<u8>> {
	let data_hash = super::crypto::blake2_256(data);
	let payload = submit_signing_payload(&data_hash, submit_timestamp_ms);
	let args = vec![
		Value::from_bytes(data),
		sr25519_variant(&signer.public_key().0),
		sr25519_variant(&signer.sign(&payload).0),
		Value::u128(submit_timestamp_ms as u128),
	];

	let thunk = call(client, "create_promotion_extrinsic", args).await?;
	Vec::<u8>::decode(&mut thunk.encoded())
		.map_err(|e| anyhow!("decode create_promotion_extrinsic: {e}"))
}
