// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Transaction submission for Bulletin Chain operations.
//!
//! This module provides the actual blockchain interaction layer using subxt.

use crate::{
	authorization::{Authorization, AuthorizationManager},
	blob_source::{
		plan_stream, ChunkPlan, GetData, ItemData, SeekableSource, SkipReason, StreamEstimate,
		UploadEstimate, UploadEstimateItem, UploadEstimateOptions,
	},
	cid::{calculate_cid_with_config, cid_to_bytes, CidCodec, ContentHash, HashingAlgorithm},
	compat,
	pipeline::{run_resolved, ResolvedItem, UploadConfig, UploadResult},
	types::{AuthorizationScope, Error, ProgressCallback, ProgressEvent, Result, WaitFor},
};
use bulletin_transaction_storage_primitives::TransactionRef;
use std::{
	collections::{BTreeSet, HashMap, HashSet},
	sync::Arc,
};
use subxt::{
	backend::{legacy::LegacyRpcMethods, rpc::RpcClient},
	blocks::BlockRef,
	config::DefaultExtrinsicParamsBuilder,
	utils::AccountId32,
	OnlineClient, PolkadotConfig,
};
use subxt_signer::sr25519::Keypair;

// Subxt metadata for TransactionStorage pallet
#[subxt::subxt(runtime_metadata_path = "../metadata.scale")]
pub mod bulletin {}

/// Convert the primitives `TransactionRef` into the subxt-generated one.
fn to_runtime_ref(
	entry: &TransactionRef<u32>,
) -> bulletin::runtime_types::bulletin_transaction_storage_primitives::TransactionRef<u32> {
	use bulletin::runtime_types::bulletin_transaction_storage_primitives::TransactionRef as Gen;
	match entry {
		TransactionRef::Position { block, index } => Gen::Position { block: *block, index: *index },
		TransactionRef::ContentHash(hash) => Gen::ContentHash(*hash),
	}
}

/// Input to [`TransactionClient::estimate_upload`]: either in-memory items (each
/// stored as its own unit, no manifest) or a re-openable byte source (chunked
/// into a DAG-PB file). Mirrors the TS `estimateUpload(UploadItem[] | BlobSource)`.
pub enum UploadInput {
	Items(Vec<crate::pipeline::UploadItem>),
	Source(Arc<dyn SeekableSource>),
}

/// Transaction submission client for Bulletin Chain.
///
/// This wraps a subxt OnlineClient and provides high-level methods
/// for all TransactionStorage pallet operations.
///
/// Resolves the next nonce from `system_accountNextIndex` (pool-aware) before
/// each submission. That RPC reflects both already-included and pool-pending
/// transactions, so sequential submissions never collide on a nonce — unlike a
/// best-block `account_nonce` read, which can return a stale value when the
/// previous tx is still in the pool or its block reorged.
pub struct TransactionClient {
	api: OnlineClient<PolkadotConfig>,
	rpc: LegacyRpcMethods<PolkadotConfig>,
	/// Broadcast targets — one per provider, the first being the monitor's. The
	/// pipeline fans `author_submitExtrinsic` out to all of them, so a single
	/// dead provider can't stall a wave.
	submit_rpcs: Vec<LegacyRpcMethods<PolkadotConfig>>,
}

impl TransactionClient {
	/// Create a new transaction client by connecting to a single endpoint.
	pub async fn new(endpoint: &str) -> Result<Self> {
		Self::from_endpoints(&[endpoint]).await
	}

	/// Create a transaction client over multiple WS endpoints. Convenience over
	/// [`Self::from_rpc_clients`] that opens a WS `RpcClient` per endpoint.
	pub async fn from_endpoints(endpoints: &[&str]) -> Result<Self> {
		let mut clients = Vec::with_capacity(endpoints.len());
		for ep in endpoints {
			clients.push(
				RpcClient::from_url(ep).await.map_err(|e| {
					Error::NetworkError(format!("Failed to connect to {ep}: {e:?}"))
				})?,
			);
		}
		Self::from_rpc_clients(clients).await
	}

	/// Create a client from pre-built RPC clients — the provider abstraction.
	///
	/// Each [`RpcClient`] is a "provider": a WS connection (`RpcClient::from_url`)
	/// or a smoldot light client (`lc.parachain(spec)?.into()`). This mirrors the
	/// TS SDK's `providers: () => [getWsProvider(..), getSmProvider(..)]` list.
	/// The first client is the monitor (reconcile + nonce/storage reads); every
	/// client is a broadcast target for `author_submitExtrinsic` fan-out, so one
	/// dead provider can't stall a run.
	///
	/// For a light-client provider you must keep its `LightClient` alive yourself
	/// (dropping it tears down the connection); the SDK only holds the `RpcClient`.
	pub async fn from_rpc_clients(clients: Vec<RpcClient>) -> Result<Self> {
		let monitor = clients
			.first()
			.ok_or_else(|| Error::InvalidConfig("at least one RPC client is required".into()))?
			.clone();
		let rpc = LegacyRpcMethods::<PolkadotConfig>::new(monitor.clone());
		let api = OnlineClient::<PolkadotConfig>::from_rpc_client(monitor)
			.await
			.map_err(|e| Error::NetworkError(format!("Failed to connect: {e:?}")))?;
		let submit_rpcs =
			clients.into_iter().map(LegacyRpcMethods::<PolkadotConfig>::new).collect();
		Ok(Self { api, rpc, submit_rpcs })
	}

