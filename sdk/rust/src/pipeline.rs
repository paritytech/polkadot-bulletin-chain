// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Wave-batched, reconcile-driven upload pipeline.
//!
//! A direct port of the TypeScript SDK's `pipelineStore`, providing the same
//! guarantees for bulk uploads:
//!
//! - **Wave batching**: items are grouped into waves sized to the chain's per-block capacity
//!   ([`BlockLimits`]) so blocks are filled, not trickled.
//! - **Pool-aware nonce floor**: each item is assigned one nonce from a
//!   `max(system_accountNextIndex, our-floor)` floor and keeps it across re-broadcasts, so a wave's
//!   transactions never collide.
//! - **Finalized mortality anchor**: transactions are signed mortal against the latest finalized
//!   block (re-anchored as finality advances), so a tip reorg never invalidates a signature (no
//!   BadProof eviction).
//! - **Reconcile-driven inclusion**: inclusion/finalization is read from the on-chain
//!   `TransactionByContentHash` map at each best/finalized block (not from per-tx watches), with
//!   clear-on-absence reorg handling.
//! - **Exactly-once**: a content-hash pre-check skips already-stored items, and each item owns one
//!   nonce slot — re-broadcast replaces it, never double-pays.
//! - **Hijack recovery**: if our assigned nonces execute (chain nonce reaches the expected final
//!   nonce) but an item never lands, its slot was taken by another transaction from the same
//!   signer; the slot is released, the item re-queued with a fresh nonce, and after
//!   [`MAX_RETRY_ATTEMPTS`] it fails with [`Error::HijackBudgetExceeded`].
//! - **Watchdogs**: a chainHead-silence timeout and a no-progress best-block counter detect a stuck
//!   connection.
//! - **Retry-resume**: on a stall/disconnect the run re-subscribes and re-broadcasts
//!   not-yet-confirmed items at their carried nonce (one `State` persists, so it stays
//!   exactly-once), up to [`MAX_STALL_RETRIES`]; only then does it surface [`Error::StoreStalled`].
//!
//! Both signed and unsigned (preimage-authorized) submission are supported:
//! `signer: None` skips nonce assignment, hijack detection, and signing, and
//! broadcasts bare extrinsics validated by the chain's `ValidateUnsigned`.
//!
//! Broadcast is multi-provider: each wave fans out to every configured endpoint
//! (accepted-if-any), so one dead endpoint can't stall it. The *monitor*
//! (reconcile/nonce) stream is still single-endpoint — hot-standby failover for
//! it is the remaining item in `TODOS.md`.

use crate::{
	blob_source::{GetData, ItemData},
	cid::{CidCodec, ContentHash, HashingAlgorithm},
	transaction::bulletin,
	types::{Error, Result, WaitFor},
};
use alloc::{collections::VecDeque, format, string::String, sync::Arc, vec::Vec};
use core::time::Duration;
use futures::stream::StreamExt;
use std::collections::{BTreeMap, BTreeSet};
use subxt::{
	backend::legacy::LegacyRpcMethods,
	config::{substrate::H256, DefaultExtrinsicParamsBuilder},
	utils::AccountId32,
	OnlineClient, PolkadotConfig,
};
use subxt_signer::sr25519::Keypair;

/// Mortal era length (blocks). Generous vs the spec-recommended 4 to absorb
/// queueing + finality lag, matching the TypeScript pipeline.
const ERA_PERIOD: u64 = 64;
/// A wave may span this many blocks' worth of capacity (TS `WAVE_BUFFER_BLOCKS`).
const WAVE_BUFFER_BLOCKS: u128 = 2;
/// Per-item nonce-slot retry budget before `HijackBudgetExceeded`.
const MAX_RETRY_ATTEMPTS: u32 = 10;
/// No chainHead event within this window ⇒ `StoreStalled`.
const STALL_TIMEOUT: Duration = Duration::from_secs(18);
/// This many best blocks with no newly-confirmed item ⇒ `StoreStalled`.
const MAX_NO_PROGRESS_BEST_BLOCKS: u32 = 20;
/// Outer retry-resume budget: re-subscribe + re-broadcast after a stall or
/// disconnect this many times before surfacing the error (TS `maxRetries`).
const MAX_STALL_RETRIES: u32 = 3;
/// Backoff before each retry attempt; doubles per attempt (1s, 2s, 4s).
const STALL_RETRY_BACKOFF_MS: u64 = 1_000;

/// Per-chain block-capacity constants used to size waves. Mirrors the
/// TypeScript SDK's `BlockLimits`.
#[derive(Debug, Clone)]
pub struct BlockLimits {
	/// Max normal-class weight (ref_time) per block.
	pub max_normal_weight: u64,
	/// Max normal-class block length in bytes.
	pub normal_block_length: u32,
	/// Hard per-block cap on store extrinsics (`MaxBlockTransactions`).
	pub max_block_transactions: u32,
	/// Base ref_time weight of a `store` extrinsic.
	pub store_weight_base: u64,
	/// Per-byte ref_time weight of a `store` extrinsic.
	pub store_weight_per_byte: u64,
	/// Encoding overhead per extrinsic (signature + address + extensions).
	pub extrinsic_overhead: u32,
}

/// Defaults for bulletin-westend / bulletin-paseo runtimes (mirrors the
/// TypeScript `DEFAULT_BLOCK_LIMITS`).
pub const DEFAULT_BLOCK_LIMITS: BlockLimits = BlockLimits {
	max_normal_weight: 1_500_000_000_000,
	normal_block_length: 9_437_184,
	max_block_transactions: 512,
	store_weight_base: 35_489_000,
	store_weight_per_byte: 6_912,
	extrinsic_overhead: 110,
};

