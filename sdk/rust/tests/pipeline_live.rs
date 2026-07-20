// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Live integration test for the wave-batched upload pipeline.
//!
//! Requires a running Bulletin chain whose genesis pre-authorizes `//Alice` to
//! store and `//Eve` as an authorizer (the local zombienet preset does). The
//! endpoint comes from `BULLETIN_RPC_URL` (default `ws://localhost:10000`,
//! matching the TS suite) and the second collator from `BULLETIN_RPC_URL_2`
//! (default `ws://localhost:12346`).
//!
//! Ignored by default — these share the `//Alice` account so run them
//! **serially**:
//!
//! ```text
//! cargo test -p bulletin-sdk-rust --test pipeline_live -- --ignored --test-threads=1 --nocapture
//! ```

use bulletin_sdk_rust::prelude::*;
use std::{
	collections::HashMap,
	sync::{
		atomic::{AtomicUsize, Ordering},
		Arc, Mutex,
	},
};
use subxt::utils::AccountId32;
use subxt_signer::sr25519::{dev, Keypair};

/// Monitor/RPC endpoint. Override with `BULLETIN_RPC_URL` (same env var as the
/// TS integration suite); defaults to the local zombienet collator.
fn ws() -> String {
	std::env::var("BULLETIN_RPC_URL").unwrap_or_else(|_| "ws://localhost:10000".to_string())
}

/// Second collator endpoint for the multi-provider fan-out test. Override with
/// `BULLETIN_RPC_URL_2`.
fn ws2() -> String {
	std::env::var("BULLETIN_RPC_URL_2").unwrap_or_else(|_| "ws://localhost:12346".to_string())
}

/// Upload in-memory items through the single `estimate_upload` → `submit`
/// primitive: plan the items (no manifest), then submit against a
/// `blob_from_items` source. Mirrors how the TS SDK uploads items.
async fn upload_items(
	client: &TransactionClient,
	signer: &Keypair,
	items: Vec<UploadItem>,
	config: UploadConfig,
) -> Result<UploadResult> {
	let datas: Vec<Vec<u8>> = items.iter().map(|i| i.data.clone()).collect();
	let est = client
		.estimate_upload(UploadInput::Items(items), UploadEstimateOptions::default())
		.await?;
	let source: Arc<dyn SeekableSource> = Arc::new(blob_from_items(datas));
	client.submit(signer, est, source, config).await
}

/// Unsigned (preimage-authorized) variant of [`upload_items`].
async fn upload_items_unsigned(
	client: &TransactionClient,
	items: Vec<UploadItem>,
	config: UploadConfig,
) -> Result<UploadResult> {
	let datas: Vec<Vec<u8>> = items.iter().map(|i| i.data.clone()).collect();
	let est = client
		.estimate_upload(UploadInput::Items(items), UploadEstimateOptions::default())
		.await?;
	let source: Arc<dyn SeekableSource> = Arc::new(blob_from_items(datas));
	client.submit_unsigned(est, source, config).await
}

/// Upload several items in one wave through the pipeline; assert all CIDs come
/// back and the per-item event stream fires. Validates concurrent submission,
/// pool-nonce flooring (distinct nonces), the finalized-era anchor, the TBCH
/// slot lookup, and exactly-once on a re-run.
#[tokio::test]
#[ignore]
async fn pipeline_uploads_a_wave() {
	let client = TransactionClient::new(&ws()).await.expect("connect");
	let alice = dev::alice();

	// Unique per run so the first upload submits fresh content and the
	// skip_existing re-run below is meaningful.
	let nonce_seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos();
	let items: Vec<UploadItem> = (0..3u8)
		.map(|i| {
			let mut data = format!("pipeline-live {nonce_seed} #{i} ").into_bytes();
			data.resize(256 + i as usize, i);
			UploadItem::new(data)
		})
		.collect();

	let events = Arc::new(Mutex::new(Vec::<UploadEvent>::new()));
	let sink = events.clone();
	// Finalize so the re-run below reliably hits the TBCH dedup (lookup is at
	// the finalized anchor).
	let config = UploadConfig {
		complete_on: WaitFor::Finalized,
		block_limits: DEFAULT_BLOCK_LIMITS,
		on_event: Some(Arc::new(move |ev| sink.lock().unwrap().push(ev))),
		submission_strategy: SubmissionStrategyKind::NonceTracking,
	};

	let result = upload_items(&client, &alice, items.clone(), config).await.expect("upload");

	assert_eq!(result.cids.len(), 3, "one CID per item");
	for cid in &result.cids {
		assert!(!cid.is_empty(), "non-empty CID");
	}

	{
		let evs = events.lock().unwrap();
		let started = evs.iter().filter(|e| matches!(e, UploadEvent::ItemStarted { .. })).count();
		let in_block = evs.iter().filter(|e| matches!(e, UploadEvent::ItemInBlock { .. })).count();
		let finalized =
			evs.iter().filter(|e| matches!(e, UploadEvent::ItemFinalized { .. })).count();
		let failed = evs.iter().filter(|e| matches!(e, UploadEvent::ItemFailed { .. })).count();
		// Every finalized event should carry the renew slot from the TBCH lookup.
		let with_slot = evs
			.iter()
			.filter(|e| matches!(e, UploadEvent::ItemFinalized { transaction_index: Some(_), .. }))
			.count();
		println!(
			"events: started={started} in_block={in_block} finalized={finalized} with_slot={with_slot} failed={failed} total={}",
			evs.len()
		);
		assert_eq!(started, 3, "ItemStarted per item");
		assert_eq!(finalized, 3, "ItemFinalized per item");
		assert_eq!(failed, 0, "no failures");
		assert_eq!(with_slot, 3, "TBCH lookup populated the renew transaction_index");
	}

	// Re-upload the same (now-finalized) items with `skip_existing`: the
	// estimate marks every unit already-on-chain, submit skips them all (no
	// re-store, no payment) and still returns the same CIDs. Without
	// `skip_existing` a re-run would re-store on purpose (pays, refreshes
	// retention).
	let datas: Vec<Vec<u8>> = items.iter().map(|i| i.data.clone()).collect();
	let est2 = client
		.estimate_upload(
			UploadInput::Items(items),
			UploadEstimateOptions { skip_existing: true, ..Default::default() },
		)
		.await
		.expect("re-estimate");
	assert_eq!(est2.base.already_stored.len(), 3, "all three units found on chain");
	assert_eq!(est2.base.transactions, 0, "nothing left to submit");
	let source: Arc<dyn SeekableSource> = Arc::new(blob_from_items(datas));
	let result2 = client
		.submit(&alice, est2, source, UploadConfig::default())
		.await
		.expect("re-upload");
	assert_eq!(result2.cids, result.cids, "re-run returns identical CIDs (exactly-once)");
	println!(
		"exactly-once re-run OK: {} CIDs returned, all skipped as already-stored",
		result2.cids.len()
	);
}