	/// Create a transaction client from a single existing RPC client.
	pub async fn from_rpc_client(rpc_client: RpcClient) -> Result<Self> {
		Self::from_rpc_clients(vec![rpc_client]).await
	}

	/// Get the underlying subxt client.
	pub fn api(&self) -> &OnlineClient<PolkadotConfig> {
		&self.api
	}

	// Uploads go through the single `estimate_upload` → `submit` primitive (see
	// below). `submit_unsigned` is the preimage-authorized variant. In-memory
	// items are uploaded by planning them with `estimate_upload(UploadInput::Items)`
	// and submitting against `blob_from_items(..)`.

	/// Plan an upload and size its authorization without buffering. For
	/// [`UploadInput::Source`] the source is streamed once (`O(chunk_size)`) into
	/// chunks + a DAG-PB manifest; for [`UploadInput::Items`] each item is one
	/// unit (no manifest). The returned [`UploadEstimate`] carries the per-unit
	/// disposition, within-input duplicates, and (when `skip_existing`) units
	/// already on chain. Pass the returned [`StreamEstimate`] to [`Self::submit`]
	/// with the matching source (`blob_from_items(..)` for items). The single
	/// planning entry point — mirrors TS `estimateUpload(input, options)`.
	pub async fn estimate_upload(
		&self,
		input: UploadInput,
		options: UploadEstimateOptions,
	) -> Result<StreamEstimate> {
		let plan = match input {
			UploadInput::Items(items) => Self::plan_from_items(&items)?,
			UploadInput::Source(source) =>
				plan_stream(source.as_ref(), &options.chunker, &options.store).await?,
		};
		self.assemble_estimate(plan, &options).await
	}

	/// Build a manifestless plan from in-memory items: one unit each, per-item
	/// codec/hash, offsets indexing into `blob_from_items`. Mirrors TS
	/// `estimateUploadItems`.
	fn plan_from_items(items: &[crate::pipeline::UploadItem]) -> Result<ChunkPlan> {
		if items.is_empty() {
			return Err(Error::EmptyData);
		}
		let mut chunk_cids = Vec::with_capacity(items.len());
		let mut chunk_sizes = Vec::with_capacity(items.len());
		let mut offsets = Vec::with_capacity(items.len());
		let mut codecs = Vec::with_capacity(items.len());
		let mut hash_algos = Vec::with_capacity(items.len());
		let mut total = 0u64;
		for item in items {
			if item.data.is_empty() {
				return Err(Error::EmptyData);
			}
			let codec = item.codec.unwrap_or(CidCodec::Raw);
			let hash_algo = item.hash_algo.unwrap_or(HashingAlgorithm::Blake2b256);
			let cid = calculate_cid_with_config(&item.data, codec, hash_algo)?;
			offsets.push(total);
			total += item.data.len() as u64;
			chunk_sizes.push(item.data.len() as u64);
			chunk_cids.push(cid);
			codecs.push(codec);
			hash_algos.push(hash_algo);
		}
		Ok(ChunkPlan {
			chunk_cids,
			chunk_sizes,
			offsets,
			codecs,
			hash_algos,
			total_size: total,
			root_cid: None,
			manifest_data: None,
		})
	}