/// One payload to store as a single `store` extrinsic. The CID is derived from
/// `(data, codec, hash_algo)` and used as the item's identity on every event.
#[derive(Debug, Clone)]
pub struct UploadItem {
	/// Raw bytes to store.
	pub data: Vec<u8>,
	/// CID codec (default: raw). `DagPb` for UnixFS manifests.
	pub codec: Option<CidCodec>,
	/// Multihash algorithm (default: blake2b-256).
	pub hash_algo: Option<HashingAlgorithm>,
}

impl UploadItem {
	/// Construct an item with default codec/hashing (raw + blake2b-256).
	pub fn new(data: Vec<u8>) -> Self {
		Self { data, codec: None, hash_algo: None }
	}
}

/// High-level upload lifecycle status. Mirrors the TypeScript `UploadStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadStatus {
	/// Item accepted into the pipeline.
	ItemStarted,
	/// Item's `store` is in a best block.
	ItemInBlock,
	/// Item's `store` is finalized.
	ItemFinalized,
	/// Item failed terminally.
	ItemFailed,
}

/// Per-item upload lifecycle event. Mirrors the TypeScript `UploadEvent`; every
/// variant carries the item's CID so callers can correlate by content.
#[derive(Debug, Clone)]
pub enum UploadEvent {
	/// Item accepted into the pipeline.
	ItemStarted { index: usize, total: usize, cid: Vec<u8> },
	/// Item's `store` was included in a best block.
	ItemInBlock {
		index: usize,
		total: usize,
		cid: Vec<u8>,
		block_hash: String,
		block_number: Option<u32>,
		/// `Stored`-event slot for `renew(block_number, transaction_index)`.
		transaction_index: Option<u32>,
	},
	/// Item's `store` was finalized.
	ItemFinalized {
		index: usize,
		total: usize,
		cid: Vec<u8>,
		block_hash: String,
		block_number: Option<u32>,
		transaction_index: Option<u32>,
	},
	/// Item failed terminally and will not be retried.
	ItemFailed { index: usize, total: usize, cid: Vec<u8>, error: String },
}

/// Callback for [`UploadEvent`]s. Cloneable + thread-safe.
pub type UploadCallback = Arc<dyn Fn(UploadEvent) + Send + Sync>;

/// Result of [`crate::transaction::TransactionClient::submit`]. `cids[i]`
/// corresponds to plan unit `i` (CIDv1 bytes), with the manifest root last.
#[derive(Debug, Clone)]
pub struct UploadResult {
	/// CIDs of all uploaded items, in input order.
	pub cids: Vec<Vec<u8>>,
}

/// Wire-level submission strategy — how signed extrinsics reach the chain.
/// Mirrors the TypeScript SDK's `SubmissionStrategyKind`. Today only
/// [`SubmissionStrategyKind::NonceTracking`] is implemented; the enum is the
/// seam through which alternatives (e.g. a `transactionWatch`-based strategy —
/// see `TODOS.md`) plug in without changing the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubmissionStrategyKind {
	/// `author_submitExtrinsic` fan-out; inclusion and hijack detection are
	/// driven by the chainHead reconciler via per-item nonce +
	/// `TransactionByContentHash`.
	#[default]
	NonceTracking,
}

/// Configuration for an upload run.
#[derive(Clone)]
pub struct UploadConfig {
	/// Confirmation level per item before it counts as done (default: Finalized).
	pub complete_on: WaitFor,
	/// Per-block capacity used to size waves.
	pub block_limits: BlockLimits,
	/// Optional progress sink.
	pub on_event: Option<UploadCallback>,
	/// Wire submission strategy (default: [`SubmissionStrategyKind::NonceTracking`]).
	pub submission_strategy: SubmissionStrategyKind,
}

impl Default for UploadConfig {
	fn default() -> Self {
		Self {
			complete_on: WaitFor::Finalized,
			block_limits: DEFAULT_BLOCK_LIMITS,
			on_event: None,
			submission_strategy: SubmissionStrategyKind::NonceTracking,
		}
	}
}

/// Resolved per-item config used internally. `get_data` fetches the bytes lazily
/// (resident for eager `upload`, a seekable range-read for streamed `submit`);
/// `size` is the byte length, known without loading the data.
pub(crate) struct ResolvedItem {
	size: usize,
	codec: CidCodec,
	hash_algo: HashingAlgorithm,
	content_hash: ContentHash,
	cid_bytes: Vec<u8>,
	get_data: GetData,
}

impl ResolvedItem {
	/// Build a resolved item from a precomputed CID + a lazy byte fetch.
	pub(crate) fn new(
		size: usize,
		codec: CidCodec,
		hash_algo: HashingAlgorithm,
		content_hash: ContentHash,
		cid_bytes: Vec<u8>,
		get_data: GetData,
	) -> Self {
		Self { size, codec, hash_algo, content_hash, cid_bytes, get_data }
	}
}

/// Where an item landed on chain (the `TransactionByContentHash` value).
#[derive(Clone, Copy, PartialEq, Eq)]
struct StoredLoc {
	block_number: u32,
	transaction_index: u32,
}

/// Select the prefix of `queue` that fits within `WAVE_BUFFER_BLOCKS` blocks of
/// capacity. Returns the count of leading items to submit (>= 1 unless the
/// first item alone exceeds the budget). Mirrors TS `selectWaveBatch`.
fn select_wave_count(
	queue: &VecDeque<usize>,
	items: &[ResolvedItem],
	limits: &BlockLimits,
) -> usize {
	let max_weight = limits.max_normal_weight as u128 * WAVE_BUFFER_BLOCKS;
	let max_length = limits.normal_block_length as u128 * WAVE_BUFFER_BLOCKS;
	let max_txs = limits.max_block_transactions as usize * WAVE_BUFFER_BLOCKS as usize;

	let mut acc_weight: u128 = 0;
	let mut acc_length: u128 = 0;
	let mut count = 0usize;

	for &idx in queue {
		let size = items[idx].size as u128;
		let tx_weight =
			limits.store_weight_base as u128 + limits.store_weight_per_byte as u128 * size;
		let tx_length = size + limits.extrinsic_overhead as u128;

		if acc_weight + tx_weight > max_weight ||
			acc_length + tx_length > max_length ||
			count >= max_txs
		{
			break;
		}
		acc_weight += tx_weight;
		acc_length += tx_length;
		count += 1;
	}
	count
}