/// Upload a 64 MiB file as 1 MiB chunks (64 items) through the pipeline. This
/// is the multi-wave stress: the wave budget (~18 MiB of length) fits ~17
/// chunks, so the run spans ~4 waves — exercising cross-wave nonce flooring
/// (each wave re-floors off the pool nonce after the previous wave is in
/// flight) that a single-wave upload can't reach.
///
/// Alice is authorized with ample capacity first (via Eve, the genesis
/// authorizer) so the run isn't a soft-cap edge case.
#[tokio::test]
#[ignore]
async fn pipeline_uploads_64mib_multiwave() {
	const CHUNK: usize = 1024 * 1024; // 1 MiB
	const CHUNKS: usize = 64; // 64 MiB total

	let client = TransactionClient::new(&ws()).await.expect("connect");
	let alice = dev::alice();
	let eve = dev::eve();
	let alice_acct = AccountId32::from(alice.public_key().0);

	// Authorize Alice for plenty of transactions + bytes (Eve is in the
	// genesis allowed_authorizers set).
	client
		.authorize_account(alice_acct, 200, 128 * 1024 * 1024, &eve, WaitFor::Finalized)
		.await
		.expect("authorize alice");

	// 64 distinct 1 MiB chunks, unique per run: each chunk embeds the run seed
	// + its index so no two chunks share a content hash (which would be deduped)
	// and a re-run wouldn't collide with a previous run.
	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos()
		.to_le_bytes();
	let items: Vec<UploadItem> = (0..CHUNKS)
		.map(|i| {
			let mut data = vec![i as u8; CHUNK];
			data[..16].copy_from_slice(&seed);
			data[16..24].copy_from_slice(&(i as u64).to_le_bytes());
			UploadItem::new(data)
		})
		.collect();

	let started = Arc::new(AtomicUsize::new(0));
	let in_block = Arc::new(AtomicUsize::new(0));
	let failed = Arc::new(AtomicUsize::new(0));
	let (s, b, f) = (started.clone(), in_block.clone(), failed.clone());
	let config = UploadConfig {
		complete_on: WaitFor::InBlock, // multi-wave coverage without per-wave finality wait
		block_limits: DEFAULT_BLOCK_LIMITS,
		on_event: Some(Arc::new(move |ev| match ev {
			UploadEvent::ItemStarted { .. } => {
				s.fetch_add(1, Ordering::SeqCst);
			},
			UploadEvent::ItemInBlock { .. } => {
				b.fetch_add(1, Ordering::SeqCst);
			},
			UploadEvent::ItemFailed { index, error, .. } => {
				f.fetch_add(1, Ordering::SeqCst);
				eprintln!("item {index} failed: {error}");
			},
			_ => {},
		})),
		submission_strategy: SubmissionStrategyKind::NonceTracking,
	};

	let t0 = std::time::Instant::now();
	let result = upload_items(&client, &alice, items, config).await.expect("64 MiB upload");
	let secs = t0.elapsed().as_secs_f64();

	println!(
		"64 MiB / {CHUNKS} chunks uploaded in {secs:.1}s — started={} in_block={} failed={} cids={}",
		started.load(Ordering::SeqCst),
		in_block.load(Ordering::SeqCst),
		failed.load(Ordering::SeqCst),
		result.cids.len(),
	);

	assert_eq!(result.cids.len(), CHUNKS, "one CID per chunk");
	assert_eq!(started.load(Ordering::SeqCst), CHUNKS, "ItemStarted per chunk");
	assert_eq!(in_block.load(Ordering::SeqCst), CHUNKS, "every chunk reached a block");
	assert_eq!(failed.load(Ordering::SeqCst), 0, "no chunk failed");

	// All CIDs distinct (no accidental dedup of distinct chunks).
	let mut sorted = result.cids.clone();
	sorted.sort();
	sorted.dedup();
	assert_eq!(sorted.len(), CHUNKS, "all chunk CIDs distinct");
}

