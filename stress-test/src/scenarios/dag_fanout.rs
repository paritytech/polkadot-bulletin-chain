use anyhow::{anyhow, Context as _, Result};
use futures::stream::{self, StreamExt, TryStreamExt};
use std::{
	collections::HashMap,
	sync::Arc,
	time::{Duration, Instant},
};
use subxt::{
	config::transaction_extensions::Params,
	dynamic::Value,
	ext::scale_value::Composite,
	utils::{AccountId32, H256},
	OnlineClient,
};
use subxt_signer::sr25519::Keypair;

use crate::{
	accounts::NonceTracker,
	cid_hash::Hashing,
	client::{BulletinConfig, BulletinExtrinsicParamsBuilder},
	dag_pb,
	http_gateway::HttpGatewayClient,
	metrics::metrics,
	report::ScenarioResult,
	store::{self, PreSignedTx},
};

const VARIANT: &str = "dag-fanout";
const HASHING: Hashing = Hashing::Sha2_256;
const STORE_MORTALITY_BLOCKS: u64 = 64;
const SIGN_PARALLELISM: usize = 16;
const CONFIRM_PARALLELISM: usize = 32;
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(300);
const CONFIRM_POLL_INTERVAL: Duration = Duration::from_secs(6);
const RESEND_AFTER: Duration = Duration::from_secs(45);
const CAR_FETCH_TIMEOUT: Duration = Duration::from_secs(420);

struct Item {
	cid: cid::Cid,
	data: Vec<u8>,
	signer: Keypair,
}

impl Item {
	fn new(cid: cid::Cid, data: Vec<u8>) -> Self {
		Self { cid, data, signer: Keypair::from_secret_key(rand::random()).expect("valid keypair") }
	}

	fn account_id(&self) -> AccountId32 {
		self.signer.public_key().to_account_id()
	}

	fn store_call(&self) -> subxt::tx::DefaultPayload<Composite<()>> {
		let cid_config = Value::named_composite(vec![
			("codec".to_string(), Value::u128(self.cid.codec() as u128)),
			(
				"hashing".to_string(),
				Value::variant(HASHING.runtime_variant_name(), Composite::Unnamed(vec![])),
			),
		]);
		subxt::dynamic::tx(
			"TransactionStorage",
			"store_with_cid_config",
			vec![cid_config, Value::from_bytes(&self.data)],
		)
	}
}

async fn is_stored_at(client: &OnlineClient<BulletinConfig>, block: H256, cid: &cid::Cid) -> bool {
	let addr = subxt::dynamic::storage(
		"TransactionStorage",
		"TransactionByContentHash",
		vec![Value::from_bytes(cid.hash().digest())],
	);
	matches!(client.storage().at(block).fetch(&addr).await, Ok(Some(_)))
}

fn build_items(n_leaves: usize, leaf_size: usize) -> Vec<Item> {
	let mut items: Vec<Item> = (0..n_leaves)
		.map(|leaf_index| {
			let data = store::generate_indexed_payload(leaf_size, leaf_index as u32);
			Item::new(dag_pb::raw_leaf_cid(&data), data)
		})
		.collect();
	let leaf_links: Vec<(cid::Cid, usize)> =
		items.iter().map(|item| (item.cid, item.data.len())).collect();
	let (root_bytes, root_cid) = dag_pb::build_unixfs_file_root(&leaf_links);
	items.push(Item::new(root_cid, root_bytes));
	items
}

async fn sign_items(
	client: &OnlineClient<BulletinConfig>,
	items: &Arc<Vec<Item>>,
) -> Result<Vec<PreSignedTx>> {
	let best = client.blocks().at_latest().await?;
	let anchor_number = best.number() as u64;
	let anchor_hash = best.hash();

	stream::iter(0..items.len())
		.map(|idx| {
			let client = client.clone();
			let items = Arc::clone(items);
			async move {
				tokio::task::spawn_blocking(move || {
					let item = &items[idx];
					let mut params = BulletinExtrinsicParamsBuilder::new()
						.nonce(0)
						.mortal(STORE_MORTALITY_BLOCKS)
						.build();
					params.inject_block(anchor_number, anchor_hash);
					let mut partial = client
						.tx()
						.create_partial_offline(&item.store_call(), params)
						.map_err(|error| anyhow!("sign item {idx}: {error}"))?;
					let signed = partial.sign(&item.signer);
					Ok::<_, anyhow::Error>(PreSignedTx {
						nonce: 0,
						encoded: signed.into_encoded(),
						tx_hash: H256::zero(),
						payload_size: item.data.len(),
					})
				})
				.await
				.map_err(|error| anyhow!("sign item {idx}: join: {error}"))?
			}
		})
		.buffered(SIGN_PARALLELISM)
		.try_collect()
		.await
}

async fn unconfirmed_items(
	client: &OnlineClient<BulletinConfig>,
	items: &[Item],
	candidates: &[usize],
) -> Result<Vec<usize>> {
	let finalized = client.backend().latest_finalized_block_ref().await?.hash();
	let mut still_unconfirmed: Vec<usize> = stream::iter(candidates.iter().copied())
		.map(|idx| async move { (idx, is_stored_at(client, finalized, &items[idx].cid).await) })
		.buffer_unordered(CONFIRM_PARALLELISM)
		.filter_map(|(idx, stored)| async move { (!stored).then_some(idx) })
		.collect()
		.await;
	still_unconfirmed.sort_unstable();
	Ok(still_unconfirmed)
}

