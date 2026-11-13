// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Transaction priority on a live collator: signers share full blocks by queue position, and
//! higher priority levels come first within a block.

use crate::{
	test_log,
	utils::{
		blake2_256, build_parachain_network_config_three_relay_validators, generate_test_data,
		initialize_network, verify_parachain_binaries, wait_for_block_height,
		wait_for_finalized_height, wait_for_session_change_on_node, NETWORK_READY_TIMEOUT_SECS,
	},
};
use anyhow::{anyhow, Context, Result};
use pallet_bulletin_transaction_storage::DEFAULT_MAX_BLOCK_TRANSACTIONS;
use std::{collections::BTreeMap, time::Duration};
use subxt::{
	config::substrate::{SubstrateConfig, SubstrateExtrinsicParamsBuilder},
	dynamic::{tx, Value},
	ext::{
		scale_value::value,
		subxt_rpcs::{LegacyRpcMethods, RpcClient},
	},
	tx::Payload,
	OnlineClient,
};
use subxt_signer::{
	sr25519::{dev, Keypair},
	DeriveJunction,
};

const SESSION_CHANGE_TIMEOUT_SECS: u64 = 300;
const BLOCK_WAIT_TIMEOUT_SECS: u64 = 600;
const FINALITY_TIMEOUT_SECS: u64 = 600;
const SETUP_RESUBMIT_LIMIT: usize = 20;
const SUBMIT_RETRY_LIMIT: usize = 5;
const SUBMIT_CONCURRENCY: usize = 64;
const RETRY_DELAY: Duration = Duration::from_secs(3);
const STORE_BYTES: usize = 4 * 1024;
const SIGNER_COUNT: usize = 4;
const STORES_PER_SIGNER: usize = 400;
const FULL_BLOCKS_REQUIRED: usize = 2;
const BACKLOG_PER_SIGNER: usize = 300;
const LARGE_ALLOWANCE_BYTES: u64 = 1024 * 1024 * 1024;
const OVER_BUDGET_ALLOWANCE_BYTES: u64 = 1;
const ALLOWANCE_TRANSACTIONS: u32 = 10_000;
const TRANSFER_VALUE: u128 = 1_000_000_000_000;
const MAX_BLOCK_STORES: usize = DEFAULT_MAX_BLOCK_TRANSACTIONS as usize;

fn para_node_args() -> Vec<String> {
	vec![
		"--pool-limit=20000".into(),
		"--pool-kbytes=131072".into(),
		"-lruntime::transaction-storage=info,txpool=info".into(),
		"--".into(),
		"--network-backend=libp2p".into(),
	]
}

struct SignerKeys {
	keypair: Keypair,
	label: &'static str,
}

fn signers() -> Vec<SignerKeys> {
	vec![
		SignerKeys { keypair: dev::bob(), label: "bob" },
		SignerKeys { keypair: dev::charlie(), label: "charlie" },
		SignerKeys { keypair: dev::dave(), label: "dave" },
		SignerKeys { keypair: dev::ferdie(), label: "ferdie" },
	]
}

fn store_payload(
	data: Vec<u8>,
) -> subxt::tx::DefaultPayload<subxt::ext::scale_value::Composite<()>> {
	tx("TransactionStorage", "store", vec![Value::from_bytes(data)])
}

fn transfer_payload(
	to: [u8; 32],
) -> subxt::tx::DefaultPayload<subxt::ext::scale_value::Composite<()>> {
	tx(
		"Balances",
		"transfer_keep_alive",
		vec![value!(Id(Value::from_bytes(to))), Value::u128(TRANSFER_VALUE)],
	)
}

fn signed_bytes(
	client: &OnlineClient<SubstrateConfig>,
	signer: &Keypair,
	call: &impl Payload,
	nonce: u64,
	tip: u128,
) -> Result<Vec<u8>> {
	let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).tip(tip).build();
	let mut partial = client.tx().create_partial_offline(call, params)?;
	Ok(partial.sign(signer).into_encoded())
}

