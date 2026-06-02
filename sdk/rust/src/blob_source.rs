// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Re-openable byte sources for streamed uploads.
//!
//! The estimate pass streams a [`BlobSource`] forward in `O(chunkSize)` memory
//! (hash each chunk, then free it). The submission pass fetches each chunk's
//! bytes on demand via a [`SeekableSource`]'s `read(offset, len)` and frees them
//! after finalization — so a large upload's resident memory tracks the in-flight
//! window, not the whole file. Direct port of the TypeScript SDK's
//! `blob-source.ts` + `planStream`.

use crate::{
	cid::{calculate_cid_with_config, CidCodec, CidData, HashingAlgorithm},
	dag::UnixFsDagBuilder,
	types::{ChunkerConfig, Error, Result, StoreOptions},
};
use alloc::{boxed::Box, format, sync::Arc, vec::Vec};
use futures::{future::BoxFuture, stream::BoxStream, StreamExt};

/// Item bytes shared between the in-flight cache and the signer without copying.
pub type ItemData = Arc<[u8]>;

/// Lazy per-item byte fetch. Called on every (re-)broadcast; the pipeline caches
/// the result while the item is in flight and frees it on finalization. Eager
/// callers return resident bytes; streamed callers range-read from a
/// [`SeekableSource`].
pub type GetData = Arc<dyn Fn() -> BoxFuture<'static, Result<ItemData>> + Send + Sync>;

/// Re-openable forward byte source. `open()` must be callable more than once and
/// yield the same bytes each time — that re-readability lets the estimate pass
/// and the submission pass share one source without buffering the whole file.
pub trait BlobSource: Send + Sync {
	/// Total byte length if known up front (lets `estimate_upload` size
	/// authorization without a full pass).
	fn size_hint(&self) -> Option<u64>;
	/// Open a fresh forward read from the start.
	fn open(&self) -> BoxStream<'_, Result<Vec<u8>>>;
}

/// A [`BlobSource`] that also supports random-access byte-range reads. Lazy
/// submission needs this: it fetches chunk `i` via `read(offsets[i], sizes[i])`
/// instead of holding the whole source in memory. A file satisfies both halves;
/// a forward-only stream satisfies only [`BlobSource`] and must be buffered (see
/// [`collect_blob`]).
///
/// Contract: `read` must return the same bytes the estimate pass hashed —
/// otherwise the precomputed CIDs won't match the chain's content hashes and the
/// reconciler will never confirm the upload.
pub trait SeekableSource: BlobSource {
	fn total_size(&self) -> u64;
	/// Read exactly `length` bytes starting at `offset`.
	fn read(&self, offset: u64, length: u64) -> BoxFuture<'_, Result<ItemData>>;
}

// ───────────────────────────── source constructors ─────────────────────────

/// In-memory bytes as a [`SeekableSource`] — `read` is a zero-copy slice; also
/// streamable for the estimate pass.
pub fn blob_from_bytes(data: Vec<u8>) -> impl SeekableSource {
	BytesSource { data: Arc::from(data) }
}

/// A re-openable stream factory (e.g. `|| fs::File::open(path)` adapted to a
/// byte stream). Forward-only, so submission buffers it via [`collect_blob`];
/// `size` is optional but lets the estimate size authorization eagerly.
pub fn blob_from_factory<F, S>(open: F, size: Option<u64>) -> impl BlobSource
where
	F: Fn() -> S + Send + Sync + 'static,
	S: futures::Stream<Item = Result<Vec<u8>>> + Send + 'static,
{
	FactorySource { open, size }
}

/// A [`SeekableSource`] over the concatenation of `items` — the source for the
/// items-as-is submission path. Reads are item-aligned, so a chunk-sized read at
/// an item boundary returns that item's bytes without copying.
pub fn blob_from_items(items: Vec<Vec<u8>>) -> impl SeekableSource {
	let mut offsets = Vec::with_capacity(items.len());
	let mut total = 0u64;
	for it in &items {
		offsets.push(total);
		total += it.len() as u64;
	}
	ItemsSource { items: items.into_iter().map(Arc::from).collect(), offsets, total }
}

