//! A collection of node-specific RPC methods.
//! Substrate provides the `sc-rpc` crate, which defines the core RPC layer
//! used by Substrate nodes. This file extends those RPC definitions with
//! capabilities that are specific to this project's runtime configuration.

#![warn(missing_docs)]

use std::sync::Arc;

use crate::node_primitives::{AccountId, Block, BlockNumber, Hash, Nonce};
use jsonrpsee::RpcModule;
use crate::sc_consensus_babe::{BabeApi, BabeWorkerHandle};
use crate::sc_consensus_grandpa::{
	FinalityProofProvider, GrandpaJustificationStream, SharedAuthoritySet, SharedVoterState,
};
use crate::sc_consensus_grandpa_rpc::{Grandpa, GrandpaApiServer};
use crate::sc_rpc::SubscriptionTaskExecutor;
use crate::sc_sync_state_rpc::{SyncState, SyncStateApiServer};
use crate::sc_transaction_pool_api::TransactionPool;
use crate::sp_api::ProvideRuntimeApi;
use crate::sp_block_builder::BlockBuilder;
use crate::sp_blockchain::{Error as BlockChainError, HeaderBackend, HeaderMetadata};
use crate::sp_consensus::SelectChain;
use crate::sp_keystore::KeystorePtr;

/// Full client dependencies.
pub struct FullDeps<C, P, SC, B> {
	/// The client instance to use.
	pub client: Arc<C>,
	/// Transaction pool instance.
	pub pool: Arc<P>,
	/// The chain selection strategy.
	pub select_chain: SC,
	/// A copy of the chain spec.
	pub chain_spec: Box<dyn crate::sc_chain_spec::ChainSpec>,
	/// BABE RPC dependencies.
	pub babe: BabeDeps,
	/// GRANDPA RPC dependencies.
	pub grandpa: GrandpaDeps<B>,
}

/// BABE RPC dependencies.
pub struct BabeDeps {
	/// A handle to the BABE worker for issuing requests.
	pub babe_worker_handle: BabeWorkerHandle<Block>,
	/// The keystore that manages the keys of the node.
	pub keystore: KeystorePtr,
}

/// GRANDPA RPC dependncies.
pub struct GrandpaDeps<B> {
	/// Subscription task executor.
	pub subscription_executor: SubscriptionTaskExecutor,
	/// GRANDPA authority set.
	pub shared_authority_set: SharedAuthoritySet<Hash, BlockNumber>,
	/// GRANDPA voter state.
	pub shared_voter_state: SharedVoterState,
	/// GRANDPA justifications.
	pub justification_stream: GrandpaJustificationStream<Block>,
	/// Finality proof provider.
	pub finality_proof_provider: Arc<FinalityProofProvider<B, Block>>,
}

/// Instantiate all full RPC extensions.
pub fn create_full<C, P, SC, B>(
	deps: FullDeps<C, P, SC, B>,
) -> Result<RpcModule<()>, Box<dyn std::error::Error + Send + Sync>>
where
	C: ProvideRuntimeApi<Block>,
	C: HeaderBackend<Block> + HeaderMetadata<Block, Error = BlockChainError> + 'static,
	C: Send + Sync + 'static + crate::sc_client_api::AuxStore,
	C::Api: crate::substrate_frame_rpc_system::AccountNonceApi<Block, AccountId, Nonce>,
	C::Api: BlockBuilder<Block>,
	C::Api: BabeApi<Block>,
	P: TransactionPool + 'static,
	SC: SelectChain<Block> + 'static,
	B: crate::sc_client_api::Backend<Block> + Send + Sync + 'static,
{
	use crate::sc_consensus_babe_rpc::{Babe, BabeApiServer};
	use crate::substrate_frame_rpc_system::{System, SystemApiServer};

	let mut module = RpcModule::new(());
	let FullDeps { client, pool, select_chain, chain_spec, babe, grandpa } = deps;
	let BabeDeps { babe_worker_handle, keystore } = babe;

	module.merge(System::new(client.clone(), pool).into_rpc())?;
	module.merge(
		Babe::new(client.clone(), babe_worker_handle.clone(), keystore, select_chain).into_rpc(),
	)?;
	module.merge(
		Grandpa::new(
			grandpa.subscription_executor,
			grandpa.shared_authority_set.clone(),
			grandpa.shared_voter_state,
			grandpa.justification_stream,
			grandpa.finality_proof_provider,
		)
		.into_rpc(),
	)?;
	module.merge(
		SyncState::new(chain_spec, client, grandpa.shared_authority_set, babe_worker_handle)?
			.into_rpc(),
	)?;

	// Extend this RPC with a custom API by using the following syntax.
	// `YourRpcStruct` should have a reference to a client, which is needed
	// to call into the runtime.
	// `module.merge(YourRpcTrait::into_rpc(YourRpcStruct::new(ReferenceToClient, ...)))?;`

	Ok(module)
}