fn to_onchain_hashing(
	h: HashingAlgorithm,
) -> bulletin::runtime_types::bulletin_transaction_storage_primitives::cids::HashingAlgorithm {
	use bulletin::runtime_types::bulletin_transaction_storage_primitives::cids::HashingAlgorithm as Oc;
	match h {
		HashingAlgorithm::Sha2_256 => Oc::Sha2_256,
		HashingAlgorithm::Keccak256 => Oc::Keccak256,
		// `HashingAlgorithm` is `#[non_exhaustive]`; Blake2b256 is the canonical
		// default and the only other protocol variant.
		HashingAlgorithm::Blake2b256 | _ => Oc::Blake2b256,
	}
}

/// `TransactionByContentHash` lookup at a pinned block.
async fn tbch_lookup(
	api: &OnlineClient<PolkadotConfig>,
	block_hash: H256,
	content_hash: ContentHash,
) -> Option<StoredLoc> {
	let addr = bulletin::storage()
		.transaction_storage()
		.transaction_by_content_hash(content_hash);
	match api.storage().at(block_hash).fetch(&addr).await {
		Ok(Some((block_number, transaction_index))) =>
			Some(StoredLoc { block_number, transaction_index }),
		_ => None,
	}
}

/// On-chain *executed* account nonce at `block_hash` (via `AccountNonceApi`).
/// This advances only as our transactions actually execute — the hijack
/// trigger compares it against the expected final nonce. NOT the pool nonce.
async fn chain_nonce(
	api: &OnlineClient<PolkadotConfig>,
	block_hash: H256,
	account: &AccountId32,
) -> Result<u64> {
	let payload = bulletin::apis().account_nonce_api().account_nonce(account.clone());
	let nonce: u32 = api
		.runtime_api()
		.at(block_hash)
		.call(payload)
		.await
		.map_err(|e| Error::NetworkError(format!("account_nonce: {e:?}")))?;
	Ok(nonce as u64)
}

/// Offline-sign one `store_with_cid_config` for `item` at `nonce`, mortal
/// against `anchor` (number, hash). Returns the raw signed extrinsic bytes.
fn sign_store(
	api: &OnlineClient<PolkadotConfig>,
	signer: &Keypair,
	item: &ResolvedItem,
	data: &[u8],
	nonce: u64,
	anchor: (u64, H256),
) -> Result<Vec<u8>> {
	let cfg = bulletin::runtime_types::bulletin_transaction_storage_primitives::cids::CidConfig {
		codec: item.codec.code(),
		hashing: to_onchain_hashing(item.hash_algo),
	};
	let call = bulletin::tx().transaction_storage().store_with_cid_config(cfg, data.to_vec());
	let params = DefaultExtrinsicParamsBuilder::<PolkadotConfig>::new()
		.nonce(nonce)
		.mortal_from_unchecked(ERA_PERIOD, anchor.0, anchor.1)
		.build();
	let signed = api
		.tx()
		.create_partial_offline(&call, params)
		.map_err(|e| Error::StorageFailed(format!("sign: {e:?}")))?
		.sign(signer);
	Ok(signed.into_encoded())
}

/// A pool-rejection that is terminal for this signed payload (no point
/// re-broadcasting the same bytes). Everything else is treated as transient.
/// Classification of an `author_submitExtrinsic` outcome. Mirrors the TS
/// `classifyAuthorRpcError` code mapping, matched on the error text since the
/// legacy RPC surfaces a message rather than a numeric code.
enum BroadcastClass {
	/// Accepted by the pool, or already imported (1013) — counts as broadcast.
	Accepted,
	/// Pool pressure / not-yet-valid: re-broadcast at the same nonce. Carries
	/// the canonical RPC code when recognised.
	Retryable(Option<u32>),
	/// Terminal validity failure: the nonce slot is unusable.
	Terminal(Option<u32>),
}

/// Classify a broadcast error message. Recognised retryable codes match the TS
/// strategy: `TemporarilyBanned` (1012), `TooLowPriority` (1014),
/// `ImmediatelyDropped` (1016), `FutureTransaction` (1021). Unrecognised errors
/// are treated as retryable-at-the-same-nonce — strictly safer for exactly-once
/// than TS's "unknown ⇒ terminal", since re-sending the identical extrinsic is
/// a no-op if it already reached the pool, whereas a fresh nonce could
/// double-submit the same content.
fn classify_broadcast_error(msg: &str) -> BroadcastClass {
	let m = msg.to_lowercase();
	if m.contains("already") {
		return BroadcastClass::Accepted;
	}
	if m.contains("temporarily banned") || m.contains("banned") {
		return BroadcastClass::Retryable(Some(1012));
	}
	if m.contains("too low priority") ||
		m.contains("priority is too low") ||
		m.contains("lowpriority")
	{
		return BroadcastClass::Retryable(Some(1014));
	}
	if m.contains("immediately dropped") || m.contains("dropped") {
		return BroadcastClass::Retryable(Some(1016));
	}
	if m.contains("future") {
		return BroadcastClass::Retryable(Some(1021));
	}
	if m.contains("stale") ||
		m.contains("badproof") ||
		m.contains("bad proof") ||
		m.contains("invalidtransaction") ||
		m.contains("invalid transaction") ||
		m.contains("payment")
	{
		return BroadcastClass::Terminal(None);
	}
	BroadcastClass::Retryable(None)
}