	/// Build the per-unit [`UploadEstimate`] from a plan: collapse within-input
	/// duplicate content hashes (`dedup_input`), optionally exclude units already
	/// on chain (`skip_existing`, one TBCH read per unique content hash), and sum
	/// the authorization for what's left. Mirrors TS `assembleEstimate`.
	async fn assemble_estimate(
		&self,
		plan: ChunkPlan,
		options: &UploadEstimateOptions,
	) -> Result<StreamEstimate> {
		// One entry per plan unit (chunks, then the manifest if present).
		let chunk_count = plan.chunk_cids.len();
		let hashes: Vec<ContentHash> = plan
			.chunk_cids
			.iter()
			.chain(plan.root_cid.iter())
			.map(|c| c.content_hash)
			.collect();
		let mut cids: Vec<Vec<u8>> = Vec::with_capacity(hashes.len());
		for c in plan.chunk_cids.iter().chain(plan.root_cid.iter()) {
			cids.push(cid_to_bytes(c)?);
		}
		let size_of = |i: usize| -> u64 {
			if i < chunk_count {
				plan.chunk_sizes[i]
			} else {
				plan.manifest_data.as_ref().map_or(0, |m| m.len() as u64)
			}
		};

		// Within-input duplicates: first occurrence wins.
		let mut duplicate_indices = Vec::new();
		let mut first_seen: HashMap<ContentHash, usize> = HashMap::new();
		if options.dedup_input {
			for (i, h) in hashes.iter().enumerate() {
				if first_seen.contains_key(h) {
					duplicate_indices.push(i);
				} else {
					first_seen.insert(*h, i);
				}
			}
		}

		// Optional on-chain dedup: TBCH read per unique content hash.
		let mut already_stored = Vec::new();
		if options.skip_existing {
			let query: Vec<usize> = if options.dedup_input {
				first_seen.values().copied().collect()
			} else {
				(0..hashes.len()).collect()
			};
			let finalized = self
				.rpc
				.chain_get_finalized_head()
				.await
				.map_err(|e| Error::NetworkError(format!("finalized head: {e:?}")))?;
			let storage = self.api.storage().at(finalized);
			let mut on_chain: HashSet<ContentHash> = HashSet::new();
			for i in query {
				let addr = bulletin::storage()
					.transaction_storage()
					.transaction_by_content_hash(hashes[i]);
				let present = storage
					.fetch(&addr)
					.await
					.map_err(|e| Error::NetworkError(format!("TransactionByContentHash: {e:?}")))?
					.is_some();
				if present {
					on_chain.insert(hashes[i]);
				}
			}
			// Mark every index whose content is on chain — including duplicates.
			for (i, h) in hashes.iter().enumerate() {
				if on_chain.contains(h) {
					already_stored.push(i);
				}
			}
		}

		let dup_set: HashSet<usize> = duplicate_indices.iter().copied().collect();
		let stored_set: HashSet<usize> = already_stored.iter().copied().collect();
		let mut items = Vec::with_capacity(hashes.len());
		let mut to_upload = Vec::new();
		let mut bytes = 0u64;
		for (i, cid) in cids.iter().enumerate() {
			// Duplicate takes precedence over already-on-chain for the reason.
			let skip_reason = if dup_set.contains(&i) {
				Some(SkipReason::DuplicateInput)
			} else if stored_set.contains(&i) {
				Some(SkipReason::AlreadyOnChain)
			} else {
				None
			};
			items.push(UploadEstimateItem {
				index: i,
				cid: cid.clone(),
				bytes: size_of(i),
				skip_reason,
			});
			if skip_reason.is_none() {
				to_upload.push(i);
				bytes += size_of(i);
			}
		}

		Ok(StreamEstimate {
			base: UploadEstimate {
				total: hashes.len(),
				items,
				transactions: to_upload.len() as u32,
				bytes,
				duplicate_indices,
				already_stored,
				to_upload,
			},
			plan,
		})
	}

	/// Submit a streamed upload from a prepared [`StreamEstimate`], fetching each
	/// chunk's bytes lazily via `source.read(offset, size)` so resident memory
	/// tracks the in-flight window, not the whole file. The manifest, if any, is
	/// submitted last. Units the estimate excluded — within-input duplicates
	/// (`dedup_input`) and, with `skip_existing`, units already on chain — are
	/// not submitted; their cids are still returned. Content already on chain is
	/// otherwise re-stored on purpose (pays, refreshes retention). Returns the
	/// CIDs in plan order, with the manifest root last when present.
	pub async fn submit(
		&self,
		signer: &Keypair,
		estimate: StreamEstimate,
		source: Arc<dyn SeekableSource>,
		config: UploadConfig,
	) -> Result<UploadResult> {
		let pre_skipped = Self::pre_skipped(&estimate);
		Self::assert_unique_content_hashes(&estimate.plan, &pre_skipped)?;
		let resolved = Self::resolve_plan(estimate.plan, source)?;
		run_resolved(
			self.api.clone(),
			self.rpc.clone(),
			self.submit_rpcs.clone(),
			Some(signer),
			resolved,
			pre_skipped,
			config,
		)
		.await
	}

	/// Submit a streamed upload via the unsigned (preimage-authorized) path.
	///
	/// Identical to [`Self::submit`] but each chunk + the manifest are broadcast
	/// as bare extrinsics — no signer, no nonce. Every content hash (each chunk
	/// CID and the manifest root) must have been authorized with
	/// [`Self::authorize_preimage`] first, or the run ends in
	/// [`crate::types::Error::StoreStalled`].
	pub async fn submit_unsigned(
		&self,
		estimate: StreamEstimate,
		source: Arc<dyn SeekableSource>,
		config: UploadConfig,
	) -> Result<UploadResult> {
		let pre_skipped = Self::pre_skipped(&estimate);
		Self::assert_unique_content_hashes(&estimate.plan, &pre_skipped)?;
		let resolved = Self::resolve_plan(estimate.plan, source)?;
		run_resolved(
			self.api.clone(),
			self.rpc.clone(),
			self.submit_rpcs.clone(),
			None,
			resolved,
			pre_skipped,
			config,
		)
		.await
	}

	/// Units the estimate excluded from submission: within-input duplicates
	/// (`dedup_input`) and units already on chain (`skip_existing`). Mirrors
	/// the TS client's `preSkipped`.
	fn pre_skipped(estimate: &StreamEstimate) -> BTreeSet<usize> {
		estimate
			.base
			.duplicate_indices
			.iter()
			.chain(estimate.base.already_stored.iter())
			.copied()
			.collect()
	}