fn bare_bytes(client: &OnlineClient<SubstrateConfig>, call: &impl Payload) -> Result<Vec<u8>> {
	Ok(client.tx().create_unsigned(call)?.into_encoded())
}

/// Submits pre-signed extrinsics with `author_submitExtrinsic` (no subscription) over a
/// dedicated connection. A watcher subscription per transaction hits the node's 1024
/// subscriptions-per-connection limit and leaves the shared client unusable.
struct RawSubmitter {
	methods: LegacyRpcMethods<SubstrateConfig>,
}

impl RawSubmitter {
	async fn connect(ws_url: &str) -> Result<Self> {
		let rpc = RpcClient::from_insecure_url(ws_url).await?;
		Ok(Self { methods: LegacyRpcMethods::new(rpc) })
	}

	async fn submit_all(&self, batch: Vec<Vec<u8>>) -> Result<()> {
		use futures::StreamExt;
		let mut pending = batch;
		for attempt in 0..SUBMIT_RETRY_LIMIT {
			let outcomes: Vec<(Vec<u8>, Result<()>)> = futures::stream::iter(pending)
				.map(|bytes| async move {
					let outcome = self
						.methods
						.author_submit_extrinsic(&bytes)
						.await
						.map(|_| ())
						.map_err(|error| anyhow!("author_submitExtrinsic: {error}"));
					(bytes, outcome)
				})
				.buffer_unordered(SUBMIT_CONCURRENCY)
				.collect()
				.await;
			let mut failed = Vec::new();
			let mut first_error = None;
			for (bytes, outcome) in outcomes {
				if let Err(error) = outcome {
					if is_banned(&error) {
						anyhow::bail!("a submission is banned by the pool, resubmitting cannot succeed: {error}");
					}
					if !is_already_known(&error) {
						first_error.get_or_insert(error);
						failed.push(bytes);
					}
				}
			}
			if failed.is_empty() {
				return Ok(());
			}
			tracing::warn!(
				"attempt {}: {} submissions failed, first: {}",
				attempt + 1,
				failed.len(),
				first_error.as_ref().map(|error| error.to_string()).unwrap_or_default()
			);
			pending = failed;
			tokio::time::sleep(RETRY_DELAY).await;
		}
		anyhow::bail!("submissions still failing after {SUBMIT_RETRY_LIMIT} attempts")
	}
}

fn is_already_known(error: &anyhow::Error) -> bool {
	let text = error.to_string();
	text.contains("Already Imported") || text.contains("already imported") || text.contains("1013")
}

fn is_banned(error: &anyhow::Error) -> bool {
	let text = error.to_string();
	text.contains("temporarily banned") || text.contains("1012")
}

async fn account_nonce(client: &OnlineClient<SubstrateConfig>, signer: &Keypair) -> Result<u64> {
	Ok(client.tx().account_nonce(&signer.public_key().to_account_id()).await?)
}

async fn best_block_number(client: &OnlineClient<SubstrateConfig>) -> Result<u64> {
	let mut subscription = client.blocks().subscribe_best().await?;
	let block = subscription.next().await.ok_or_else(|| anyhow!("best block stream ended"))??;
	Ok(block.number() as u64)
}

/// Submits a setup transaction and waits for finality. A transaction included on a losing
/// relay fork ends its watcher with `Invalid` or `Dropped`; it is then signed again with the
/// current nonce and resubmitted.
async fn submit_and_wait_finalized(
	client: &OnlineClient<SubstrateConfig>,
	signer: &Keypair,
	call: &impl Payload,
) -> Result<()> {
	for attempt in 0..SETUP_RESUBMIT_LIMIT {
		let nonce = account_nonce(client, signer).await?;
		let params = SubstrateExtrinsicParamsBuilder::new().nonce(nonce).immortal().build();
		let outcome = tokio::time::timeout(Duration::from_secs(FINALITY_TIMEOUT_SECS), async {
			let progress = client.tx().sign_and_submit_then_watch(call, signer, params).await?;
			crate::utils::wait_for_finalized(progress).await?;
			Ok::<_, anyhow::Error>(())
		})
		.await
		.map_err(|_| anyhow!("transaction timed out"))?;
		match outcome {
			Ok(()) => return Ok(()),
			Err(error) if is_already_known(&error) => return Ok(()),
			Err(error) => {
				tracing::warn!("setup transaction attempt {} failed: {error}", attempt + 1);
				tokio::time::sleep(RETRY_DELAY).await;
			},
		}
	}
	anyhow::bail!("setup transaction failed {SETUP_RESUBMIT_LIMIT} times")
}