/// Hijack recovery: two pipelines upload from the SAME signer (//Alice) in
/// parallel, racing for the same nonces. Each pipeline's hijack detection
/// reassigns fresh nonces (pool-aware floor) so both uploads finish. Mirrors
/// the TS "two parallel uploads from the same signer both succeed" test.
#[tokio::test]
#[ignore]
async fn pipeline_hijack_two_parallel_same_signer() {
	// Separate connections, same signer → they fight over the same nonces.
	let client_a = TransactionClient::new(&ws()).await.expect("connect A");
	let client_b = TransactionClient::new(&ws()).await.expect("connect B");
	let alice = dev::alice();

	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos();
	// Distinct content → distinct content hashes → each must land at its own
	// nonce (the pool only dedupes identical content, not distinct payloads).
	let data_a = format!("hijack-A {seed}").into_bytes();
	let data_b = format!("hijack-B {seed}").into_bytes();

	let (res_a, res_b) = tokio::join!(
		upload_items(&client_a, &alice, vec![UploadItem::new(data_a)], UploadConfig::default()),
		upload_items(&client_b, &alice, vec![UploadItem::new(data_b)], UploadConfig::default()),
	);
	let res_a = res_a.expect("upload A");
	let res_b = res_b.expect("upload B");

	assert_eq!(res_a.cids.len(), 1, "A returns one CID");
	assert_eq!(res_b.cids.len(), 1, "B returns one CID");
	assert_ne!(res_a.cids[0], res_b.cids[0], "distinct content → distinct CIDs");
	println!("hijack recovery OK: both parallel same-signer uploads finished with distinct CIDs");
}

/// Exactly-once accounting under concurrent same-account upload — the
/// "never pay twice for the same CID" invariant. Each successful `store`
/// advances the caller's `Authorizations.extent` by one transaction and
/// `data.len()` bytes. If the pipeline ever double-broadcasts a chunk as a
/// second distinct tx (e.g. on a watchdog retry) the extent would drift from
/// the exact sum. We snapshot the remaining allowance before/after N parallel
/// same-signer uploads and assert the consumed delta equals the exact sum.
/// Mirrors the TS "3 parallel uploads → extent advances by exact sum" test.
#[tokio::test]
#[ignore]
async fn pipeline_exactly_once_accounting_parallel() {
	const SESSIONS: usize = 3;
	const ITEMS_PER: usize = 8;
	const ITEM_SIZE: usize = 256 * 1024; // 256 KiB

	let client = TransactionClient::new(&ws()).await.expect("connect");
	let alice = dev::alice();
	let eve = dev::eve();
	let alice_acct = AccountId32::from(alice.public_key().0);

	// Generous additive headroom so usage never reaches the allowance (a
	// clamped `remaining` would break the delta). Eve is a genesis authorizer.
	let total_items = (SESSIONS * ITEMS_PER) as u32;
	client
		.authorize_account(
			alice_acct.clone(),
			total_items + 50,
			(total_items as u64 + 50) * ITEM_SIZE as u64,
			&eve,
			WaitFor::Finalized,
		)
		.await
		.expect("authorize alice");

	// Snapshot remaining allowance BEFORE.
	let (rem_tx_before, rem_bytes_before) = client
		.query_account_authorization(&alice_acct)
		.await
		.expect("query before")
		.expect("alice authorized");

	// SESSIONS × ITEMS_PER unique payloads: a per-run seed + (session, item)
	// index guarantee distinct content hashes (no dedup across the batch) and
	// distinct from any earlier run.
	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos()
		.to_le_bytes();
	let make_session = |s: usize| -> Vec<UploadItem> {
		(0..ITEMS_PER)
			.map(|i| {
				let mut data = vec![(s * ITEMS_PER + i) as u8; ITEM_SIZE];
				data[..16].copy_from_slice(&seed);
				data[16..24].copy_from_slice(&(s as u64).to_le_bytes());
				data[24..32].copy_from_slice(&(i as u64).to_le_bytes());
				UploadItem::new(data)
			})
			.collect()
	};

	// 3 separate clients sharing //Alice — emulates 3 scripts racing for nonces.
	let c0 = TransactionClient::new(&ws()).await.expect("connect 0");
	let c1 = TransactionClient::new(&ws()).await.expect("connect 1");
	let c2 = TransactionClient::new(&ws()).await.expect("connect 2");
	let cfg = || UploadConfig { complete_on: WaitFor::Finalized, ..Default::default() };

	let (r0, r1, r2) = tokio::join!(
		upload_items(&c0, &alice, make_session(0), cfg()),
		upload_items(&c1, &alice, make_session(1), cfg()),
		upload_items(&c2, &alice, make_session(2), cfg()),
	);
	let results = [r0.expect("session 0"), r1.expect("session 1"), r2.expect("session 2")];
	for (s, r) in results.iter().enumerate() {
		assert_eq!(r.cids.len(), ITEMS_PER, "session {s} returns ITEMS_PER CIDs");
	}

	// All CIDs across all sessions distinct (no accidental dedup).
	let mut all_cids: Vec<_> = results.iter().flat_map(|r| r.cids.clone()).collect();
	let distinct = all_cids.len();
	all_cids.sort();
	all_cids.dedup();
	assert_eq!(all_cids.len(), distinct, "all CIDs across sessions distinct");
	assert_eq!(distinct, SESSIONS * ITEMS_PER, "one CID per item");

	// Snapshot remaining allowance AFTER.
	let (rem_tx_after, rem_bytes_after) = client
		.query_account_authorization(&alice_acct)
		.await
		.expect("query after")
		.expect("alice still authorized");

	let tx_consumed = rem_tx_before - rem_tx_after;
	let bytes_consumed = rem_bytes_before - rem_bytes_after;
	println!(
		"exactly-once accounting: consumed tx={tx_consumed} bytes={bytes_consumed} (expected tx={total_items} bytes={})",
		total_items as u64 * ITEM_SIZE as u64
	);
	assert_eq!(
		tx_consumed, total_items,
		"extent advanced by exactly one tx per item — no double-pay"
	);
	assert_eq!(
		bytes_consumed,
		total_items as u64 * ITEM_SIZE as u64,
		"extent advanced by exactly the total bytes — no double-pay"
	);
}