	/// Reject a plan whose SUBMITTED (non-skipped) units share a content hash —
	/// the reconciler identifies units by content hash and can't tell
	/// duplicates apart. Estimate-collapsed duplicates are in `skip`, so this
	/// only fires when `dedup_input` was disabled and genuine duplicates
	/// remain. Mirrors TS `assertUniqueContentHashes`.
	fn assert_unique_content_hashes(plan: &ChunkPlan, skip: &BTreeSet<usize>) -> Result<()> {
		let mut seen: HashMap<ContentHash, usize> = HashMap::new();
		for (i, c) in plan.chunk_cids.iter().chain(plan.root_cid.iter()).enumerate() {
			if skip.contains(&i) {
				continue;
			}
			if let Some(prior) = seen.insert(c.content_hash, i) {
				return Err(Error::InvalidConfig(format!(
					"submit(): unit {i} has the same content hash as unit {prior} — the SDK \
					 identifies units by content hash and can't distinguish duplicates; use the \
					 default dedup_input estimate (which skips duplicates), or store the same \
					 data in separate submits"
				)));
			}
		}
		Ok(())
	}

	/// Turn a [`ChunkPlan`] + seekable source into lazy pipeline items: one per
	/// chunk (bytes range-read on demand) plus the manifest last (resident).
	fn resolve_plan(plan: ChunkPlan, source: Arc<dyn SeekableSource>) -> Result<Vec<ResolvedItem>> {
		// The plan's CIDs were hashed from the estimate-pass bytes; submit reads
		// from `source` and reconciles against those CIDs. A size mismatch means
		// the source changed or differs from the one estimated — the chain would
		// store one payload while the pipeline reconciles another hash. Fail fast.
		if source.total_size() != plan.total_size {
			return Err(Error::InvalidConfig(format!(
				"submit(): source size {} does not match the estimate ({} bytes) — the source \
				 changed or differs from the one passed to estimate_upload(); re-run \
				 estimate_upload(source) with the current source",
				source.total_size(),
				plan.total_size
			)));
		}
		let mut resolved: Vec<ResolvedItem> = Vec::with_capacity(plan.chunk_cids.len() + 1);

		// The manifest (if any) is hashed with the same algorithm as the chunks.
		let manifest_hash_algo =
			plan.hash_algos.first().copied().unwrap_or(HashingAlgorithm::Blake2b256);
		for i in 0..plan.chunk_cids.len() {
			let cid_data = plan.chunk_cids[i];
			let cid_bytes = cid_to_bytes(&cid_data)?;
			let offset = plan.offsets[i];
			let size = plan.chunk_sizes[i];
			let codec = plan.codecs[i];
			let hash_algo = plan.hash_algos[i];
			let src = source.clone();
			let get_data: GetData = Arc::new(move || {
				let src = src.clone();
				Box::pin(async move { src.read(offset, size).await })
			});
			resolved.push(ResolvedItem::new(
				size as usize,
				codec,
				hash_algo,
				cid_data.content_hash,
				cid_bytes,
				get_data,
			));
		}

		// Manifest store goes last; its bytes are resident.
		if let (Some(root), Some(manifest_data)) = (plan.root_cid, plan.manifest_data) {
			let cid_bytes = cid_to_bytes(&root)?;
			let size = manifest_data.len();
			let data: ItemData = ItemData::from(manifest_data);
			let get_data: GetData = Arc::new(move || {
				let data = data.clone();
				Box::pin(async move { Ok(data) })
			});
			resolved.push(ResolvedItem::new(
				size,
				CidCodec::DagPb,
				manifest_hash_algo,
				root.content_hash,
				cid_bytes,
				get_data,
			));
		}

		Ok(resolved)
	}

	/// Submit a transaction with a pool-aware nonce.
	///
	/// Resolves the nonce via `system_accountNextIndex`, which counts both
	/// already-included and pool-pending transactions. For sequential submits
	/// (e.g. one `store` per chunk) this means tx N+1 sees tx N already in the
	/// pool and takes the next slot, instead of racing a best-block read that
	/// could reuse N's nonce ("usurped" / "bad nonce") when N is still pending
	/// or its block reorged.
	async fn submit_with_pool_nonce(
		&self,
		tx: &impl subxt::tx::Payload,
		signer: &Keypair,
		make_error: &impl Fn(String) -> Error,
	) -> Result<subxt::tx::TxProgress<PolkadotConfig, OnlineClient<PolkadotConfig>>> {
		let account = AccountId32::from(signer.public_key().0);

		let nonce = self
			.rpc
			.system_account_next_index(&account)
			.await
			.map_err(|e| make_error(format!("{e:?}")))?;

		let params = DefaultExtrinsicParamsBuilder::<PolkadotConfig>::new().nonce(nonce).build();
		self.api
			.tx()
			.sign_and_submit_then_watch(tx, signer, params)
			.await
			.map_err(|e| make_error(format!("{e:?}")))
	}

