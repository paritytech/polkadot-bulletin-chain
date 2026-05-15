// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0
//
// TEMPORARY probe — measures fork-view divergence on the parachain stream and dumps
// authoring / txpool / collator-protocol logs to help explain why subxt's
// `subscribe_best` ever emits a hash that finalization later overrides.
//
// Run with:
//   cargo test --release -p bulletin-chain-zombienet-sdk-tests \
//     --features zombie-auto-renew-tests \
//     fork_probe -- --ignored --nocapture --test-threads=1

use crate::utils::{
	get_alice_nonce, get_para_id, get_parachain_binary_path, get_parachain_chain_spec,
	get_relay_binary_path, get_relay_chain, init_logging, initialize_network,
	polkadot_mainnet_async_backing_overrides, verify_parachain_binaries, wait_for_finalized_height,
	BLOCK_PRODUCTION_TIMEOUT_SECS, NETWORK_READY_TIMEOUT_SECS,
};
use anyhow::{anyhow, Result};
use std::{
	collections::{HashMap, HashSet},
	sync::Arc,
};
use subxt::{
	config::substrate::{SubstrateConfig, SubstrateExtrinsicParamsBuilder},
	dynamic::{tx, Value},
	OnlineClient,
};
use subxt_signer::sr25519::dev;
use tokio::sync::Mutex;
use zombienet_sdk::{NetworkConfig, NetworkConfigBuilder};

const TARGET_FINALIZED_BLOCKS: u64 = 50;

// Match the regular tests' log levels — anything more aggressive slows startup enough that
// the chain doesn't finalize fast enough for our probe to subscribe. Per-node log files
// still carry default-INFO collator/relay output for post-hoc forensics.
const RELAY_LOG_ARGS: &str = "-lruntime=debug";
// Collator arg list. Arguments after "--" go to the embedded relay client. The
// `--network-backend=libp2p` flag is critical: the default litep2p backend on our pinned
// polkadot-sdk commit crashes the collator at startup with
// `Websocket listener terminated with error Kind(InvalidInput)` → `network-worker` essential
// task failure → panic.
const COLLATOR_ARGS: &[&str] =
	&["-lparachain=info,aura=debug,cumulus=debug,txpool=info", "--", "--network-backend=libp2p"];

#[tokio::test(flavor = "multi_thread")]
#[ignore = "temporary fork-view probe — run with --ignored"]
async fn fork_probe_2_validators() -> Result<()> {
	init_logging();
	probe(2).await?.print();
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "temporary fork-view probe — run with --ignored"]
async fn fork_probe_3_validators() -> Result<()> {
	init_logging();
	probe(3).await?.print();
	Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "temporary fork-view probe — run with --ignored"]
async fn fork_probe_5_validators() -> Result<()> {
	init_logging();
	probe(5).await?.print();
	Ok(())
}

#[derive(Clone)]
struct BlockInfo {
	hash: subxt::utils::H256,
	height: u64,
	parent_hash: subxt::utils::H256,
	extrinsics: Vec<String>,
	seen_as_best: bool,
	canonical: bool,
}

struct ProbeStats {
	validators: usize,
	finalized_observed: u64,
	multi_view_heights: Vec<(u64, usize)>,
	canonical_missing_from_best: Vec<u64>,
	peer_counts: Vec<(String, u64)>,
	blocks: HashMap<subxt::utils::H256, BlockInfo>,
	min_height: u64,
	max_height: u64,
}

impl ProbeStats {
	fn print(&self) {
		println!("\n=== {}-validator probe ===", self.validators);
		println!("Finalized blocks observed: {}", self.finalized_observed);
		println!("Peer counts at end of run:");
		for (name, n) in &self.peer_counts {
			println!("  {} → {} peers", name, n);
		}
		println!(
			"Heights where subscribe_best saw >1 distinct hash: {}",
			self.multi_view_heights.len()
		);
		for (n, c) in &self.multi_view_heights {
			println!("  height {} → {} distinct best hashes", n, c);
		}
		println!(
			"Heights where canonical hash never appeared in best-set: {}",
			self.canonical_missing_from_best.len()
		);
		for n in &self.canonical_missing_from_best {
			println!("  height {}", n);
		}
		self.print_tree();
	}