/// Funds `who` from Alice so it can pay fees. `One`/`Two` are not endowed at genesis.
async fn fund_from_alice(client: &OnlineClient<SubstrateConfig>, who: [u8; 32]) -> Result<()> {
	submit_and_wait_finalized(client, &dev::alice(), &transfer_payload(who)).await
}

async fn authorize_from_eve(
	client: &OnlineClient<SubstrateConfig>,
	who: [u8; 32],
	bytes: u64,
) -> Result<()> {
	let call = tx(
		"TransactionStorage",
		"authorize_account",
		vec![
			Value::from_bytes(who),
			Value::u128(ALLOWANCE_TRANSACTIONS as u128),
			Value::u128(bytes as u128),
		],
	);
	submit_and_wait_finalized(client, &dev::eve(), &call).await
}

async fn authorize_preimage_from_eve(
	client: &OnlineClient<SubstrateConfig>,
	content_hash: [u8; 32],
	max_size: u64,
) -> Result<()> {
	let call = tx(
		"TransactionStorage",
		"authorize_preimage",
		vec![Value::from_bytes(content_hash), Value::u128(max_size as u128)],
	);
	submit_and_wait_finalized(client, &dev::eve(), &call).await
}

fn presigned_stores(
	client: &OnlineClient<SubstrateConfig>,
	signer: &SignerKeys,
	start_nonce: u64,
	count: usize,
	salt: &str,
) -> Result<Vec<Vec<u8>>> {
	(0..count)
		.map(|index| {
			let pattern = format!("{salt}_{}_{}_", signer.label, start_nonce + index as u64);
			let data = generate_test_data(STORE_BYTES, pattern.as_bytes());
			signed_bytes(
				client,
				&signer.keypair,
				&store_payload(data),
				start_nonce + index as u64,
				0,
			)
		})
		.collect()
}

/// Signs `per_signer` stores per signer from each signer's current on-chain nonce and submits
/// them all at once.
async fn submit_backlog(
	client: &OnlineClient<SubstrateConfig>,
	submitter: &RawSubmitter,
	signers: &[SignerKeys],
	per_signer: usize,
	salt: &str,
) -> Result<()> {
	let mut batch = Vec::with_capacity(signers.len() * per_signer);
	for signer in signers {
		let nonce = account_nonce(client, &signer.keypair).await?;
		batch.extend(presigned_stores(client, signer, nonce, per_signer, salt)?);
	}
	submitter.submit_all(batch).await
}