	/// Submit a transaction, stream status events, and return when the target
	/// confirmation level is reached.
	///
	/// This is the single submission loop used by all public methods. It:
	/// - Streams all `TxStatus` events, firing the optional progress callback
	/// - Breaks on `InBestBlock` or `InFinalizedBlock` depending on the value of `wait_for`
	/// - Returns block hash and extrinsic hash on success
	async fn submit_and_watch(
		&self,
		tx: &impl subxt::tx::Payload,
		signer: &Keypair,
		wait_for: WaitFor,
		progress_callback: Option<ProgressCallback>,
		make_error: impl Fn(String) -> Error,
	) -> Result<SubmitResult> {
		let mut progress = self.submit_with_pool_nonce(tx, signer, &make_error).await?;

		let mut result_block_hash = None;
		let mut result_extrinsic_hash = None;

		while let Some(status) = progress.next().await {
			match status {
				Ok(status) => {
					use subxt::tx::TxStatus;
					match status {
						TxStatus::Validated =>
							if let Some(ref cb) = progress_callback {
								cb(ProgressEvent::tx_validated());
							},
						TxStatus::Broadcasted =>
							if let Some(ref cb) = progress_callback {
								cb(ProgressEvent::tx_broadcasted());
							},
						TxStatus::InBestBlock(in_block) => {
							let block_hash = in_block.block_hash().to_string();
							let extrinsic_hash = in_block.extrinsic_hash().to_string();

							if let Some(ref cb) = progress_callback {
								let block_number =
									self.get_block_number(in_block.block_hash()).await.ok();
								cb(ProgressEvent::tx_in_block(
									block_hash.clone(),
									block_number,
									None,
								));
							}

							result_block_hash = Some(block_hash);
							result_extrinsic_hash = Some(extrinsic_hash);

							if wait_for == WaitFor::InBlock {
								in_block.wait_for_success().await.map_err(|e| {
									make_error(format!("Transaction failed: {e:?}"))
								})?;
								break;
							}
						},
						TxStatus::InFinalizedBlock(in_block) => {
							let block_hash = in_block.block_hash().to_string();
							let extrinsic_hash = in_block.extrinsic_hash().to_string();

							if let Some(ref cb) = progress_callback {
								let block_number =
									self.get_block_number(in_block.block_hash()).await.ok();
								cb(ProgressEvent::tx_finalized(
									block_hash.clone(),
									block_number,
									None,
								));
							}

							result_block_hash = Some(block_hash);
							result_extrinsic_hash = Some(extrinsic_hash);

							in_block
								.wait_for_success()
								.await
								.map_err(|e| make_error(format!("Transaction failed: {e:?}")))?;
							break;
						},
						TxStatus::NoLongerInBestBlock =>
							if let Some(ref cb) = progress_callback {
								cb(ProgressEvent::Transaction(
									crate::types::TransactionStatusEvent::NoLongerInBlock {
										chunk_index: None,
									},
								));
							},
						TxStatus::Invalid { message } => {
							if let Some(ref cb) = progress_callback {
								cb(ProgressEvent::Transaction(
									crate::types::TransactionStatusEvent::Invalid {
										error: message.clone(),
										chunk_index: None,
									},
								));
							}
							return Err(make_error(format!("Transaction invalid: {message}")));
						},
						TxStatus::Dropped { message } => {
							if let Some(ref cb) = progress_callback {
								cb(ProgressEvent::Transaction(
									crate::types::TransactionStatusEvent::Dropped {
										error: message.clone(),
										chunk_index: None,
									},
								));
							}
							return Err(make_error(format!("Transaction dropped: {message}")));
						},
						TxStatus::Error { message } =>
							return Err(make_error(format!("Transaction error: {message}"))),
					}
				},
				Err(e) => return Err(make_error(format!("Status error: {e:?}"))),
			}
		}

		match (result_block_hash, result_extrinsic_hash) {
			(Some(block_hash), Some(extrinsic_hash)) =>
				Ok(SubmitResult { block_hash, extrinsic_hash }),
			_ => Err(make_error("Transaction stream ended without block inclusion".into())),
		}
	}

	/// Query the current authorization for an account and return the remaining boost-tier
	/// capacity as `(transactions_remaining, bytes_remaining)`.
	///
	/// `bytes` and `transactions` on `AuthorizationExtent` are *consumed* counters; this
	/// helper subtracts them from the granted caps (saturating to `0` if the holder is
	/// already over-cap on either axis). Note that overshooting the caps no longer rejects
	/// the transaction — it only forfeits the priority boost — so the returned remaining
	/// values are a soft-budget preflight, not a hard precondition.
	///
	/// Returns `None` if no authorization exists or it has expired.
	pub async fn query_account_authorization(
		&self,
		who: &AccountId32,
	) -> Result<Option<(u32, u64)>> {
		use bulletin::runtime_types::pallet_bulletin_transaction_storage::types::AuthorizationScope as OnChainScope;

		let storage_query = bulletin::storage()
			.transaction_storage()
			.authorizations(OnChainScope::Account(who.clone()));

		let latest_block = self
			.api
			.blocks()
			.at_latest()
			.await
			.map_err(|e| Error::NetworkError(format!("Failed to get latest block: {e:?}")))?;

		let current_block_number = latest_block.number();

		let maybe_auth =
			latest_block.storage().fetch(&storage_query).await.map_err(|e| {
				Error::NetworkError(format!("Failed to query authorization: {e:?}"))
			})?;

		match maybe_auth {
			Some(auth) if auth.expiration > current_block_number => {
				let transactions_remaining =
					auth.extent.transactions_allowance.saturating_sub(auth.extent.transactions);
				let bytes_remaining = auth.extent.bytes_allowance.saturating_sub(auth.extent.bytes);
				Ok(Some((transactions_remaining, bytes_remaining)))
			},
			Some(_) => Ok(None), // expired
			None => Ok(None),
		}
	}