/// Read a [`BlobSource`] fully into memory — `O(size)`. The fallback for
/// forward-only sources that can't be range-read for lazy submission.
pub async fn collect_blob(source: &dyn BlobSource) -> Result<Vec<u8>> {
	let mut out = Vec::with_capacity(source.size_hint().unwrap_or(0) as usize);
	let mut stream = source.open();
	while let Some(part) = stream.next().await {
		out.extend_from_slice(&part?);
	}
	Ok(out)
}

struct BytesSource {
	data: Arc<[u8]>,
}

impl BlobSource for BytesSource {
	fn size_hint(&self) -> Option<u64> {
		Some(self.data.len() as u64)
	}
	fn open(&self) -> BoxStream<'_, Result<Vec<u8>>> {
		let data = self.data.clone();
		Box::pin(futures::stream::once(async move { Ok(data.to_vec()) }))
	}
}

impl SeekableSource for BytesSource {
	fn total_size(&self) -> u64 {
		self.data.len() as u64
	}
	fn read(&self, offset: u64, length: u64) -> BoxFuture<'_, Result<ItemData>> {
		let data = self.data.clone();
		Box::pin(async move { slice_arc(&data, offset, length) })
	}
}

struct FactorySource<F> {
	open: F,
	size: Option<u64>,
}

impl<F, S> BlobSource for FactorySource<F>
where
	F: Fn() -> S + Send + Sync,
	S: futures::Stream<Item = Result<Vec<u8>>> + Send + 'static,
{
	fn size_hint(&self) -> Option<u64> {
		self.size
	}
	fn open(&self) -> BoxStream<'_, Result<Vec<u8>>> {
		Box::pin((self.open)())
	}
}

struct ItemsSource {
	items: Vec<Arc<[u8]>>,
	offsets: Vec<u64>,
	total: u64,
}

impl BlobSource for ItemsSource {
	fn size_hint(&self) -> Option<u64> {
		Some(self.total)
	}
	fn open(&self) -> BoxStream<'_, Result<Vec<u8>>> {
		let items = self.items.clone();
		Box::pin(futures::stream::iter(items.into_iter().map(|it| Ok(it.to_vec()))))
	}
}

impl SeekableSource for ItemsSource {
	fn total_size(&self) -> u64 {
		self.total
	}
	fn read(&self, offset: u64, length: u64) -> BoxFuture<'_, Result<ItemData>> {
		// Fast path: a read aligned to an item boundary that takes exactly that
		// item returns its bytes without copying.
		if let Ok(idx) = self.offsets.binary_search(&offset) {
			if self.items[idx].len() as u64 == length {
				let item = self.items[idx].clone();
				return Box::pin(async move { Ok(item) });
			}
		}
		// General path: gather a possibly-spanning range.
		let end = offset.saturating_add(length);
		if end > self.total {
			let total = self.total;
			return Box::pin(async move {
				Err(Error::InvalidConfig(format!("read {offset}+{length} past end (len {total})")))
			});
		}
		let mut out = Vec::with_capacity(length as usize);
		let mut pos = offset;
		// Find the item containing `pos`, then copy forward across items.
		let mut idx = self.offsets.partition_point(|&o| o <= offset).saturating_sub(1);
		while (out.len() as u64) < length {
			let item = &self.items[idx];
			let within = (pos - self.offsets[idx]) as usize;
			let take = core::cmp::min(length as usize - out.len(), item.len() - within);
			out.extend_from_slice(&item[within..within + take]);
			pos += take as u64;
			idx += 1;
		}
		Box::pin(async move { Ok(Arc::from(out)) })
	}
}

fn slice_arc(data: &Arc<[u8]>, offset: u64, length: u64) -> Result<ItemData> {
	let start = offset as usize;
	let end = start.checked_add(length as usize).filter(|&e| e <= data.len()).ok_or_else(|| {
		Error::InvalidConfig(format!("read {offset}+{length} past end (len {})", data.len()))
	})?;
	Ok(Arc::from(&data[start..end]))
}