/// Phase 3 streaming submission: `estimate_upload` plans an 8 MiB file in
/// `O(chunk)` memory (8 × 1 MiB chunks + a DAG-PB manifest), then `submit`
/// stores it fetching each chunk's bytes lazily via the seekable source's
/// `read(offset, size)`. Asserts the manifest root + every chunk CID come back,
/// and that the on-chain `Authorizations.extent` advances by exactly the
/// estimate's planned amount — exactly-once for the streamed path.
#[tokio::test]
#[ignore]
async fn submit_streams_a_file_lazily() {
	const CHUNK: usize = 1024 * 1024; // 1 MiB
	const CHUNKS: usize = 8;
	const FILE: usize = CHUNK * CHUNKS; // 8 MiB

	let client = TransactionClient::new(&ws()).await.expect("connect");
	let alice = dev::alice();
	let eve = dev::eve();
	let alice_acct = AccountId32::from(alice.public_key().0);

	client
		.authorize_account(alice_acct.clone(), 100, 64 * 1024 * 1024, &eve, WaitFor::Finalized)
		.await
		.expect("authorize alice");

	// Unique-per-run 8 MiB blob; embed the run seed + chunk index in each chunk
	// so no two chunks (or runs) share a content hash.
	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos()
		.to_le_bytes();
	let mut data = vec![0u8; FILE];
	for (i, b) in data.iter_mut().enumerate() {
		*b = (i as u8) ^ seed[i & 7];
	}
	for c in 0..CHUNKS {
		let off = c * CHUNK;
		data[off..off + 16].copy_from_slice(&seed);
		data[off + 16..off + 24].copy_from_slice(&(c as u64).to_le_bytes());
	}

	let cfg = ChunkerConfig { chunk_size: CHUNK as u32, max_parallel: 8, create_manifest: true };

	// Plan + size authorization without buffering.
	let src: Arc<dyn SeekableSource> = Arc::new(blob_from_bytes(data.clone()));
	let est = client
		.estimate_upload(
			UploadInput::Source(src.clone()),
			UploadEstimateOptions { chunker: cfg, ..Default::default() },
		)
		.await
		.expect("estimate");
	println!(
		"estimate: tx={} bytes={} already_stored={:?} chunks={} manifest={}",
		est.base.transactions,
		est.base.bytes,
		est.base.already_stored,
		est.plan.chunk_cids.len(),
		est.plan.root_cid.is_some(),
	);
	assert_eq!(est.plan.chunk_cids.len(), CHUNKS, "8 chunk CIDs planned");
	assert!(est.plan.root_cid.is_some(), "manifest planned");
	assert_eq!(est.base.transactions, (CHUNKS + 1) as u32, "8 chunks + manifest, none stored yet");
	let expected_tx = est.base.transactions;
	let expected_bytes = est.base.bytes;

	// Accounting snapshot BEFORE.
	let (rem_tx_before, rem_bytes_before) = client
		.query_account_authorization(&alice_acct)
		.await
		.expect("query before")
		.expect("auth");

	// Stream the file in: each chunk's bytes are range-read on demand.
	let result = client
		.submit(
			&alice,
			est,
			src,
			UploadConfig { complete_on: WaitFor::Finalized, ..Default::default() },
		)
		.await
		.expect("submit");

	assert_eq!(result.cids.len(), CHUNKS + 1, "8 chunk CIDs + manifest root");
	let mut sorted = result.cids.clone();
	sorted.sort();
	sorted.dedup();
	assert_eq!(sorted.len(), CHUNKS + 1, "all CIDs distinct");

	// Accounting AFTER — extent must advance by exactly the planned amount.
	let (rem_tx_after, rem_bytes_after) = client
		.query_account_authorization(&alice_acct)
		.await
		.expect("query after")
		.expect("auth");
	assert_eq!(
		rem_tx_before - rem_tx_after,
		expected_tx,
		"extent advanced by exactly the planned tx count"
	);
	assert_eq!(
		rem_bytes_before - rem_bytes_after,
		expected_bytes,
		"extent advanced by exactly the planned bytes"
	);
	println!(
		"streamed {FILE}-byte file: {} stores (chunks + manifest), exactly-once accounting OK",
		result.cids.len()
	);
}