/// Mutable pipeline state, owned by the single driver task.
struct State {
	total: usize,
	complete_on: WaitFor,
	on_event: Option<UploadCallback>,
	limits: BlockLimits,

	// nonce bookkeeping
	item_nonce: Vec<Option<u64>>,
	next_free_nonce: u64,
	expected_final_nonce: u64,

	// queue
	send_queue: VecDeque<usize>,
	in_queue: BTreeSet<usize>,
	submission_anchor_block: Vec<Option<u32>>,
	retry_attempts: Vec<u32>,

	// reconciliation
	stored_at: Vec<Option<StoredLoc>>,
	in_block_emitted: Vec<bool>,
	finalized_emitted: Vec<bool>,
	failed_items: BTreeSet<usize>,

	// in-flight item bytes, fetched lazily via `ResolvedItem::get_data`. Holds
	// only items currently being (re-)broadcast; freed on finalization/failure
	// so resident memory tracks the in-flight window, not the whole upload.
	data_cache: BTreeMap<usize, ItemData>,

	// mortality anchor (finalized)
	anchor: (u64, H256),

	// watchdog
	max_confirmed_ever: usize,
	no_progress_best_blocks: u32,
}

impl State {
	fn emit(&self, ev: UploadEvent) {
		if let Some(cb) = &self.on_event {
			cb(ev);
		}
	}

	/// Items still outstanding for the configured completion level.
	fn pending(&self) -> usize {
		(0..self.total)
			.filter(|&i| {
				if self.failed_items.contains(&i) {
					return false;
				}
				match self.complete_on {
					WaitFor::InBlock => self.stored_at[i].is_none(),
					WaitFor::Finalized => !self.finalized_emitted[i],
				}
			})
			.count()
	}

	fn done(&self) -> bool {
		self.pending() == 0
	}
}

/// Core reconcile-driven wave pipeline over already-resolved (lazy) items.
/// Shared by the eager [`run_pipeline`] and the streamed
/// [`crate::transaction::TransactionClient::submit`] path. Drives all items to
/// `complete_on` and returns their CIDs in input order. `signer: None` runs the
/// unsigned (preimage-authorized) path: no nonce, no hijack detection, no
/// signing — each item broadcasts a bare extrinsic and is confirmed via TBCH.
///
/// Resilience has two layers. *Within* a streaming attempt: per-item
/// re-broadcast on pool pressure + hijack recovery + watchdogs. *Across* a
/// chainHead stall or disconnect: an outer retry-resume loop re-subscribes and
/// re-broadcasts not-yet-confirmed items at their carried nonce, up to
/// [`MAX_STALL_RETRIES`] with exponential backoff. One `State` persists across
/// attempts, so nonces and already-emitted events carry forward — combined with
/// the per-attempt `TransactionByContentHash` dedup, retries are exactly-once.
/// Mirrors the TS pipeline's outer retry + disconnect recovery.
pub(crate) async fn run_resolved(
	api: OnlineClient<PolkadotConfig>,
	rpc: LegacyRpcMethods<PolkadotConfig>,
	submit_rpcs: Vec<LegacyRpcMethods<PolkadotConfig>>,
	signer: Option<&Keypair>,
	resolved: Vec<ResolvedItem>,
	config: UploadConfig,
) -> Result<UploadResult> {
	let total = resolved.len();
	if total == 0 {
		return Ok(UploadResult { cids: Vec::new() });
	}

	let unsigned = signer.is_none();
	// Signer-derived account is only needed for the signed path's nonce reads.
	let account = signer.map(|s| AccountId32::from(s.public_key().0));

	// Bootstrap the initial mortality anchor + (signed) executed start nonce.
	let (anchor, start_nonce) = bootstrap_anchor(&api, &account).await?;

	let mut st = State {
		total,
		complete_on: config.complete_on,
		on_event: config.on_event.clone(),
		limits: config.block_limits,
		item_nonce: alloc::vec![None; total],
		next_free_nonce: start_nonce,
		expected_final_nonce: start_nonce + total as u64,
		send_queue: VecDeque::with_capacity(total),
		in_queue: BTreeSet::new(),
		submission_anchor_block: alloc::vec![None; total],
		retry_attempts: alloc::vec![0; total],
		stored_at: alloc::vec![None; total],
		in_block_emitted: alloc::vec![false; total],
		finalized_emitted: alloc::vec![false; total],
		failed_items: BTreeSet::new(),
		data_cache: BTreeMap::new(),
		anchor,
		max_confirmed_ever: 0,
		no_progress_best_blocks: 0,
	};

	// ItemStarted is emitted once per item here; the dedup pre-check + queueing
	// runs per attempt inside `drive_streams`.
	for (i, item) in resolved.iter().enumerate() {
		st.emit(UploadEvent::ItemStarted { index: i, total, cid: item.cid_bytes.clone() });
	}

	let strategy = build_strategy(config.submission_strategy, submit_rpcs);

	// Outer retry-resume loop.
	let mut attempt = 0u32;
	loop {
		match drive_streams(&api, &rpc, &strategy, signer, &account, unsigned, &resolved, &mut st)
			.await
		{
			Ok(()) => break,
			Err(e) => {
				// Only a stall/disconnect is resumable; deterministic errors
				// (e.g. an item exceeding the block budget) are returned at once.
				let resumable = matches!(e, Error::StoreStalled(_) | Error::NetworkError(_));
				if !resumable || attempt >= MAX_STALL_RETRIES {
					return Err(e);
				}
				attempt += 1;
				tokio::time::sleep(Duration::from_millis(STALL_RETRY_BACKOFF_MS << (attempt - 1)))
					.await;
			},
		}
	}

	Ok(UploadResult { cids: resolved.into_iter().map(|r| r.cid_bytes).collect() })
}