	/// Check that sufficient authorization exists for a store operation.
	///
	/// Queries the chain for the account's current authorization and validates
	/// that it has enough transactions and bytes remaining for the boost tier.
	///
	/// This is a soft preflight: under the soft-cap design, on-chain validation
	/// no longer rejects a `store` for being over-budget — it only drops the
	/// priority boost. Use this check to decide whether to send (likely-boosted)
	/// or to top up the authorization first; do not treat a failure here as
	/// "the chain will reject this tx".
	///
	/// If the query itself fails (e.g., network error), the error is returned
	/// so the caller can decide whether to proceed.
	pub async fn check_authorization_for_store(
		&self,
		who: &AccountId32,
		required_transactions: u32,
		required_bytes: u64,
	) -> Result<()> {
		let auth_data = self.query_account_authorization(who).await?;

		match auth_data {
			Some((transactions, bytes)) => {
				let auth = Authorization {
					scope: AuthorizationScope::Account,
					transactions,
					max_size: bytes,
					expires_at: None, // already filtered out expired
				};
				let manager = AuthorizationManager::new();
				manager.check_authorization(&auth, required_bytes, required_transactions)
			},
			None => Err(Error::AuthorizationNotFound(format!("{who}"))),
		}
	}

	/// Store data on-chain.
	///
	/// Submits a `TransactionStorage.store` extrinsic.
	pub async fn store(
		&self,
		data: Vec<u8>,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<StoreReceipt> {
		self.store_with_progress(data, signer, wait_for, None).await
	}

	/// Store data on-chain with progress callbacks.
	///
	/// Submits a `TransactionStorage.store` extrinsic and emits progress
	/// events as the transaction moves through the network.
	///
	/// Before submitting, checks the account's on-chain authorization.
	/// Returns an error immediately if authorization is missing, expired,
	/// or insufficient (avoiding a wasted transaction submission).
	///
	/// Progress events emitted:
	/// - `TransactionStatusEvent::Validated` - Transaction validated in pool
	/// - `TransactionStatusEvent::Broadcasted` - Transaction sent to peers
	/// - `TransactionStatusEvent::InBlock` - Transaction in a best block
	/// - `TransactionStatusEvent::Finalized` - Transaction finalized
	pub async fn store_with_progress(
		&self,
		data: Vec<u8>,
		signer: &Keypair,
		wait_for: WaitFor,
		progress_callback: Option<ProgressCallback>,
	) -> Result<StoreReceipt> {
		let data_size = data.len() as u64;

		// Authorization check before submission
		let account = AccountId32::from(signer.public_key().0);
		self.check_authorization_for_store(&account, 1, data_size).await?;

		let tx = bulletin::tx().transaction_storage().store(data);
		let result = self
			.submit_and_watch(&tx, signer, wait_for, progress_callback, |e| {
				Error::StorageFailed(format!("Store failed: {e}"))
			})
			.await?;

		Ok(StoreReceipt {
			block_hash: result.block_hash,
			extrinsic_hash: result.extrinsic_hash,
			data_size,
		})
	}

	/// Helper to get block number from block hash.
	async fn get_block_number<H: Into<BlockRef<subxt::config::substrate::H256>>>(
		&self,
		block_hash: H,
	) -> Result<u32> {
		let block = self
			.api
			.blocks()
			.at(block_hash)
			.await
			.map_err(|e| Error::NetworkError(format!("Failed to get block: {e:?}")))?;

		Ok(block.number())
	}

	/// Authorize an account to store data.
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn authorize_account(
		&self,
		who: AccountId32,
		transactions: u32,
		bytes: u64,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<AuthorizationReceipt> {
		let tx = bulletin::tx().transaction_storage().authorize_account(
			who.clone(),
			transactions,
			bytes,
		);

		let result = self
			.submit_and_watch(&tx, signer, wait_for, None, |e| {
				Error::TransactionFailed(format!("Authorization failed: {e}"))
			})
			.await?;

		Ok(AuthorizationReceipt {
			account: who,
			transactions,
			bytes,
			block_hash: result.block_hash,
		})
	}

	/// Authorize one or many accounts in a single transaction.
	///
	/// With multiple entries the calls are wrapped in `Utility.batch_all` —
	/// atomic: either every authorization applies or none do. With `sudo: true`
	/// the call (single or batch) is dispatched through `Sudo.sudo` (Root
	/// origin), so the signer authorizes without being a registered authorizer.
	/// Returns one receipt per entry, all sharing the inclusion block hash.
	/// Mirrors the TS `authorizeAccount(entries)` + `.withSudo()`.
	pub async fn authorize_accounts(
		&self,
		entries: Vec<AuthorizeAccountEntry>,
		sudo: bool,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<Vec<AuthorizationReceipt>> {
		use bulletin::runtime_types::{
			bulletin_westend_runtime::RuntimeCall,
			pallet_bulletin_transaction_storage::pallet::Call as StorageCall,
			pallet_utility::pallet::Call as UtilityCall,
		};

		if entries.is_empty() {
			return Err(Error::InvalidConfig(
				"authorize_accounts requires at least one entry".into(),
			));
		}
		let err = |e: String| Error::TransactionFailed(format!("Authorization failed: {e}"));

		let calls: Vec<RuntimeCall> = entries
			.iter()
			.map(|e| {
				RuntimeCall::TransactionStorage(StorageCall::authorize_account {
					who: e.who.clone(),
					transactions: e.transactions,
					bytes: e.bytes,
				})
			})
			.collect();

		let block_hash = if sudo {
			// Single call or a batch, dispatched with Root via Sudo.sudo.
			let call = if calls.len() == 1 {
				calls.into_iter().next().expect("len == 1")
			} else {
				RuntimeCall::Utility(UtilityCall::batch_all { calls })
			};
			let tx = bulletin::tx().sudo().sudo(call);
			self.submit_and_watch(&tx, signer, wait_for, None, err).await?.block_hash
		} else if calls.len() == 1 {
			let e = &entries[0];
			let tx = bulletin::tx().transaction_storage().authorize_account(
				e.who.clone(),
				e.transactions,
				e.bytes,
			);
			self.submit_and_watch(&tx, signer, wait_for, None, err).await?.block_hash
		} else {
			let tx = bulletin::tx().utility().batch_all(calls);
			self.submit_and_watch(&tx, signer, wait_for, None, err).await?.block_hash
		};

		Ok(entries
			.into_iter()
			.map(|e| AuthorizationReceipt {
				account: e.who,
				transactions: e.transactions,
				bytes: e.bytes,
				block_hash: block_hash.clone(),
			})
			.collect())
	}

	/// Authorize a preimage (by content hash) to be stored.
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn authorize_preimage(
		&self,
		content_hash: ContentHash,
		max_size: u64,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<PreimageAuthorizationReceipt> {
		let tx = bulletin::tx().transaction_storage().authorize_preimage(content_hash, max_size);

		let result = self
			.submit_and_watch(&tx, signer, wait_for, None, |e| {
				Error::TransactionFailed(format!("Authorization failed: {e}"))
			})
			.await?;

		Ok(PreimageAuthorizationReceipt { content_hash, max_size, block_hash: result.block_hash })
	}

	/// Schedule a one-shot renewal of stored data.
	///
	/// The renewal fires once when the data reaches its retention boundary; it
	/// does not renew synchronously. For immediate renewal use
	/// [`force_renew`](Self::force_renew).
	///
	/// `entry` accepts anything convertible to [`TransactionRef`]: a
	/// `(block, index)` tuple or a [`ContentHash`].
	///
	/// The fleet runs several runtime generations at once, so the call is
	/// dispatched through the compat registry: the connected chain's
	/// `TransactionStorage.renew` type-tree hash selects the encoder (a local
	/// lookup, no extra RPC) — see [`crate::compat`]. An absent or unknown
	/// shape fails closed. Legacy positional runtimes take `(block, index)`
	/// only; content-hash renewal there fails closed.
	pub async fn renew(
		&self,
		entry: impl Into<TransactionRef<u32>>,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<RenewReceipt> {
		let entry = entry.into();
		let result = match compat::renew_adapter(&self.api.metadata())? {
			compat::RenewAdapter::TransactionRef => {
				let tx = bulletin::tx().transaction_storage().renew(to_runtime_ref(&entry));
				self.submit_and_watch(&tx, signer, wait_for, None, |e| {
					Error::RenewalFailed(format!("Renew failed: {e}"))
				})
				.await?
			},
			compat::RenewAdapter::Positional => {
				let TransactionRef::Position { block, index } = &entry else {
					return Err(Error::RenewalFailed(
						"content-hash renewal is not supported by this runtime".into(),
					));
				};
				let tx =
					compat::bulletin_v1000011::tx().transaction_storage().renew(*block, *index);
				self.submit_and_watch(&tx, signer, wait_for, None, |e| {
					Error::RenewalFailed(format!("Renew failed: {e}"))
				})
				.await?
			},
		};

		Ok(RenewReceipt { entry, block_hash: result.block_hash })
	}

	/// Immediately renew stored data, extending its retention from the current
	/// block.
	///
	/// `entry` accepts anything convertible to [`TransactionRef`]: a
	/// `(block, index)` tuple or a [`ContentHash`]. Requires a runtime that
	/// ships `TransactionRef` / `force_renew`; legacy positional runtimes fail
	/// closed.
	pub async fn force_renew(
		&self,
		entry: impl Into<TransactionRef<u32>>,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<RenewReceipt> {
		if compat::renew_adapter(&self.api.metadata())? != compat::RenewAdapter::TransactionRef {
			return Err(Error::RenewalFailed("force_renew is not supported by this runtime".into()));
		}
		let entry = entry.into();
		let tx = bulletin::tx().transaction_storage().force_renew(to_runtime_ref(&entry));
		let result = self
			.submit_and_watch(&tx, signer, wait_for, None, |e| {
				Error::RenewalFailed(format!("Force renew failed: {e}"))
			})
			.await?;

		Ok(RenewReceipt { entry, block_hash: result.block_hash })
	}

	/// Refresh an account authorization (extends expiry).
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn refresh_account_authorization(
		&self,
		who: AccountId32,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<()> {
		let tx = bulletin::tx().transaction_storage().refresh_account_authorization(who);
		self.submit_and_watch(&tx, signer, wait_for, None, |e| {
			Error::TransactionFailed(format!("Refresh failed: {e}"))
		})
		.await?;
		Ok(())
	}

	/// Refresh a preimage authorization (extends expiry).
	///
	/// Requires authorizer origin (typically sudo).
	pub async fn refresh_preimage_authorization(
		&self,
		content_hash: ContentHash,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<()> {
		let tx = bulletin::tx()
			.transaction_storage()
			.refresh_preimage_authorization(content_hash);
		self.submit_and_watch(&tx, signer, wait_for, None, |e| {
			Error::TransactionFailed(format!("Refresh failed: {e}"))
		})
		.await?;
		Ok(())
	}

	/// Remove an expired account authorization.
	pub async fn remove_expired_account_authorization(
		&self,
		who: AccountId32,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<()> {
		let tx = bulletin::tx().transaction_storage().remove_expired_account_authorization(who);
		self.submit_and_watch(&tx, signer, wait_for, None, |e| {
			Error::TransactionFailed(format!("Removal failed: {e}"))
		})
		.await?;
		Ok(())
	}

	/// Remove an expired preimage authorization.
	pub async fn remove_expired_preimage_authorization(
		&self,
		content_hash: ContentHash,
		signer: &Keypair,
		wait_for: WaitFor,
	) -> Result<()> {
		let tx = bulletin::tx()
			.transaction_storage()
			.remove_expired_preimage_authorization(content_hash);
		self.submit_and_watch(&tx, signer, wait_for, None, |e| {
			Error::TransactionFailed(format!("Removal failed: {e}"))
		})
		.await?;
		Ok(())
	}
}

/// Internal result from `submit_and_watch`.
struct SubmitResult {
	block_hash: String,
	extrinsic_hash: String,
}

/// Receipt from a successful store operation.
#[derive(Debug, Clone)]
pub struct StoreReceipt {
	pub block_hash: String,
	pub extrinsic_hash: String,
	pub data_size: u64,
}

/// One account-authorization grant for the batched
/// [`TransactionClient::authorize_accounts`]. Mirrors the TS `AuthorizeAccountEntry`.
#[derive(Debug, Clone)]
pub struct AuthorizeAccountEntry {
	pub who: AccountId32,
	pub transactions: u32,
	pub bytes: u64,
}

/// Receipt from a successful authorization.
#[derive(Debug, Clone)]
pub struct AuthorizationReceipt {
	pub account: AccountId32,
	pub transactions: u32,
	pub bytes: u64,
	pub block_hash: String,
}

/// Receipt from a successful preimage authorization.
#[derive(Debug, Clone)]
pub struct PreimageAuthorizationReceipt {
	pub content_hash: ContentHash,
	pub max_size: u64,
	pub block_hash: String,
}

/// Receipt from a successful renew operation.
#[derive(Debug, Clone)]
pub struct RenewReceipt {
	/// The renewed entry (position or content hash) that was submitted.
	pub entry: TransactionRef<u32>,
	pub block_hash: String,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn plan_of(datas: &[&[u8]]) -> ChunkPlan {
		let mut chunk_cids = Vec::new();
		let mut chunk_sizes = Vec::new();
		let mut offsets = Vec::new();
		let mut total = 0u64;
		for d in datas {
			chunk_cids.push(
				calculate_cid_with_config(d, CidCodec::Raw, HashingAlgorithm::Blake2b256).unwrap(),
			);
			offsets.push(total);
			chunk_sizes.push(d.len() as u64);
			total += d.len() as u64;
		}
		ChunkPlan {
			chunk_cids,
			chunk_sizes,
			offsets,
			codecs: vec![CidCodec::Raw; datas.len()],
			hash_algos: vec![HashingAlgorithm::Blake2b256; datas.len()],
			total_size: total,
			root_cid: None,
			manifest_data: None,
		}
	}

	/// Leftover duplicates (dedup disabled) are rejected; estimate-collapsed
	/// duplicates in `skip` pass. Mirrors the TS duplicate-content guard.
	#[test]
	fn duplicate_content_rejected_unless_skipped() {
		let plan = plan_of(&[b"a", b"b", b"a"]);
		let err = TransactionClient::assert_unique_content_hashes(&plan, &BTreeSet::new())
			.expect_err("dup content with empty skip set must be rejected");
		assert_eq!(err.code(), "INVALID_CONFIG");
		let skip: BTreeSet<usize> = [2usize].into_iter().collect();
		assert!(TransactionClient::assert_unique_content_hashes(&plan, &skip).is_ok());
	}
}