/// Exactly-once accounting across THREE DIFFERENT accounts uploading in
/// parallel. Each account has its own signer (no nonce contention), but every
/// account's `Authorizations.extent` must advance by exactly its own item
/// count — proving the accounting never cross-contaminates between accounts.
/// Mirrors the TS "3 parallel uploads from 3 DIFFERENT accounts" test.
#[tokio::test]
#[ignore]
async fn exactly_once_accounting_three_different_accounts() {
	const ITEMS_PER: usize = 6;
	const ITEM_SIZE: usize = 256 * 1024;

	let authorizer = TransactionClient::new(&ws()).await.expect("connect");
	let eve = dev::eve();
	let uploaders = [dev::alice(), dev::bob(), dev::charlie()];
	let accts: Vec<AccountId32> =
		uploaders.iter().map(|kp| AccountId32::from(kp.public_key().0)).collect();

	// Authorize each account (via Eve, the genesis authorizer) with headroom.
	for acct in &accts {
		authorizer
			.authorize_account(
				acct.clone(),
				(ITEMS_PER + 20) as u32,
				((ITEMS_PER + 20) * ITEM_SIZE) as u64,
				&eve,
				WaitFor::Finalized,
			)
			.await
			.expect("authorize");
	}

	// Snapshot remaining allowance BEFORE, per account.
	let mut before = Vec::new();
	for acct in &accts {
		before.push(
			authorizer
				.query_account_authorization(acct)
				.await
				.expect("query before")
				.expect("authorized"),
		);
	}

	// Unique items per session — cross-session distinct (the chain's TBCH is
	// keyed by content hash regardless of signer, so a collision across
	// accounts would dedup and break the per-account delta).
	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos()
		.to_le_bytes();
	let make_session = |s: usize| -> Vec<UploadItem> {
		(0..ITEMS_PER)
			.map(|i| {
				let mut data = vec![(s * ITEMS_PER + i) as u8; ITEM_SIZE];
				data[..16].copy_from_slice(&seed);
				data[16..24].copy_from_slice(&(s as u64).to_le_bytes());
				data[24..32].copy_from_slice(&(i as u64).to_le_bytes());
				UploadItem::new(data)
			})
			.collect()
	};

	// One client per account — each with its own signer, no nonce sharing.
	let c0 = TransactionClient::new(&ws()).await.expect("c0");
	let c1 = TransactionClient::new(&ws()).await.expect("c1");
	let c2 = TransactionClient::new(&ws()).await.expect("c2");
	let cfg = || UploadConfig { complete_on: WaitFor::Finalized, ..Default::default() };

	let (r0, r1, r2) = tokio::join!(
		upload_items(&c0, &uploaders[0], make_session(0), cfg()),
		upload_items(&c1, &uploaders[1], make_session(1), cfg()),
		upload_items(&c2, &uploaders[2], make_session(2), cfg()),
	);
	let results = [r0.expect("session 0"), r1.expect("session 1"), r2.expect("session 2")];
	for (s, r) in results.iter().enumerate() {
		assert_eq!(r.cids.len(), ITEMS_PER, "session {s} returns ITEMS_PER CIDs");
	}

	// Per-account exact delta: each account did exactly ITEMS_PER stores.
	for (i, acct) in accts.iter().enumerate() {
		let (rem_tx_after, rem_bytes_after) = authorizer
			.query_account_authorization(acct)
			.await
			.expect("query after")
			.expect("authorized");
		let (rem_tx_before, rem_bytes_before) = before[i];
		assert_eq!(rem_tx_before - rem_tx_after, ITEMS_PER as u32, "account {i} tx delta");
		assert_eq!(
			rem_bytes_before - rem_bytes_after,
			(ITEMS_PER * ITEM_SIZE) as u64,
			"account {i} bytes delta"
		);
	}
	println!(
		"3 different accounts: each extent advanced by exactly {ITEMS_PER} tx / {} bytes — no cross-account double-pay",
		ITEMS_PER * ITEM_SIZE
	);
}