/// Subscribe to finalized blocks and read the first for the mortality anchor +
/// (signed only) the executed start nonce.
async fn bootstrap_anchor(
	api: &OnlineClient<PolkadotConfig>,
	account: &Option<AccountId32>,
) -> Result<((u64, H256), u64)> {
	let mut fin = api
		.blocks()
		.subscribe_finalized()
		.await
		.map_err(|e| Error::NetworkError(format!("subscribe_finalized: {e:?}")))?;
	let first = fin
		.next()
		.await
		.ok_or_else(|| Error::NetworkError("finalized stream ended".into()))?
		.map_err(|e| Error::NetworkError(format!("{e:?}")))?;
	let anchor = (u64::from(first.number()), first.hash());
	let start_nonce = match account {
		Some(acct) => chain_nonce(api, first.hash(), acct).await?,
		None => 0,
	};
	Ok((anchor, start_nonce))
}

/// One streaming attempt: (re-)subscribe, re-anchor, dedup-and-(re-)queue every
/// not-yet-confirmed item, then drive the wave loop. Returns `Err(StoreStalled)`
/// / `Err(NetworkError)` on a stall or disconnect so the caller can retry-resume;
/// `Ok(())` once every item reached `complete_on`.
// `i` indexes the parallel `st.*` and `resolved` arrays in the pre-check loop.
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
async fn drive_streams(
	api: &OnlineClient<PolkadotConfig>,
	rpc: &LegacyRpcMethods<PolkadotConfig>,
	strategy: &ActiveStrategy,
	signer: Option<&Keypair>,
	account: &Option<AccountId32>,
	unsigned: bool,
	resolved: &[ResolvedItem],
	st: &mut State,
) -> Result<()> {
	// (Re-)subscribe finalized and re-anchor from the first block.
	let mut fin_stream = api
		.blocks()
		.subscribe_finalized()
		.await
		.map_err(|e| Error::NetworkError(format!("subscribe_finalized: {e:?}")))?;
	let first_fin = fin_stream
		.next()
		.await
		.ok_or_else(|| Error::NetworkError("finalized stream ended".into()))?
		.map_err(|e| Error::NetworkError(format!("{e:?}")))?;
	st.anchor = (u64::from(first_fin.number()), first_fin.hash());
	// Fresh no-progress window for this attempt.
	st.no_progress_best_blocks = 0;

	// Dedup pre-check + (re-)queue. Items already on chain are reported finalized
	// and skipped; the rest are queued for (re-)broadcast at their carried nonce
	// (resetting the submission anchor so reconcile re-accepts a fresh inclusion).
	// Emits are guarded by `finalized_emitted`, so no duplicate events across
	// attempts.
	for i in 0..st.total {
		if st.failed_items.contains(&i) || st.finalized_emitted[i] {
			continue;
		}
		if let Some(loc) = tbch_lookup(api, st.anchor.1, resolved[i].content_hash).await {
			st.stored_at[i] = Some(loc);
			st.in_block_emitted[i] = true;
			st.finalized_emitted[i] = true;
			st.data_cache.remove(&i);
			st.emit(UploadEvent::ItemFinalized {
				index: i,
				total: st.total,
				cid: resolved[i].cid_bytes.clone(),
				block_hash: h256_hex(st.anchor.1),
				block_number: Some(loc.block_number),
				transaction_index: Some(loc.transaction_index),
			});
		} else if st.in_queue.insert(i) {
			st.submission_anchor_block[i] = None;
			st.send_queue.push_back(i);
		}
	}

	if st.done() {
		return Ok(());
	}

	// (Re-)subscribe best; merge with the remaining finalized stream.
	#[derive(Clone, Copy)]
	enum Kind {
		Best,
		Finalized,
	}
	let best_stream = api
		.blocks()
		.subscribe_best()
		.await
		.map_err(|e| Error::NetworkError(format!("subscribe_best: {e:?}")))?;
	let mut merged = futures::stream::select(
		best_stream.map(|r| r.map(|b| (Kind::Best, b))),
		fin_stream.map(|r| r.map(|b| (Kind::Finalized, b))),
	);

	loop {
		let next = tokio::time::timeout(STALL_TIMEOUT, merged.next()).await;
		let (kind, block) = match next {
			Err(_) =>
				return Err(Error::StoreStalled(format!(
					"no chainHead event for {}s",
					STALL_TIMEOUT.as_secs()
				))),
			Ok(None) => return Err(Error::StoreStalled("chainHead stream ended".into())),
			Ok(Some(Err(e))) => return Err(Error::NetworkError(format!("{e:?}"))),
			Ok(Some(Ok(v))) => v,
		};
		let block_hash = block.hash();
		let block_number = block.number();

		match kind {
			Kind::Finalized => {
				st.anchor = (u64::from(block.number()), block_hash);
				reconcile(api, st, resolved, block_hash, block_number, true).await;
			},
			Kind::Best => {
				reconcile(api, st, resolved, block_hash, block_number, false).await;

				// No-progress watchdog.
				let confirmed = st.stored_at.iter().filter(|s| s.is_some()).count();
				if confirmed > st.max_confirmed_ever {
					st.max_confirmed_ever = confirmed;
					st.no_progress_best_blocks = 0;
				} else {
					st.no_progress_best_blocks += 1;
					if st.no_progress_best_blocks > MAX_NO_PROGRESS_BEST_BLOCKS {
						return Err(Error::StoreStalled(format!(
							"no item confirmed in {MAX_NO_PROGRESS_BEST_BLOCKS} best blocks"
						)));
					}
				}

				// Hijack detection (signed only).
				let chain = match account {
					Some(acct) => chain_nonce(api, block_hash, acct).await?,
					None => 0,
				};
				if !unsigned && chain >= st.expected_final_nonce {
					detect_hijacks(st, resolved);
				}

				if st.done() {
					break;
				}

				// Dispatch the next wave. The pool nonce floor is signed-only.
				let pool = match account {
					Some(acct) => rpc
						.system_account_next_index(acct)
						.await
						.map_err(|e| Error::NetworkError(format!("{e:?}")))?,
					None => 0,
				};
				dispatch_wave(api, strategy, signer, st, resolved, pool, chain, block_number)
					.await?;
			},
		}

		if st.done() {
			break;
		}
	}

	Ok(())
}