	fn print_tree(&self) {
		println!("\nFork history (height {} → {}):", self.min_height, self.max_height);
		println!(
			"  legend: [C]=canonical, [B]=seen-by-subscribe_best, [B-only]=best-but-not-canonical, [C-only]=canonical-but-never-best"
		);
		let mut by_height: HashMap<u64, Vec<&BlockInfo>> = HashMap::new();
		for b in self.blocks.values() {
			by_height.entry(b.height).or_default().push(b);
		}
		let mut heights: Vec<u64> = by_height.keys().cloned().collect();
		heights.sort();
		for h in heights {
			let mut blocks_at_h = by_height.remove(&h).unwrap();
			blocks_at_h.sort_by_key(|b| b.hash);
			let is_fork = blocks_at_h.len() > 1;
			for b in &blocks_at_h {
				let marker = match (b.canonical, b.seen_as_best) {
					(true, true) => "[C+B]",
					(true, false) => "[C-only]",
					(false, true) => "[B-only]",
					(false, false) => "[?]",
				};
				let connector = if is_fork { "├─" } else { "└─" };
				println!(
					"  h={:>4} {} 0x{}.. {} parent=0x{}..",
					b.height,
					connector,
					hex_short(&b.hash),
					marker,
					hex_short(&b.parent_hash),
				);
				for ext in &b.extrinsics {
					println!("        ext: {}", ext);
				}
			}
		}
		println!();
	}
}

fn hex_short(h: &subxt::utils::H256) -> String {
	let bytes = h.0;
	format!("{:02x}{:02x}{:02x}{:02x}", bytes[0], bytes[1], bytes[2], bytes[3])
}

async fn probe(num_validators: usize) -> Result<ProbeStats> {
	verify_parachain_binaries()?;
	let config = build_topology(num_validators)?;
	let network = initialize_network(config).await?;
	network.wait_until_is_up(NETWORK_READY_TIMEOUT_SECS).await?;
	let collator = network.get_node("collator-1").map_err(|e| anyhow!("collator-1: {e}"))?;
	// Give the parachain time to settle before subscribing — fresh networks can drop the
	// chainHead websocket before producing their first finalized block.
	wait_for_finalized_height(collator, 3, BLOCK_PRODUCTION_TIMEOUT_SECS).await?;
	let client: OnlineClient<SubstrateConfig> = collator.wait_client().await?;

	let alice = dev::alice();
	let nonce: u64 = get_alice_nonce(collator).await?;

	let best_at: Arc<Mutex<HashMap<u64, HashSet<subxt::utils::H256>>>> = Default::default();
	let final_at: Arc<Mutex<HashMap<u64, subxt::utils::H256>>> = Default::default();

	let best_at_w = best_at.clone();
	let client_w = client.clone();
	let alice_w = alice.clone();
	let best_task = tokio::spawn(async move {
		let mut n_local = nonce;
		loop {
			let mut best_sub = client_w.blocks().subscribe_best().await?;
			while let Some(item) = best_sub.next().await {
				let Ok(block) = item else { continue };
				best_at_w
					.lock()
					.await
					.entry(block.number() as u64)
					.or_default()
					.insert(block.hash());
				let call = tx("System", "remark", vec![Value::from_bytes(b"probe".to_vec())]);
				let params = SubstrateExtrinsicParamsBuilder::new().nonce(n_local).build();
				if client_w.tx().sign_and_submit(&call, &alice_w, params).await.is_ok() {
					n_local += 1;
				}
			}
			tracing::warn!("best stream ended, re-subscribing");
		}
		#[allow(unreachable_code)]
		anyhow::Ok(())
	});

	let final_at_w = final_at.clone();
	let final_task = tokio::spawn(async move {
		let mut start: Option<u64> = None;
		loop {
			let mut final_sub = client.blocks().subscribe_finalized().await?;
			while let Some(item) = final_sub.next().await {
				let Ok(block) = item else { continue };
				let n = block.number() as u64;
				final_at_w.lock().await.insert(n, block.hash());
				let s = *start.get_or_insert(n);
				if n.saturating_sub(s) >= TARGET_FINALIZED_BLOCKS {
					return anyhow::Ok(());
				}
			}
			tracing::warn!("finalized stream ended, re-subscribing");
		}
	});

	final_task.await??;
	best_task.abort();

	// Read peer counts from every node's Prometheus endpoint.
	let mut peer_counts: Vec<(String, u64)> = Vec::new();
	for node in network.relaychain().nodes() {
		let name = node.name().to_string();
		let count = node.reports("substrate_sub_libp2p_peers_count").await.unwrap_or(-1.0);
		peer_counts.push((format!("relay/{}", name), count.max(0.0) as u64));
	}
	for para in network.parachains() {
		for node in para.collators() {
			let name = node.name().to_string();
			let count = node.reports("substrate_sub_libp2p_peers_count").await.unwrap_or(-1.0);
			peer_counts.push((format!("para/{}", name), count.max(0.0) as u64));
		}
	}

	let best_at = best_at.lock().await.clone();
	let final_at = final_at.lock().await.clone();

	let mut multi_view_heights = Vec::new();
	let mut canonical_missing_from_best = Vec::new();
	let mut heights: Vec<u64> = final_at.keys().cloned().collect();
	heights.sort();
	for n in &heights {
		let canonical = final_at[n];
		if let Some(hashes) = best_at.get(n) {
			if hashes.len() > 1 {
				multi_view_heights.push((*n, hashes.len()));
			}
			if !hashes.contains(&canonical) {
				canonical_missing_from_best.push(*n);
			}
		}
	}

	// Collect block info for the canonical chain + all observed best-only forks.
	let mut blocks: HashMap<subxt::utils::H256, BlockInfo> = HashMap::new();
	let min_height = heights.first().copied().unwrap_or(0);
	let max_height = heights.last().copied().unwrap_or(0);

	// Walk canonical chain from max_height back to min_height.
	if let Some(&top_canonical) = final_at.get(&max_height) {
		let mut cur_hash = top_canonical;
		loop {
			let Ok(block) = collator_client(&network).await?.blocks().at(cur_hash).await else {
				break
			};
			let h = block.number() as u64;
			let info = BlockInfo {
				hash: cur_hash,
				height: h,
				parent_hash: block.header().parent_hash,
				extrinsics: extrinsic_summary(&block).await.unwrap_or_default(),
				seen_as_best: false,
				canonical: true,
			};
			let parent = info.parent_hash;
			blocks.insert(cur_hash, info);
			if h <= min_height {
				break;
			}
			cur_hash = parent;
		}
	}

	// Add best-seen blocks not already on the canonical chain.
	let cli = collator_client(&network).await?;
	for (h, hashes) in &best_at {
		for hash in hashes {
			if let Some(info) = blocks.get_mut(hash) {
				info.seen_as_best = true;
				continue;
			}
			if let Ok(block) = cli.blocks().at(*hash).await {
				blocks.insert(
					*hash,
					BlockInfo {
						hash: *hash,
						height: *h,
						parent_hash: block.header().parent_hash,
						extrinsics: extrinsic_summary(&block).await.unwrap_or_default(),
						seen_as_best: true,
						canonical: false,
					},
				);
			}
		}
	}

	Ok(ProbeStats {
		validators: num_validators,
		finalized_observed: final_at.len() as u64,
		multi_view_heights,
		canonical_missing_from_best,
		peer_counts,
		blocks,
		min_height,
		max_height,
	})
}