/// Per-item event ordering: every item must traverse Started → InBlock →
/// Finalized in that order, with no state ever going backwards. Mirrors the TS
/// "fires events in input order" test.
#[tokio::test]
#[ignore]
async fn events_fire_in_per_item_order() {
	fn code(ev: &UploadEvent) -> (usize, u8) {
		match ev {
			UploadEvent::ItemStarted { index, .. } => (*index, 0),
			UploadEvent::ItemInBlock { index, .. } => (*index, 1),
			UploadEvent::ItemFinalized { index, .. } => (*index, 2),
			UploadEvent::ItemFailed { index, .. } => (*index, 3),
		}
	}

	let client = TransactionClient::new(&ws()).await.expect("connect");
	let alice = dev::alice();

	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos();
	let items: Vec<UploadItem> = (0..4u8)
		.map(|i| {
			let mut data = format!("event-order {seed} #{i} ").into_bytes();
			data.resize(300 + i as usize, i);
			UploadItem::new(data)
		})
		.collect();

	// Record each item's last-seen state and flag any out-of-order transition.
	let last: Arc<Mutex<HashMap<usize, u8>>> = Arc::new(Mutex::new(HashMap::new()));
	let violations: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let (l, v) = (last.clone(), violations.clone());
	let config = UploadConfig {
		complete_on: WaitFor::Finalized,
		on_event: Some(Arc::new(move |ev| {
			let (index, c) = code(&ev);
			let mut last = l.lock().unwrap();
			let prev = last.insert(index, c);
			let ok = match c {
				0 => prev.is_none(),                    // Started is first
				1 => prev == Some(0),                   // InBlock follows Started
				2 => matches!(prev, Some(0) | Some(1)), // Finalized follows Started/InBlock
				_ => false,                             // no failures expected
			};
			if !ok {
				v.lock().unwrap().push(format!("item {index}: state {c} after {prev:?}"));
			}
		})),
		..Default::default()
	};

	let result = upload_items(&client, &alice, items, config).await.expect("upload");
	assert_eq!(result.cids.len(), 4, "4 items");

	let violations = violations.lock().unwrap();
	assert!(violations.is_empty(), "out-of-order events: {violations:?}");
	let last = last.lock().unwrap();
	assert_eq!(last.len(), 4, "every item produced events");
	for (index, &state) in last.iter() {
		assert_eq!(state, 2, "item {index} reached Finalized");
	}
	println!("per-item event ordering OK: 4 items each traversed Started → InBlock → Finalized");
}

/// Unsigned (preimage-authorized) upload. Each item's content hash is authorized
/// as a single-use preimage (via Eve, the genesis authorizer), then the batch is
/// uploaded with NO signer — the pipeline broadcasts bare extrinsics that the
/// chain's `ValidateUnsigned` admits because the content is preimage-authorized.
/// A re-run dedups via TBCH (exactly-once), even though preimage grants are
/// single-use. Mirrors the TS `asUnsigned()` path.
#[tokio::test]
#[ignore]
async fn upload_unsigned_via_preimage_auth() {
	let client = TransactionClient::new(&ws()).await.expect("connect");
	let eve = dev::eve();

	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos();
	let datas: Vec<Vec<u8>> = (0..3u8)
		.map(|i| {
			let mut d = format!("unsigned-live {seed} #{i} ").into_bytes();
			d.resize(300 + i as usize, i);
			d
		})
		.collect();

	// Authorize each content hash as a preimage (Eve is a genesis authorizer).
	for d in &datas {
		let cid =
			calculate_cid_with_config(d, CidCodec::Raw, HashingAlgorithm::Blake2b256).expect("cid");
		client
			.authorize_preimage(cid.content_hash, d.len() as u64, &eve, WaitFor::Finalized)
			.await
			.expect("authorize preimage");
	}

	let items: Vec<UploadItem> = datas.iter().cloned().map(UploadItem::new).collect();
	let started = Arc::new(AtomicUsize::new(0));
	let finalized = Arc::new(AtomicUsize::new(0));
	let (s, fz) = (started.clone(), finalized.clone());
	let config = UploadConfig {
		complete_on: WaitFor::Finalized,
		on_event: Some(Arc::new(move |ev| match ev {
			UploadEvent::ItemStarted { .. } => {
				s.fetch_add(1, Ordering::SeqCst);
			},
			UploadEvent::ItemFinalized { .. } => {
				fz.fetch_add(1, Ordering::SeqCst);
			},
			_ => {},
		})),
		..Default::default()
	};

	// No signer — the unsigned (preimage-authorized) path.
	let result = upload_items_unsigned(&client, items, config).await.expect("unsigned upload");
	assert_eq!(result.cids.len(), 3, "one CID per item");
	assert_eq!(started.load(Ordering::SeqCst), 3, "ItemStarted per item");
	assert_eq!(finalized.load(Ordering::SeqCst), 3, "all items finalized via the unsigned path");

	// Re-run: preimage grants are single-use (consumed), but the content is now
	// stored, so the TBCH dedup pre-check skips submission and returns the same
	// CIDs — exactly-once, no re-authorization needed.
	let items2: Vec<UploadItem> = datas.iter().cloned().map(UploadItem::new).collect();
	let result2 = upload_items_unsigned(&client, items2, UploadConfig::default())
		.await
		.expect("unsigned re-upload");
	assert_eq!(result2.cids, result.cids, "re-run returns identical CIDs (exactly-once dedup)");
	println!("unsigned upload OK: 3 items stored via preimage auth, exactly-once re-run dedup");
}