async fn upload_until_finalized(
	client: &OnlineClient<BulletinConfig>,
	ws_urls: &[String],
	items: &[Item],
	txs: &[PreSignedTx],
) -> Result<Duration> {
	let upload_start = Instant::now();
	let (ok, errors) = store::submit_sequential_wave(ws_urls, txs).await;
	tracing::info!("Submitted {} store txs: {ok} ok, {} errors", txs.len(), errors.len());

	let total = items.len();
	let deadline = Instant::now() + CONFIRM_TIMEOUT;
	let mut last_submit = Instant::now();
	let mut unconfirmed: Vec<usize> = (0..total).collect();
	loop {
		tokio::time::sleep(CONFIRM_POLL_INTERVAL).await;
		unconfirmed = unconfirmed_items(client, items, &unconfirmed).await?;
		tracing::info!("Confirmed {}/{total} at finalized block", total - unconfirmed.len());
		if unconfirmed.is_empty() {
			return Ok(upload_start.elapsed());
		}
		if Instant::now() > deadline {
			anyhow::bail!(
				"timed out: only {}/{total} items confirmed at finalized block",
				total - unconfirmed.len()
			);
		}
		if last_submit.elapsed() > RESEND_AFTER {
			let resend: Vec<PreSignedTx> =
				unconfirmed.iter().map(|&idx| txs[idx].clone()).collect();
			tracing::info!("Resubmitting {} unconfirmed store txs", resend.len());
			let _ = store::submit_sequential_wave(ws_urls, &resend).await;
			last_submit = Instant::now();
		}
	}
}

#[allow(clippy::too_many_arguments)]
pub async fn run_dag_fanout(
	client: &OnlineClient<BulletinConfig>,
	authorizer: &Keypair,
	nonce_tracker: &NonceTracker,
	ws_urls: &[String],
	gateway_url: &str,
	leaf_counts: &[usize],
	leaf_size: usize,
	results: &mut Vec<ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<ScenarioResult>),
) -> Result<()> {
	tracing::info!("dag-fanout: authorizer = {}", authorizer.public_key().to_account_id());

	for &n_leaves in leaf_counts {
		let result =
			run_one(client, nonce_tracker, ws_urls, authorizer, gateway_url, n_leaves, leaf_size)
				.await
				.with_context(|| format!("dag-fanout with {n_leaves} leaves"))?;
		results.push(result);
		on_result(results);
	}
	Ok(())
}

async fn run_one(
	client: &OnlineClient<BulletinConfig>,
	nonce_tracker: &NonceTracker,
	ws_urls: &[String],
	authorizer: &Keypair,
	gateway_url: &str,
	n_leaves: usize,
	leaf_size: usize,
) -> Result<ScenarioResult> {
	tracing::info!("=== dag-fanout: {n_leaves} leaves of {leaf_size} B ===");

	let items = Arc::new(build_items(n_leaves, leaf_size));
	let root = items.last().expect("build_items appends the root");
	let root_cid = root.cid;
	tracing::info!(
		"Built UnixFS file root {root_cid} over {n_leaves} leaves ({} B)",
		root.data.len()
	);

	let account_ids: Vec<AccountId32> = items.iter().map(Item::account_id).collect();
	let bytes_allow = leaf_size.max(root.data.len()) as u64;
	tracing::info!("Authorizing {} accounts...", account_ids.len());
	crate::authorize::authorize_accounts(
		client,
		authorizer,
		nonce_tracker,
		&account_ids,
		1,
		bytes_allow,
	)
	.await?;

	let txs = sign_items(client, &items).await?;
	let upload_elapsed = upload_until_finalized(client, ws_urls, &items, &txs).await?;
	tracing::info!(
		"All {} items finalized in {:.1}s, root {root_cid}",
		items.len(),
		upload_elapsed.as_secs_f64()
	);

	let gateway = HttpGatewayClient::new(gateway_url).await?;
	tracing::info!("Fetching root via gateway CAR (dag-scope=all): {gateway_url}");
	let fetch_start = Instant::now();
	let pairs = gateway
		.fetch_dag(&root_cid, CAR_FETCH_TIMEOUT)
		.await
		.map_err(|error| anyhow!("gateway CAR fetch of {root_cid} failed: {error}"))?;
	let fetch_elapsed = fetch_start.elapsed();
	let car_bytes: usize = pairs.iter().map(|(_, data)| data.len()).sum();

	let returned: HashMap<cid::Cid, &[u8]> =
		pairs.iter().map(|(cid, data)| (*cid, data.as_slice())).collect();
	let mut content_ok = 0usize;
	for item in items.iter() {
		match returned.get(&item.cid) {
			Some(got) if *got == item.data.as_slice() => content_ok += 1,
			Some(_) =>
				tracing::warn!("content MISMATCH for {} (bytes differ from uploaded)", item.cid),
			None => tracing::warn!("MISSING block {} in gateway response", item.cid),
		}
	}
	let content_bad = items.len() - content_ok;
	let all_ok = content_bad == 0;
	metrics().inc_reads(VARIANT, all_ok, pairs.len() as u64, car_bytes as u64);
	tracing::info!(
		"Gateway returned {} blocks ({car_bytes} B) in {:.1}s, verified {content_ok}/{} by \
		 CID+content {} (fan-out {n_leaves} leaves + root)",
		pairs.len(),
		fetch_elapsed.as_secs_f64(),
		items.len(),
		if all_ok { "OK" } else { "FAILED" },
	);

	Ok(ScenarioResult {
		name: format!(
			"dag-pb fan-out ({n_leaves} leaves, {} blocks via CAR, content {}, root {root_cid})",
			pairs.len(),
			if all_ok { "verified" } else { "FAILED" },
		),
		variant: VARIANT.to_string(),
		duration: fetch_elapsed,
		payload_size: car_bytes,
		total_reads: Some(pairs.len() as u64),
		successful_reads: Some(content_ok as u64),
		failed_reads: Some(content_bad as u64),
		data_verified: Some(all_ok),
		..Default::default()
	})
}