/// Read `TransactionByContentHash` for every broadcast-but-unfinalized item at
/// `block_hash`; set/clear `stored_at` with reorg-aware clear-on-absence and
/// emit `ItemInBlock` / `ItemFinalized`.
async fn reconcile(
	api: &OnlineClient<PolkadotConfig>,
	st: &mut State,
	resolved: &[ResolvedItem],
	block_hash: H256,
	block_number: u32,
	finalized: bool,
) {
	let considered: Vec<usize> = (0..st.total)
		.filter(|&i| {
			!st.failed_items.contains(&i) &&
				(st.submission_anchor_block[i].is_some() || st.stored_at[i].is_some())
		})
		.collect();
	if considered.is_empty() {
		return;
	}

	// Batched reads at the pinned hash.
	let reads = considered.iter().map(|&i| {
		let content_hash = resolved[i].content_hash;
		async move { (i, tbch_lookup(api, block_hash, content_hash).await) }
	});
	let results = futures::future::join_all(reads).await;

	for (i, loc) in results {
		let anchor_floor = st.submission_anchor_block[i];
		match loc {
			// Only accept an entry at/after the block we first broadcast at —
			// a lower block is a pre-existing same-content entry, not ours.
			Some(l) if anchor_floor.is_none_or(|a| l.block_number >= a) => {
				st.stored_at[i] = Some(l);
				if !st.in_block_emitted[i] {
					st.in_block_emitted[i] = true;
					st.emit(UploadEvent::ItemInBlock {
						index: i,
						total: st.total,
						cid: resolved[i].cid_bytes.clone(),
						block_hash: h256_hex(block_hash),
						block_number: Some(l.block_number),
						transaction_index: Some(l.transaction_index),
					});
				}
				if finalized && !st.finalized_emitted[i] {
					st.finalized_emitted[i] = true;
					// Item is durably stored — release its in-flight bytes.
					st.data_cache.remove(&i);
					st.emit(UploadEvent::ItemFinalized {
						index: i,
						total: st.total,
						cid: resolved[i].cid_bytes.clone(),
						block_hash: h256_hex(block_hash),
						block_number: Some(l.block_number),
						transaction_index: Some(l.transaction_index),
					});
				}
			},
			// Absent (or pre-anchor) at this branch. Retract a prior inclusion
			// only if this block is at/after where the item was recorded —
			// then absence means it was reorged out, and ItemInBlock re-fires on
			// re-inclusion. Below the recorded height absence is expected (e.g. a
			// finalized reconcile lagging behind the best block we landed in), so
			// it must not retract.
			_ =>
				if let Some(prev) = st.stored_at[i] {
					if !st.finalized_emitted[i] && block_number >= prev.block_number {
						st.stored_at[i] = None;
						st.in_block_emitted[i] = false;
					}
				},
		}
	}
}

/// When the on-chain nonce has passed `expected_final_nonce`, any item that
/// holds a nonce yet isn't stored was hijacked: release its slot and re-queue
/// (or fail it after the retry budget).
// `i` indexes the parallel `st.*` and `resolved` arrays — a range loop is
// clearest here.
#[allow(clippy::needless_range_loop)]
fn detect_hijacks(st: &mut State, resolved: &[ResolvedItem]) {
	for i in 0..st.total {
		if st.failed_items.contains(&i) || st.stored_at[i].is_some() || st.item_nonce[i].is_none() {
			continue;
		}
		// This item's nonce slot executed without landing our content.
		st.item_nonce[i] = None;
		st.submission_anchor_block[i] = None;
		st.retry_attempts[i] += 1;
		if st.retry_attempts[i] > MAX_RETRY_ATTEMPTS {
			st.failed_items.insert(i);
			st.data_cache.remove(&i);
			st.emit(UploadEvent::ItemFailed {
				index: i,
				total: st.total,
				cid: resolved[i].cid_bytes.clone(),
				error: format!("nonce slot hijacked {MAX_RETRY_ATTEMPTS}+ times"),
			});
		} else if st.in_queue.insert(i) {
			st.send_queue.push_front(i);
		}
	}
}

// ───────────────────────────── submission strategy ─────────────────────────
//
// The pipeline never calls `author_submitExtrinsic` directly; it hands each
// signed wave to a `SubmissionStrategy`. Today only `NonceTrackingStrategy`
// exists (fan-out + chainHead reconcile). The trait + `ActiveStrategy` enum are
// the seam: a new strategy is a new impl + one enum arm, and the wave loop and
// `dispatch_wave` are untouched. Mirrors `submission-strategy.ts`.

/// Signed extrinsics ready to broadcast, in wave order. Mirrors TS `BroadcastArgs`.
pub struct BroadcastArgs<'a> {
	/// `(item index, SCALE-encoded signed extrinsic)` pairs.
	pub signed: &'a [(usize, Vec<u8>)],
}

/// Per-item broadcast outcome. Mirrors TS `ItemBroadcastResult`.
#[derive(Debug, Clone)]
pub struct ItemBroadcastResult {
	pub index: usize,
	/// At least one submission was accepted by the pool (or already imported).
	pub accepted: bool,
	/// When `!accepted`: `true` ⇒ re-broadcast at the same nonce; `false` ⇒
	/// terminal (drop the nonce, burn the retry budget).
	pub retryable: bool,
	/// Canonical RPC error code when recognised (TS `retryableCode`/`terminalCode`).
	pub code: Option<u32>,
}