async fn collator_client(
	network: &zombienet_sdk::Network<zombienet_sdk::LocalFileSystem>,
) -> Result<OnlineClient<SubstrateConfig>> {
	let node = network.get_node("collator-1").map_err(|e| anyhow!("collator-1: {e}"))?;
	Ok(node.wait_client().await?)
}

async fn extrinsic_summary(
	block: &subxt::blocks::Block<SubstrateConfig, OnlineClient<SubstrateConfig>>,
) -> Result<Vec<String>> {
	let ext = block.extrinsics().await?;
	let mut out = Vec::new();
	for item in ext.iter() {
		let pallet = item.pallet_name().unwrap_or("?").to_string();
		let variant = item.variant_name().unwrap_or("?").to_string();
		out.push(format!("{}.{}", pallet, variant));
	}
	Ok(out)
}

fn build_topology(num_validators: usize) -> Result<NetworkConfig> {
	let relay_binary = get_relay_binary_path();
	let para_binary = get_parachain_binary_path();
	let para_chain_spec = get_parachain_chain_spec();
	let relay_chain = get_relay_chain();
	let para_id = get_para_id();
	let relay_args: Vec<_> = vec![RELAY_LOG_ARGS].into_iter().map(|s| s.into()).collect();
	let args2 = relay_args.clone();
	let args3 = relay_args.clone();
	let args4 = relay_args.clone();
	let args5 = relay_args.clone();
	let para_args: Vec<_> = COLLATOR_ARGS.iter().map(|s| (*s).into()).collect();

	NetworkConfigBuilder::new()
		.with_relaychain(|r| {
			let mut r = r
				.with_chain(relay_chain.as_str())
				.with_default_command(relay_binary.as_str())
				.with_genesis_overrides(polkadot_mainnet_async_backing_overrides())
				.with_node(|n| n.with_name("alice").validator(true).with_args(relay_args));
			if num_validators >= 2 {
				r = r.with_node(|n| n.with_name("bob").validator(true).with_args(args2));
			}
			if num_validators >= 3 {
				r = r.with_node(|n| n.with_name("charlie").validator(true).with_args(args3));
			}
			if num_validators >= 4 {
				r = r.with_node(|n| n.with_name("dave").validator(true).with_args(args4));
			}
			if num_validators >= 5 {
				r = r.with_node(|n| n.with_name("eve").validator(true).with_args(args5));
			}
			r
		})
		.with_parachain(|p| {
			p.with_id(para_id)
				.with_chain_spec_path(para_chain_spec.as_str())
				.cumulus_based(true)
				.with_collator(|c| {
					c.with_name("collator-1")
						.validator(true)
						.with_command(para_binary.as_str())
						.with_args(para_args)
				})
		})
		.with_global_settings(|gs| match std::env::var("ZOMBIENET_SDK_BASE_DIR") {
			Ok(val) => gs.with_base_dir(val),
			_ => gs,
		})
		.build()
		.map_err(|errs| {
			let msg = errs.into_iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
			anyhow!("config errs: {msg}")
		})
}
