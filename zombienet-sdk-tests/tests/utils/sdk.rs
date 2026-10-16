// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{anyhow, Context, Result};
use bulletin_sdk_rust::{TransactionClient, WaitFor};
use std::str::FromStr;
use subxt::{backend::rpc::RpcClient, utils::H256};

pub async fn sdk_client(node: &zombienet_sdk::NetworkNode) -> Result<TransactionClient> {
	let rpc = RpcClient::from_insecure_url(node.ws_uri())
		.await
		.with_context(|| format!("connect the SDK client to {}", node.ws_uri()))?;
	Ok(TransactionClient::from_rpc_client(rpc).await?.with_wait_for(WaitFor::InBlock))
}

pub fn receipt_block_hash(block_hash: &str) -> Result<H256> {
	H256::from_str(block_hash)
		.map_err(|error| anyhow!("invalid receipt block hash '{block_hash}': {error}"))
}