/// Fisher-Yates with an xorshift generator, so no extra dependency is needed.
fn shuffle<T>(items: &mut [T], seed: u64) {
	let mut state = seed.max(1);
	for index in (1..items.len()).rev() {
		state ^= state << 13;
		state ^= state >> 7;
		state ^= state << 17;
		let other = (state % (index as u64 + 1)) as usize;
		items.swap(index, other);
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Category {
	Store,
	Renew,
	Authorize,
	Cleanup,
	Transfer,
	Other(String),
}

#[derive(Debug, Clone)]
struct ClassifiedExtrinsic {
	index: u32,
	signer: Option<[u8; 32]>,
	category: Category,
}

async fn classified_block(
	client: &OnlineClient<SubstrateConfig>,
	number: u64,
) -> Result<Vec<ClassifiedExtrinsic>> {
	let hash = crate::utils::finalized_block_hash_at(client, number).await?;
	let block = client.blocks().at(hash).await?;
	let extrinsics = block.extrinsics().await?;
	let mut classified = Vec::new();
	for extrinsic in extrinsics.iter() {
		let pallet = extrinsic.pallet_name()?.to_string();
		let variant = extrinsic.variant_name()?.to_string();
		let category = match (pallet.as_str(), variant.as_str()) {
			("TransactionStorage", "store") | ("TransactionStorage", "store_with_cid_config") =>
				Category::Store,
			("DataRenewal", "renew") |
			("DataRenewal", "force_renew") |
			("DataRenewal", "enable_auto_renew") |
			("DataRenewal", "disable_auto_renew") => Category::Renew,
			("TransactionStorage", "authorize_account") |
			("TransactionStorage", "authorize_preimage") |
			("TransactionStorage", "refresh_account_authorization") |
			("TransactionStorage", "refresh_preimage_authorization") => Category::Authorize,
			("TransactionStorage", "remove_expired_account_authorization") |
			("TransactionStorage", "remove_expired_preimage_authorization") |
			("TransactionStorage", "remove_exhausted_authorizer") => Category::Cleanup,
			("Balances", _) => Category::Transfer,
			_ => Category::Other(format!("{pallet}::{variant}")),
		};
		let signer = extrinsic.address_bytes().and_then(|address| {
			let account: [u8; 32] = address.get(1..33)?.try_into().ok()?;
			Some(account)
		});
		classified.push(ClassifiedExtrinsic { index: extrinsic.index(), signer, category });
	}
	Ok(classified)
}

fn store_counts(block: &[ClassifiedExtrinsic]) -> BTreeMap<Option<[u8; 32]>, usize> {
	let mut counts = BTreeMap::new();
	for extrinsic in block.iter().filter(|extrinsic| extrinsic.category == Category::Store) {
		*counts.entry(extrinsic.signer).or_insert(0) += 1;
	}
	counts
}

async fn network_and_client() -> Result<(
	zombienet_sdk::Network<zombienet_sdk::LocalFileSystem>,
	OnlineClient<SubstrateConfig>,
	RawSubmitter,
)> {
	verify_parachain_binaries()?;
	let config = build_parachain_network_config_three_relay_validators(para_node_args())?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;
	let relay_alice = network.get_node("alice").context("get relay alice")?;
	wait_for_session_change_on_node(relay_alice, SESSION_CHANGE_TIMEOUT_SECS).await?;
	let collator = network.get_node("collator-1").context("get collator-1")?;
	wait_for_finalized_height(collator, 1, FINALITY_TIMEOUT_SECS).await?;
	let client: OnlineClient<SubstrateConfig> = collator.wait_client().await?;
	let submitter = RawSubmitter::connect(collator.ws_uri()).await?;
	Ok((network, client, submitter))
}

/// Four signers each queue a backlog of stores at once. Every full block gives each signer the
/// same share, within one transaction, because the queue bonus alternates between signers.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_priority_four_signers_share_full_blocks_test() -> Result<()> {
	const TEST: &str = "para_priority_four_signers";
	crate::utils::init_logging();
	test_log!(
		TEST,
		"=== {SIGNER_COUNT} signers x {STORES_PER_SIGNER} stores of {STORE_BYTES} bytes ==="
	);

	let (network, client, submitter) = network_and_client().await?;
	let collator = network.get_node("collator-1")?;
	let signers = signers();

	for signer in &signers {
		authorize_from_eve(&client, signer.keypair.public_key().0, LARGE_ALLOWANCE_BYTES).await?;
	}
	test_log!(TEST, "submitting {} pre-signed stores at once", SIGNER_COUNT * STORES_PER_SIGNER);
	let submit_block = best_block_number(&client).await?;
	submit_backlog(&client, &submitter, &signers, STORES_PER_SIGNER, "SHARE").await?;
	test_log!(TEST, "all submitted during block {submit_block}");

	let first_candidate = submit_block + 1;
	let last_candidate = first_candidate + 9;
	wait_for_finalized_height(collator, last_candidate, FINALITY_TIMEOUT_SECS).await?;

	let expected_share = MAX_BLOCK_STORES / SIGNER_COUNT;
	let mut full_blocks = 0;
	for number in first_candidate..=last_candidate {
		let block = classified_block(&client, number).await?;
		let counts = store_counts(&block);
		let total: usize = counts.values().sum();
		test_log!(TEST, "block {number}: {total} stores, per signer {:?}", counts.values());
		if total < MAX_BLOCK_STORES {
			continue;
		}
		full_blocks += 1;
		for signer in &signers {
			let count = counts.get(&Some(signer.keypair.public_key().0)).copied().unwrap_or(0);
			assert!(
				count.abs_diff(expected_share) <= 1,
				"block {number}: {} has {count} stores, expected {expected_share} ± 1",
				signer.label
			);
		}
	}
	assert!(
		full_blocks >= FULL_BLOCKS_REQUIRED,
		"only {full_blocks} full blocks in {first_candidate}..={last_candidate}, need {FULL_BLOCKS_REQUIRED}"
	);
	test_log!(TEST, "✓ {full_blocks} full blocks shared evenly between {SIGNER_COUNT} signers");
	Ok(())
}

/// While over-budget stores from four signers fill the pool, one transaction of each higher
/// category is submitted at once. In the first block that includes them, they come in level
/// order and before every backlog store.
#[tokio::test(flavor = "multi_thread")]
async fn parachain_priority_levels_order_within_block_test() -> Result<()> {
	const TEST: &str = "para_priority_levels";
	crate::utils::init_logging();
	test_log!(TEST, "=== category order under a backlog of over-budget stores ===");

	let (network, client, submitter) = network_and_client().await?;
	let collator = network.get_node("collator-1")?;
	let spammers = signers();
	let in_budget = dev::one();
	let payer = dev::two();
	let renew_owner = dev::alice();
	let new_account = dev::two().derive([DeriveJunction::hard("priority_levels")]);

	for spammer in &spammers {
		authorize_from_eve(&client, spammer.keypair.public_key().0, OVER_BUDGET_ALLOWANCE_BYTES)
			.await?;
	}
	authorize_from_eve(&client, in_budget.public_key().0, LARGE_ALLOWANCE_BYTES).await?;
	fund_from_alice(&client, payer.public_key().0).await?;

	let renew_data = generate_test_data(STORE_BYTES, b"PRIORITY_RENEW_SOURCE_");
	submit_and_wait_finalized(&client, &renew_owner, &store_payload(renew_data.clone())).await?;
	let renew_hash = blake2_256(&renew_data);

	let preimage_data = generate_test_data(STORE_BYTES, b"PRIORITY_PREIMAGE_");
	authorize_preimage_from_eve(&client, blake2_256(&preimage_data), STORE_BYTES as u64).await?;

	let in_budget_store = signed_bytes(
		&client,
		&in_budget,
		&store_payload(generate_test_data(STORE_BYTES, b"PRIORITY_IN_BUDGET_")),
		account_nonce(&client, &in_budget).await?,
		0,
	)?;
	let renew = signed_bytes(
		&client,
		&renew_owner,
		&tx(
			"DataRenewal",
			"force_renew",
			vec![Value::unnamed_variant("ContentHash", [Value::from_bytes(renew_hash)])],
		),
		account_nonce(&client, &renew_owner).await?,
		0,
	)?;
	let preimage_store = bare_bytes(&client, &store_payload(preimage_data))?;
	let authorize = signed_bytes(
		&client,
		&dev::eve(),
		&tx(
			"TransactionStorage",
			"authorize_account",
			vec![
				Value::from_bytes(new_account.public_key().0),
				Value::u128(1),
				Value::u128(STORE_BYTES as u128),
			],
		),
		account_nonce(&client, &dev::eve()).await?,
		0,
	)?;
	let transfer = signed_bytes(
		&client,
		&payer,
		&transfer_payload(new_account.public_key().0),
		account_nonce(&client, &payer).await?,
		0,
	)?;

	test_log!(TEST, "submitting {} backlog stores", SIGNER_COUNT * BACKLOG_PER_SIGNER);
	submit_backlog(&client, &submitter, &spammers, BACKLOG_PER_SIGNER, "BACKLOG").await?;
	let backlog_block = best_block_number(&client).await?;
	wait_for_block_height(collator, backlog_block + 1, BLOCK_WAIT_TIMEOUT_SECS).await?;

	// The five transactions come from five different accounts. They are submitted in
	// parallel in a per-run random order, so the test never controls arrival order and
	// only the priority rules can produce the asserted layout. The seed is logged so a
	// failing order can be replayed.
	let mut injected = vec![
		("transfer", transfer),
		("in_budget_store", in_budget_store),
		("renew", renew),
		("preimage_store", preimage_store),
		("authorize", authorize),
	];
	let seed = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|elapsed| elapsed.as_nanos() as u64)
		.unwrap_or(1);
	shuffle(&mut injected, seed);
	let order: Vec<&str> = injected.iter().map(|(label, _)| *label).collect();
	test_log!(
		TEST,
		"submitting one transaction per category in parallel, seed {seed}, order {order:?}"
	);
	let injected_block = best_block_number(&client).await?;
	submitter
		.submit_all(injected.into_iter().map(|(_, bytes)| bytes).collect())
		.await?;
	let last_candidate = injected_block + 4;
	wait_for_finalized_height(collator, last_candidate, FINALITY_TIMEOUT_SECS).await?;

	let in_budget_account = in_budget.public_key().0;
	let mut verified_block = None;
	for number in injected_block + 1..=last_candidate {
		let block = classified_block(&client, number).await?;
		let position = |predicate: &dyn Fn(&ClassifiedExtrinsic) -> bool| {
			block
				.iter()
				.find(|extrinsic| predicate(extrinsic))
				.map(|extrinsic| extrinsic.index)
		};
		let authorize_index = position(&|extrinsic| extrinsic.category == Category::Authorize);
		let Some(authorize_index) = authorize_index else { continue };
		let renew_index = position(&|extrinsic| extrinsic.category == Category::Renew)
			.ok_or_else(|| anyhow!("block {number}: renew missing"))?;
		let in_budget_index = position(&|extrinsic| {
			extrinsic.category == Category::Store && extrinsic.signer == Some(in_budget_account)
		})
		.ok_or_else(|| anyhow!("block {number}: in-budget store missing"))?;
		let preimage_index = position(&|extrinsic| {
			extrinsic.category == Category::Store && extrinsic.signer.is_none()
		})
		.ok_or_else(|| anyhow!("block {number}: preimage store missing"))?;
		let transfer_index = position(&|extrinsic| extrinsic.category == Category::Transfer)
			.ok_or_else(|| anyhow!("block {number}: transfer missing"))?;
		let first_backlog_index = position(&|extrinsic| {
			extrinsic.category == Category::Store &&
				extrinsic.signer.is_some() &&
				extrinsic.signer != Some(in_budget_account)
		})
		.ok_or_else(|| anyhow!("block {number}: no backlog store, contention not proven"))?;

		test_log!(
			TEST,
			"block {number}: authorize@{authorize_index} renew@{renew_index} in_budget@{in_budget_index} preimage@{preimage_index} transfer@{transfer_index} first_backlog@{first_backlog_index}"
		);
		let level_four_max = renew_index.max(in_budget_index).max(preimage_index);
		let level_four_min = renew_index.min(in_budget_index).min(preimage_index);
		assert!(authorize_index < level_four_min, "authorize must precede level-4 transactions");
		assert!(level_four_max < transfer_index, "level-4 transactions must precede the transfer");
		assert!(transfer_index < first_backlog_index, "transfer must precede over-budget stores");
		verified_block = Some(number);
		break;
	}
	let verified_block = verified_block.ok_or_else(|| {
		anyhow!(
			"no block in {}..={last_candidate} contains the injected transactions",
			injected_block + 1
		)
	})?;
	test_log!(TEST, "✓ category order verified in block {verified_block}");
	Ok(())
}
