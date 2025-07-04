//! A collection of node-specific RPC methods.
//! Substrate provides the `sc-rpc` crate, which defines the core RPC layer
//! used by Substrate nodes. This file extends those RPC definitions with
//! capabilities that are specific to this project's runtime configuration.

#![warn(missing_docs)]

use std::sync::Arc;

use jsonrpsee::RpcModule;
use polkadot_bulletin_chain_runtime::{opaque::Block, AccountId, BlockNumber, Hash, Nonce};
use sc_consensus_grandpa::{
	FinalityProofProvider, GrandpaJustificationStream, SharedAuthoritySet, SharedVoterState,
};
use sc_consensus_grandpa_rpc::{Grandpa, GrandpaApiServer};
use sc_rpc::SubscriptionTaskExecutor;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_block_builder::BlockBuilder;
use sp_blockchain::{Error as BlockChainError, HeaderBackend, HeaderMetadata};

/// Full client dependencies.
pub struct FullDeps<C, P, B> {
	/// The client instance to use.
	pub client: Arc<C>,
	/// Transaction pool instance.
	pub pool: Arc<P>,
	/// GRANDPA RPC dependencies.
	pub grandpa: GrandpaDeps<B>,
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
pub fn create_full<C, P, B>(
	deps: FullDeps<C, P, B>,
) -> Result<RpcModule<()>, Box<dyn std::error::Error + Send + Sync>>
where
	C: ProvideRuntimeApi<Block>,
	C: HeaderBackend<Block> + HeaderMetadata<Block, Error = BlockChainError> + 'static,
	C: Send + Sync + 'static,
	C::Api: substrate_frame_rpc_system::AccountNonceApi<Block, AccountId, Nonce>,
	C::Api: BlockBuilder<Block>,
	P: TransactionPool + 'static,
	B: sc_client_api::Backend<Block> + Send + Sync + 'static,
{
	use substrate_frame_rpc_system::{System, SystemApiServer};

	let mut module = RpcModule::new(());
	let FullDeps { client, pool, grandpa } = deps;

	module.merge(System::new(client, pool).into_rpc())?;
	module.merge(
		Grandpa::new(
			grandpa.subscription_executor,
			grandpa.shared_authority_set,
			grandpa.shared_voter_state,
			grandpa.justification_stream,
			grandpa.finality_proof_provider,
		)
		.into_rpc(),
	)?;

	// Extend this RPC with a custom API by using the following syntax.
	// `YourRpcStruct` should have a reference to a client, which is needed
	// to call into the runtime.
	// `module.merge(YourRpcTrait::into_rpc(YourRpcStruct::new(ReferenceToClient, ...)))?;`

	Ok(module)
}