/// Multi-provider broadcast fan-out. The client connects to TWO collators of the
/// same chain and fans every signed extrinsic out to both. The risk this guards
/// is double-storing: the same tx reaching two nodes must still store exactly
/// once. Asserts the upload completes and the on-chain `Authorizations.extent`
/// advances by exactly N — fan-out is redundant, not duplicative. Mirrors the TS
/// strategy's `submitClients` fan-out.
#[tokio::test]
#[ignore]
async fn multi_provider_fanout_exactly_once() {
	const N: usize = 5;
	const ITEM_SIZE: usize = 128 * 1024;

	// Two endpoints of the SAME chain — every tx broadcasts to both collators.
	let (a, b) = (ws(), ws2());
	let client = TransactionClient::from_endpoints(&[&a, &b]).await.expect("connect multi");
	let alice = dev::alice();
	let eve = dev::eve();
	let alice_acct = AccountId32::from(alice.public_key().0);

	client
		.authorize_account(
			alice_acct.clone(),
			(N + 20) as u32,
			((N + 20) * ITEM_SIZE) as u64,
			&eve,
			WaitFor::Finalized,
		)
		.await
		.expect("authorize alice");

	let (rem_tx_before, rem_bytes_before) = client
		.query_account_authorization(&alice_acct)
		.await
		.expect("query before")
		.expect("auth");

	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos()
		.to_le_bytes();
	let items: Vec<UploadItem> = (0..N)
		.map(|i| {
			let mut data = vec![i as u8; ITEM_SIZE];
			data[..16].copy_from_slice(&seed);
			data[16..24].copy_from_slice(&(i as u64).to_le_bytes());
			UploadItem::new(data)
		})
		.collect();

	let result = upload_items(
		&client,
		&alice,
		items,
		UploadConfig { complete_on: WaitFor::Finalized, ..Default::default() },
	)
	.await
	.expect("multi-provider upload");
	assert_eq!(result.cids.len(), N, "one CID per item");
	let mut sorted = result.cids.clone();
	sorted.sort();
	sorted.dedup();
	assert_eq!(sorted.len(), N, "all CIDs distinct");

	let (rem_tx_after, rem_bytes_after) = client
		.query_account_authorization(&alice_acct)
		.await
		.expect("query after")
		.expect("auth");
	assert_eq!(
		rem_tx_before - rem_tx_after,
		N as u32,
		"fan-out to 2 endpoints stored exactly N tx — no double-store"
	);
	assert_eq!(
		rem_bytes_before - rem_bytes_after,
		(N * ITEM_SIZE) as u64,
		"extent advanced by exactly N*size bytes"
	);
	println!("multi-provider fan-out OK: {N} items broadcast to 2 endpoints, stored exactly once");
}

/// Batched account authorization (#5): authorize two accounts in ONE
/// `Utility.batch_all` transaction (atomic). Verifies the batch executes and
/// both accounts end up authorized, sharing the same inclusion block.
#[tokio::test]
#[ignore]
async fn authorize_accounts_batched() {
	let client = TransactionClient::new(&ws()).await.expect("connect");
	let eve = dev::eve();
	let bob = AccountId32::from(dev::bob().public_key().0);
	let charlie = AccountId32::from(dev::charlie().public_key().0);

	let entries = vec![
		AuthorizeAccountEntry { who: bob.clone(), transactions: 7, bytes: 7 * 1024 },
		AuthorizeAccountEntry { who: charlie.clone(), transactions: 9, bytes: 9 * 1024 },
	];
	let receipts = client
		.authorize_accounts(entries, false, &eve, WaitFor::Finalized)
		.await
		.expect("batched authorize");

	assert_eq!(receipts.len(), 2, "one receipt per entry");
	assert_eq!(
		receipts[0].block_hash, receipts[1].block_hash,
		"single batched tx → both in one block"
	);
	assert!(
		client.query_account_authorization(&bob).await.expect("query bob").is_some(),
		"bob authorized"
	);
	assert!(
		client
			.query_account_authorization(&charlie)
			.await
			.expect("query charlie")
			.is_some(),
		"charlie authorized"
	);
	println!(
		"batched authorize OK: 2 accounts authorized atomically in block {}",
		receipts[0].block_hash
	);
}