// ──────────────────────────────── plan / estimate ──────────────────────────

/// Chunk CIDs, sizes, and byte offsets produced by one streaming pass over a
/// source — enough to size authorization and to fetch each chunk lazily via
/// `SeekableSource::read(offsets[i], chunk_sizes[i])`. Mirrors TS `ChunkPlan`.
#[derive(Debug, Clone)]
pub struct ChunkPlan {
	pub chunk_cids: Vec<CidData>,
	pub chunk_sizes: Vec<u64>,
	/// Cumulative byte offset of each chunk into the source.
	pub offsets: Vec<u64>,
	/// Per-unit CID codec (uniform `Raw` for file chunks; per-item for items).
	pub codecs: Vec<CidCodec>,
	/// Per-unit hashing algorithm (uniform for file chunks; per-item for items).
	pub hash_algos: Vec<HashingAlgorithm>,
	pub total_size: u64,
	/// UnixFS DAG-PB manifest root — the retrieval id. `None` if manifestless.
	pub root_cid: Option<CidData>,
	/// Encoded manifest bytes (the manifest `store` extrinsic payload).
	pub manifest_data: Option<Vec<u8>>,
}

/// Why a unit wouldn't be submitted. Mirrors TS `UploadEstimateItem.skipReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
	/// Same content hash as an earlier unit in the input (dedup_input).
	DuplicateInput,
	/// Already present in the chain's `TransactionByContentHash` (skip_existing).
	AlreadyOnChain,
}

/// Per-unit disposition in an [`UploadEstimate`]. Mirrors TS `UploadEstimateItem`.
#[derive(Debug, Clone)]
pub struct UploadEstimateItem {
	pub index: usize,
	pub cid: Vec<u8>,
	pub bytes: u64,
	/// `None` if the unit would be submitted.
	pub skip_reason: Option<SkipReason>,
}

/// Per-unit dispatch plan + the aggregated `transactions` / `bytes` the chain
/// would charge. Use it to size authorization or preview an upload. Mirrors TS
/// `UploadEstimate`.
#[derive(Debug, Clone)]
pub struct UploadEstimate {
	/// Number of units in the plan (chunks + manifest).
	pub total: usize,
	/// Per-unit disposition, parallel to the plan.
	pub items: Vec<UploadEstimateItem>,
	/// `store` extrinsics that would be submitted (= `to_upload.len()`).
	pub transactions: u32,
	/// Total bytes the submitted txs would consume.
	pub bytes: u64,
	/// Indices duplicating an earlier unit by content hash (dedup_input).
	pub duplicate_indices: Vec<usize>,
	/// Indices already on chain at estimate time (only if `skip_existing`).
	pub already_stored: Vec<usize>,
	/// Indices that would be submitted.
	pub to_upload: Vec<usize>,
}

/// Options for [`crate::transaction::TransactionClient::estimate_upload`].
/// Mirrors TS `UploadEstimateOptions`, plus the chunk/store config the Rust
/// client needs for the `Source` path (the TS client holds these on itself).
#[derive(Debug, Clone)]
pub struct UploadEstimateOptions {
	/// Query `TransactionByContentHash` and exclude units already on chain
	/// (one RPC per unique content hash). Default `false`.
	pub skip_existing: bool,
	/// Collapse repeated content hashes within the input (first occurrence wins;
	/// later indices land in `duplicate_indices`). Default `true` — the chain
	/// dedupes by content hash anyway, so charging for duplicates is wasteful.
	pub dedup_input: bool,
	/// Chunking config for a `Source` input (ignored for `Items`).
	pub chunker: ChunkerConfig,
	/// Store options (hash algorithm) for a `Source` input (ignored for `Items`).
	pub store: StoreOptions,
}

impl Default for UploadEstimateOptions {
	fn default() -> Self {
		Self {
			skip_existing: false,
			dedup_input: true,
			chunker: ChunkerConfig::default(),
			store: StoreOptions::default(),
		}
	}
}