/// Wave-level broadcast summary. Mirrors TS `WaveResult`.
#[derive(Debug, Default)]
pub struct WaveResult {
	pub txs_broadcast: u32,
	pub broadcast_errors: u32,
	pub terminal_code: Option<u32>,
	pub terminal_msg: Option<String>,
	pub retryable_count: u32,
	pub retryable_last_code: Option<u32>,
	pub item_results: Vec<ItemBroadcastResult>,
}

/// How signed extrinsics reach the chain. Mirrors the TS `SubmissionStrategy`
/// interface; kept as a seam so alternative strategies can be added without
/// touching the pipeline.
#[allow(async_fn_in_trait)]
pub trait SubmissionStrategy {
	/// Broadcast one signed wave and report per-item dispositions.
	async fn broadcast_wave(&self, args: BroadcastArgs<'_>) -> WaveResult;
	/// Release per-item resources once settled (no-op for nonce-tracking).
	fn on_item_settled(&self, _index: usize) {}
	/// Release all per-item resources (no-op for nonce-tracking).
	fn teardown(&self) {}
}

/// `author_submitExtrinsic` fan-out across every configured endpoint. An item
/// counts as broadcast if ANY endpoint accepts it (or already holds it), so a
/// single dead/lagging endpoint can't block a wave. Inclusion + hijack detection
/// are the reconciler's job. Mirrors the TS strategy's `submitClients` fan-out.
pub struct NonceTrackingStrategy {
	rpcs: Vec<LegacyRpcMethods<PolkadotConfig>>,
}

impl NonceTrackingStrategy {
	/// `rpcs` must be non-empty — one entry per broadcast endpoint.
	pub fn new(rpcs: Vec<LegacyRpcMethods<PolkadotConfig>>) -> Self {
		Self { rpcs }
	}
}

impl SubmissionStrategy for NonceTrackingStrategy {
	async fn broadcast_wave(&self, args: BroadcastArgs<'_>) -> WaveResult {
		// Each item fans out to every endpoint concurrently; the whole wave runs
		// concurrently too.
		let futs = args.signed.iter().map(|(idx, bytes)| {
			let idx = *idx;
			let rpcs = &self.rpcs;
			async move {
				let subs = rpcs.iter().map(|rpc| rpc.author_submit_extrinsic(bytes));
				(idx, futures::future::join_all(subs).await)
			}
		});
		let per_item = futures::future::join_all(futs).await;

		let mut res = WaveResult::default();
		for (idx, outcomes) in per_item {
			// Reduce this item's per-endpoint outcomes: accepted if any endpoint
			// took it; else retryable if any said so; else terminal.
			let mut accepted = false;
			let mut any_retryable = false;
			let mut retryable_code = None;
			let mut terminal_code = None;
			let mut terminal_msg = None;
			for out in outcomes {
				match out {
					Ok(_) => accepted = true,
					Err(e) => match classify_broadcast_error(&format!("{e:?}")) {
						BroadcastClass::Accepted => accepted = true,
						BroadcastClass::Retryable(code) => {
							any_retryable = true;
							retryable_code = retryable_code.or(code);
						},
						BroadcastClass::Terminal(code) =>
							if terminal_msg.is_none() {
								terminal_code = code;
								terminal_msg = Some(format!("{e:?}"));
							},
					},
				}
			}

			if accepted {
				res.txs_broadcast += 1;
				res.item_results.push(ItemBroadcastResult {
					index: idx,
					accepted: true,
					retryable: false,
					code: None,
				});
			} else if any_retryable {
				res.broadcast_errors += 1;
				res.retryable_count += 1;
				res.retryable_last_code = retryable_code.or(res.retryable_last_code);
				res.item_results.push(ItemBroadcastResult {
					index: idx,
					accepted: false,
					retryable: true,
					code: retryable_code,
				});
			} else {
				res.broadcast_errors += 1;
				if res.terminal_code.is_none() && res.terminal_msg.is_none() {
					res.terminal_code = terminal_code;
					res.terminal_msg = terminal_msg;
				}
				res.item_results.push(ItemBroadcastResult {
					index: idx,
					accepted: false,
					retryable: false,
					code: terminal_code,
				});
			}
		}
		res
	}
}

/// The concrete strategy selected for a run — one variant per
/// [`SubmissionStrategyKind`]. The pipeline drives it through
/// [`SubmissionStrategy`]; adding a strategy means adding a variant here, never
/// touching the wave loop.
enum ActiveStrategy {
	NonceTracking(NonceTrackingStrategy),
}

impl SubmissionStrategy for ActiveStrategy {
	async fn broadcast_wave(&self, args: BroadcastArgs<'_>) -> WaveResult {
		match self {
			ActiveStrategy::NonceTracking(s) => s.broadcast_wave(args).await,
		}
	}
	fn on_item_settled(&self, index: usize) {
		match self {
			ActiveStrategy::NonceTracking(s) => s.on_item_settled(index),
		}
	}
	fn teardown(&self) {
		match self {
			ActiveStrategy::NonceTracking(s) => s.teardown(),
		}
	}
}

fn build_strategy(
	kind: SubmissionStrategyKind,
	rpcs: Vec<LegacyRpcMethods<PolkadotConfig>>,
) -> ActiveStrategy {
	match kind {
		SubmissionStrategyKind::NonceTracking =>
			ActiveStrategy::NonceTracking(NonceTrackingStrategy::new(rpcs)),
	}
}

/// Select the next wave, build each item's extrinsic, and hand the wave to the
/// submission strategy.
///
/// Signed (`signer: Some`): assign each item one nonce against a pool-aware
/// floor and sign mortal against the finalized anchor; retryable pool rejections
/// keep the nonce and re-queue, terminal ones release it and burn the retry
/// budget. Unsigned (`signer: None`): build a bare (preimage-authorized)
/// extrinsic — no nonce, no retry budget — and re-queue anything the pool didn't
/// accept (a permanently-invalid item re-queues until the no-progress watchdog
/// fires). Mirrors the TS `pipelineStore` signed/unsigned split.
#[allow(clippy::too_many_arguments)]
async fn dispatch_wave(
	api: &OnlineClient<PolkadotConfig>,
	strategy: &ActiveStrategy,
	signer: Option<&Keypair>,
	st: &mut State,
	resolved: &[ResolvedItem],
	pool: u64,
	chain: u64,
	best_block_number: u32,
) -> Result<()> {
	if st.send_queue.is_empty() {
		return Ok(());
	}
	let unsigned = signer.is_none();
	let count = select_wave_count(&st.send_queue, resolved, &st.limits);
	if count == 0 {
		return Err(Error::StorageFailed("a single item exceeds the per-wave block budget".into()));
	}

	// Pop the wave.
	let wave: Vec<usize> = (0..count)
		.map(|_| {
			let idx = st.send_queue.pop_front().expect("count <= len");
			st.in_queue.remove(&idx);
			idx
		})
		.collect();

	// Signed only: assign each unassigned item a nonce from a pool-aware floor
	// = max(pool, chain, one-past-highest-claimed-nonce).
	if !unsigned {
		let mut floor = pool.max(chain);
		for n in st.item_nonce.iter().flatten() {
			floor = floor.max(n + 1);
		}
		let mut next = floor;
		for &idx in &wave {
			if st.item_nonce[idx].is_none() {
				st.item_nonce[idx] = Some(next);
				next += 1;
			}
		}
		st.next_free_nonce = next;
		st.expected_final_nonce = st.expected_final_nonce.max(next);
	}

	// Fetch each item's bytes lazily (cached while in flight), then build the
	// extrinsic and hand the wave to the strategy. On re-broadcast the cache hit
	// avoids a re-read; for streamed items the miss range-reads the seekable
	// source.
	let anchor = st.anchor;
	let mut signed: Vec<(usize, Vec<u8>)> = Vec::with_capacity(wave.len());
	for &idx in &wave {
		// Cheap `Arc` clone on a cache hit; a miss fetches (and caches) the bytes.
		let data = match st.data_cache.get(&idx) {
			Some(d) => d.clone(),
			None => {
				let d = (resolved[idx].get_data)().await?;
				st.data_cache.insert(idx, d.clone());
				d
			},
		};
		let bytes = match signer {
			Some(s) => {
				let nonce = st.item_nonce[idx].expect("assigned above");
				sign_store(api, s, &resolved[idx], &data, nonce, anchor)?
			},
			None => build_unsigned(api, &resolved[idx], &data)?,
		};
		signed.push((idx, bytes));
	}

	let result = strategy.broadcast_wave(BroadcastArgs { signed: &signed }).await;
	let terminal_msg = result.terminal_msg;

	for item in result.item_results {
		let idx = item.index;
		if item.accepted {
			st.submission_anchor_block[idx] = Some(best_block_number);
		} else if unsigned {
			// Unsigned: no nonce, no retry budget — just re-broadcast next wave.
			// A permanently-invalid item (e.g. content not preimage-authorized)
			// re-queues until the no-progress watchdog surfaces StoreStalled.
			if st.in_queue.insert(idx) {
				st.send_queue.push_front(idx);
			}
		} else if item.retryable {
			// Pool pressure / transient: keep the nonce, re-broadcast next wave.
			if st.in_queue.insert(idx) {
				st.send_queue.push_front(idx);
			}
		} else {
			// Terminal: release the nonce, get a fresh one, burn the retry budget.
			st.item_nonce[idx] = None;
			st.retry_attempts[idx] += 1;
			if st.retry_attempts[idx] > MAX_RETRY_ATTEMPTS {
				st.failed_items.insert(idx);
				st.data_cache.remove(&idx);
				st.emit(UploadEvent::ItemFailed {
					index: idx,
					total: st.total,
					cid: resolved[idx].cid_bytes.clone(),
					error: format!(
						"terminal broadcast rejection: {}",
						terminal_msg.as_deref().unwrap_or("unknown")
					),
				});
			} else if st.in_queue.insert(idx) {
				st.send_queue.push_front(idx);
			}
		}
	}
	Ok(())
}

/// Build a bare (unsigned) `store_with_cid_config` extrinsic — no nonce, no era,
/// no signature. Validity comes from the chain's `ValidateUnsigned`, which
/// admits the call only when the content hash is preimage-authorized.
fn build_unsigned(
	api: &OnlineClient<PolkadotConfig>,
	item: &ResolvedItem,
	data: &[u8],
) -> Result<Vec<u8>> {
	let cfg = bulletin::runtime_types::bulletin_transaction_storage_primitives::cids::CidConfig {
		codec: item.codec.code(),
		hashing: to_onchain_hashing(item.hash_algo),
	};
	let call = bulletin::tx().transaction_storage().store_with_cid_config(cfg, data.to_vec());
	let unsigned = api
		.tx()
		.create_unsigned(&call)
		.map_err(|e| Error::StorageFailed(format!("build unsigned: {e:?}")))?;
	Ok(unsigned.into_encoded())
}

/// Lowercase `0x`-prefixed hex of an H256, for event block-hash strings.
fn h256_hex(h: H256) -> String {
	let mut s = String::with_capacity(66);
	s.push_str("0x");
	for b in h.0.iter() {
		s.push_str(&format!("{b:02x}"));
	}
	s
}