/// Estimate dedup/skip-existing + duplicate-content submit. Builds `[a, b, a]`
/// (item 2 duplicates item 0):
/// - `estimate_upload` (dedup_input default) marks index 2 as DuplicateInput, excludes it from
///   `transactions`/`to_upload`.
/// - `submit` of that plan succeeds: the duplicate is skipped (stored exactly once), not rejected.
/// - a stored item re-estimated with `skip_existing` shows AlreadyOnChain.
#[tokio::test]
#[ignore]
async fn estimate_dedup_skip_existing_and_guard() {
	let client = TransactionClient::new(&ws()).await.expect("connect");
	let alice = dev::alice();
	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos();

	let a = format!("dedup-a {seed}").into_bytes();
	let b = format!("dedup-b {seed}").into_bytes();
	let items =
		vec![UploadItem::new(a.clone()), UploadItem::new(b.clone()), UploadItem::new(a.clone())];

	// #2/#3: within-input duplicate collapsed in the estimate.
	let est = client
		.estimate_upload(UploadInput::Items(items.clone()), UploadEstimateOptions::default())
		.await
		.expect("estimate");
	assert_eq!(est.base.total, 3, "three units planned");
	assert_eq!(est.base.duplicate_indices, vec![2], "item 2 duplicates item 0");
	assert_eq!(est.base.transactions, 2, "duplicate not charged");
	assert_eq!(est.base.to_upload, vec![0, 1], "only the unique items submit");
	assert!(matches!(est.base.items[2].skip_reason, Some(SkipReason::DuplicateInput)));
	assert!(est.base.items[0].skip_reason.is_none());

	// A within-input duplicate is SKIPPED, not rejected — submit stores the
	// unique content exactly once and returns every cid (the dup included).
	let datas: Vec<Vec<u8>> = items.iter().map(|i| i.data.clone()).collect();
	let src: Arc<dyn SeekableSource> = Arc::new(blob_from_items(datas));
	let res = client
		.submit(
			&alice,
			est,
			src,
			UploadConfig { complete_on: WaitFor::Finalized, ..Default::default() },
		)
		.await
		.expect("within-input duplicate is skipped, not rejected");
	assert_eq!(res.cids.len(), 3, "all three cids returned (the dup at index 2 included)");
	// Both unique contents are on chain; the duplicate added nothing to store.
	let verify = client
		.estimate_upload(
			UploadInput::Items(vec![UploadItem::new(a.clone()), UploadItem::new(b.clone())]),
			UploadEstimateOptions { skip_existing: true, ..Default::default() },
		)
		.await
		.expect("verify estimate");
	assert_eq!(verify.base.already_stored, vec![0, 1], "both unique contents stored exactly once");

	// #3: store a unique item, then re-estimate it with skip_existing.
	let uniq = format!("dedup-uniq {seed}").into_bytes();
	let one = vec![UploadItem::new(uniq.clone())];
	let est1 = client
		.estimate_upload(UploadInput::Items(one.clone()), UploadEstimateOptions::default())
		.await
		.expect("estimate one");
	let src1: Arc<dyn SeekableSource> = Arc::new(blob_from_items(vec![uniq.clone()]));
	client
		.submit(
			&alice,
			est1,
			src1,
			UploadConfig { complete_on: WaitFor::Finalized, ..Default::default() },
		)
		.await
		.expect("store one");

	let est2 = client
		.estimate_upload(
			UploadInput::Items(one),
			UploadEstimateOptions { skip_existing: true, ..Default::default() },
		)
		.await
		.expect("estimate skip_existing");
	assert_eq!(est2.base.already_stored, vec![0], "skip_existing finds it on chain");
	assert_eq!(est2.base.transactions, 0, "nothing left to upload");
	assert!(matches!(est2.base.items[0].skip_reason, Some(SkipReason::AlreadyOnChain)));
	println!("estimate dedup + skip_existing + submit duplicate-guard OK");
}

/// Renew round-trip through the compat registry on the local chain: the
/// registry must resolve the workspace runtime's `renew(entry: TransactionRef)`
/// shape, and a stored item's `(block, index)` slot must renew successfully.
#[tokio::test]
#[ignore]
async fn renew_roundtrip_via_registry() {
	use bulletin_sdk_rust::compat::{renew_adapter, RenewAdapter};

	let client = TransactionClient::new(&ws()).await.expect("connect");
	let alice = dev::alice();

	assert_eq!(
		renew_adapter(&client.api().metadata()).expect("shape registered"),
		RenewAdapter::TransactionRef,
		"local chain speaks the current renew shape"
	);

	// Store one unique item and capture its renew slot from ItemFinalized.
	let nonce_seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_nanos();
	let items = vec![UploadItem::new(format!("renew-live {nonce_seed}").into_bytes())];
	let slot = Arc::new(Mutex::new(None::<(u32, u32)>));
	let sink = slot.clone();
	let config = UploadConfig {
		complete_on: WaitFor::Finalized,
		on_event: Some(Arc::new(move |ev| {
			if let UploadEvent::ItemFinalized {
				block_number: Some(block),
				transaction_index: Some(index),
				..
			} = ev
			{
				*sink.lock().unwrap() = Some((block, index));
			}
		})),
		..Default::default()
	};
	upload_items(&client, &alice, items, config).await.expect("upload");
	let (block, index) = slot.lock().unwrap().expect("ItemFinalized carried the renew slot");

	let receipt = client
		.renew((block, index), &alice, WaitFor::Finalized)
		.await
		.expect("renew via registry dispatch");
	assert!(!receipt.block_hash.is_empty(), "renew included");
	println!("renew OK: slot ({block}, {index}) renewed in block {}", receipt.block_hash);
}

/// Read-only fleet probe: a live legacy chain resolves to the positional
/// adapter. Network-dependent, so opt-in via env even within the ignored
/// suite (CI runs `--ignored` hermetically against local zombienet):
///
/// ```text
/// BULLETIN_LEGACY_RPC_URL=wss://westend-bulletin-rpc.polkadot.io \
///     cargo test -p bulletin-sdk-rust --test pipeline_live -- --ignored renew_registry_resolves_live_legacy_chain
/// ```
#[tokio::test]
#[ignore]
async fn renew_registry_resolves_live_legacy_chain() {
	use bulletin_sdk_rust::compat::{renew_adapter, RenewAdapter};

	let Ok(url) = std::env::var("BULLETIN_LEGACY_RPC_URL") else {
		println!("skipped: set BULLETIN_LEGACY_RPC_URL to a legacy-shape chain RPC");
		return;
	};
	let client = TransactionClient::new(&url).await.expect("connect");
	assert_eq!(
		renew_adapter(&client.api().metadata()).expect("legacy shape registered"),
		RenewAdapter::Positional,
		"legacy chain resolves to the positional adapter"
	);
	println!("live legacy chain verified: registry resolves positional renew");
}
