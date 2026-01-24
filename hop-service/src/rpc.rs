// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! HOP (Hand-Off protocol) RPC interface implementation.

use crate::{
	pool::HopDataPool,
	primitives::HopHash,
	types::PoolStatus,
};
use jsonrpsee::{
	core::{async_trait, RpcResult},
	proc_macros::rpc,
	types::ErrorObjectOwned,
};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_core::Bytes;
use sp_runtime::{traits::Block as BlockT, SaturatedConversion};
use std::sync::Arc;

/// HOP RPC methods.
#[rpc(client, server)]
pub trait HopApi<BlockHash> {
	/// Submit data to the data pool
	///
	/// # Arguments
	/// * `data`: The data to store, in bytes
	///
	/// # Returns
	/// The hash of the data, in bytes
	#[method(name = "hop_submit")]
	fn submit(&self, data: Bytes) -> RpcResult<Bytes>;

	/// Get some data from the data pool by hash, delete it afterwards
	///
	/// # Arguments
	/// * `hash`: The hash of the data, in bytes
	///
	/// # Returns
	/// Some(data) if it's present in the pool, None if not
	#[method(name = "hop_get")]
	fn get(&self, hash: Bytes) -> RpcResult<Option<Bytes>>;

	/// Check if some data exists in the data pool
	///
	/// # Arguments
	/// * `hash`: The hash of the data, in bytes
	///
	/// # Returns
	/// Whether the data exists or not in the pool
	#[method(name = "hop_has")]
	fn has(&self, hash: Bytes) -> RpcResult<bool>;

	/// Get data pool status
	///
	/// # Returns
	/// Pool statistics including entry count and size
	#[method(name = "hop_poolStatus")]
	fn pool_status(&self) -> RpcResult<PoolStatus>;
}

/// HOP RPC server implementation.
pub struct HopRpcServer<C, Block> {
	pool: Arc<HopDataPool>,
	client: Arc<C>,
	_phantom: std::marker::PhantomData<Block>,
}

impl<C, Block> HopRpcServer<C, Block> {
	/// Create a new HOP RPC server.
	pub fn new(pool: Arc<HopDataPool>, client: Arc<C>) -> Self {
		Self { pool, client, _phantom: Default::default() }
	}

	/// Convert Bytes to Hash with validation
	fn bytes_to_hash(bytes: Bytes) -> RpcResult<HopHash> {
		let hash_bytes: [u8; 32] = bytes.0.as_slice().try_into().map_err(|_| {
			ErrorObjectOwned::owned(
				1008,
				format!("Invalid hash length: expected 32 bytes, got {}", bytes.0.len()),
				None::<()>,
			)
		})?;
		Ok(HopHash::from(hash_bytes))
	}
}

#[async_trait]
impl<C, Block> HopApiServer<<Block as BlockT>::Hash> for HopRpcServer<C, Block>
where
	Block: BlockT,
	C: HeaderBackend<Block> + ProvideRuntimeApi<Block> + Send + Sync + 'static,
{
	fn submit(&self, data: Bytes) -> RpcResult<Bytes> {
		// We need the current block number to know when the timeout is reached.
		let current_block = self.client.info().best_number.saturated_into::<u32>();
		let hash = self.pool.insert(data.0, current_block)?;
		Ok(Bytes(hash.0.to_vec()))
	}

	fn get(&self, hash: Bytes) -> RpcResult<Option<Bytes>> {
		let hash = Self::bytes_to_hash(hash)?;
		let data = self.pool.get(&hash).map(|data| Bytes(data));
		// We delete the data when someone requests it.
		// TODO: Make sure we only delete it when its intended recipient retrieves it.
		self.pool.remove(&hash)?;
		Ok(data)
	}

	fn has(&self, hash: Bytes) -> RpcResult<bool> {
		let hash_array = Self::bytes_to_hash(hash)?;
		Ok(self.pool.has(&hash_array))
	}

	fn pool_status(&self) -> RpcResult<PoolStatus> {
		Ok(self.pool.status())
	}
}