/// An [`UploadEstimate`] plus the [`ChunkPlan`] that produced it — pass the same
/// plan to `submit` to avoid re-hashing. Mirrors TS `StreamEstimate`.
#[derive(Debug, Clone)]
pub struct StreamEstimate {
	pub base: UploadEstimate,
	pub plan: ChunkPlan,
}

/// Stream a [`BlobSource`] once and produce a [`ChunkPlan`] — chunk CIDs, sizes,
/// offsets, and (optionally) the manifest — without holding the file in memory.
/// Peak working memory is ~`chunk_size` plus the CID list. Mirrors TS
/// `chunkStream` + `planStream`.
pub async fn plan_stream<S: BlobSource + ?Sized>(
	source: &S,
	config: &ChunkerConfig,
	options: &StoreOptions,
) -> Result<ChunkPlan> {
	let chunk_size = config.chunk_size as usize;
	if chunk_size == 0 {
		return Err(Error::InvalidChunkSize("chunk size must be greater than 0".into()));
	}
	let hash_algo = options.hash_algorithm;

	let mut chunk_cids = Vec::new();
	let mut chunk_sizes = Vec::new();
	let mut offsets = Vec::new();
	let mut total: u64 = 0;
	let mut buf: Vec<u8> = Vec::new();

	let hash_chunk = |chunk: &[u8],
	                  total: &mut u64,
	                  chunk_cids: &mut Vec<CidData>,
	                  chunk_sizes: &mut Vec<u64>,
	                  offsets: &mut Vec<u64>|
	 -> Result<()> {
		offsets.push(*total);
		chunk_cids.push(calculate_cid_with_config(chunk, CidCodec::Raw, hash_algo)?);
		chunk_sizes.push(chunk.len() as u64);
		*total += chunk.len() as u64;
		Ok(())
	};

	let mut stream = source.open();
	while let Some(part) = stream.next().await {
		let part = part?;
		if part.is_empty() {
			continue;
		}
		buf.extend_from_slice(&part);
		while buf.len() >= chunk_size {
			let chunk: Vec<u8> = buf.drain(..chunk_size).collect();
			hash_chunk(&chunk, &mut total, &mut chunk_cids, &mut chunk_sizes, &mut offsets)?;
		}
	}
	if !buf.is_empty() {
		hash_chunk(&buf, &mut total, &mut chunk_cids, &mut chunk_sizes, &mut offsets)?;
	}
	if chunk_cids.is_empty() {
		return Err(Error::EmptyData);
	}

	let codecs = alloc::vec![CidCodec::Raw; chunk_cids.len()];
	let hash_algos = alloc::vec![hash_algo; chunk_cids.len()];
	let mut plan = ChunkPlan {
		chunk_cids,
		chunk_sizes,
		offsets,
		codecs,
		hash_algos,
		total_size: total,
		root_cid: None,
		manifest_data: None,
	};
	if config.create_manifest {
		let manifest = UnixFsDagBuilder::new().build_from_parts(
			&plan.chunk_cids,
			&plan.chunk_sizes,
			hash_algo,
		)?;
		plan.root_cid = Some(manifest.root_cid);
		plan.manifest_data = Some(manifest.dag_bytes);
	}
	Ok(plan)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{
		chunker::{Chunker, FixedSizeChunker},
		dag::DagBuilder,
		types::Chunk,
	};

	fn make_data(size: usize) -> Vec<u8> {
		// Deterministic non-repeating bytes so chunks have distinct content.
		let mut x: u32 = 0x9e37_79b9;
		(0..size)
			.map(|_| {
				x ^= x << 13;
				x ^= x >> 17;
				x ^= x << 5;
				(x & 0xff) as u8
			})
			.collect()
	}

	/// A source that yields `data` in arbitrary-sized parts, to stress the
	/// re-chunker's remainder carry across input boundaries.
	fn blob_from_parts(parts: Vec<Vec<u8>>) -> impl BlobSource {
		let total = parts.iter().map(|p| p.len() as u64).sum();
		blob_from_factory(
			move || futures::stream::iter(parts.clone().into_iter().map(Ok)),
			Some(total),
		)
	}

	#[tokio::test]
	async fn collect_blob_concatenates_parts() {
		let data = make_data(5000);
		let parts = vec![data[..1000].to_vec(), data[1000..4096].to_vec(), data[4096..].to_vec()];
		let got = collect_blob(&blob_from_parts(parts)).await.unwrap();
		assert_eq!(got, data);
	}

	#[tokio::test]
	async fn seekable_read_round_trips() {
		let data = make_data(2048);
		let src = blob_from_bytes(data.clone());
		assert_eq!(src.total_size(), 2048);
		assert_eq!(&*src.read(100, 50).await.unwrap(), &data[100..150]);
		// Out-of-bounds read is an error, not a panic.
		assert!(src.read(2000, 100).await.is_err());
	}

	#[tokio::test]
	async fn plan_stream_reslices_arbitrary_boundaries() {
		const MIB: usize = 1024 * 1024;
		let data = make_data(3 * MIB + 777);
		// Pathological part boundaries: tiny, huge, off-by-one.
		let parts = vec![
			data[..1].to_vec(),
			data[1..MIB + 5].to_vec(),
			data[MIB + 5..3 * MIB].to_vec(),
			data[3 * MIB..].to_vec(),
		];
		let cfg = ChunkerConfig { chunk_size: MIB as u32, max_parallel: 8, create_manifest: true };
		let plan = plan_stream(&blob_from_parts(parts), &cfg, &StoreOptions::default())
			.await
			.unwrap();

		// 3 full MiB chunks + one 777-byte remainder.
		assert_eq!(plan.chunk_sizes, vec![MIB as u64, MIB as u64, MIB as u64, 777]);
		assert_eq!(plan.offsets, vec![0, MIB as u64, 2 * MIB as u64, 3 * MIB as u64]);
		assert_eq!(plan.total_size, data.len() as u64);
		assert!(plan.root_cid.is_some());
	}

	#[tokio::test]
	async fn plan_stream_matches_in_memory_chunker() {
		const MIB: usize = 1024 * 1024;
		let data = make_data(2 * MIB + 123);
		let cfg = ChunkerConfig { chunk_size: MIB as u32, max_parallel: 8, create_manifest: true };

		// Streamed plan vs the eager chunker + DAG builder must agree on every CID.
		let plan = plan_stream(&blob_from_bytes(data.clone()), &cfg, &StoreOptions::default())
			.await
			.unwrap();
		let chunks: Vec<Chunk> = FixedSizeChunker::new(cfg.clone()).unwrap().chunk(&data).unwrap();
		let manifest =
			UnixFsDagBuilder::new().build(&chunks, HashingAlgorithm::Blake2b256).unwrap();

		assert_eq!(plan.chunk_cids.len(), chunks.len());
		for (a, b) in plan.chunk_cids.iter().zip(&manifest.chunk_cids) {
			assert_eq!(a.content_hash, b.content_hash);
		}
		assert_eq!(plan.root_cid.unwrap().content_hash, manifest.root_cid.content_hash);
	}

	#[tokio::test]
	async fn plan_stream_rejects_empty_and_zero_chunk() {
		let cfg = ChunkerConfig::default();
		assert!(plan_stream(&blob_from_bytes(Vec::new()), &cfg, &StoreOptions::default())
			.await
			.is_err());
		let bad = ChunkerConfig { chunk_size: 0, ..ChunkerConfig::default() };
		assert!(plan_stream(&blob_from_bytes(make_data(10)), &bad, &StoreOptions::default())
			.await
			.is_err());
	}

	#[tokio::test]
	async fn blob_from_items_reads_item_aligned() {
		let a = make_data(100);
		let b = make_data(200);
		let src = blob_from_items(vec![a.clone(), b.clone()]);
		assert_eq!(src.total_size(), 300);
		assert_eq!(&*src.read(0, 100).await.unwrap(), &a[..]);
		assert_eq!(&*src.read(100, 200).await.unwrap(), &b[..]);
		// Spanning read across the item boundary.
		let span = src.read(50, 100).await.unwrap();
		assert_eq!(&span[..50], &a[50..]);
		assert_eq!(&span[50..], &b[..50]);
	}
}
