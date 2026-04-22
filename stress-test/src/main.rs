use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use std::{
	path::PathBuf,
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	},
};
use subxt_signer::sr25519::Keypair;

use bulletin_stress_test::{
	accounts, bitswap,
	chain_info::{ChainLimits, EnvironmentInfo},
	client, report, scenarios,
};

#[derive(Parser)]
#[command(name = "bulletin-stress-test", about = "Stress test the Bulletin Chain")]
struct Cli {
	#[command(subcommand)]
	command: Commands,

	/// WebSocket URL(s) of the bulletin chain node(s).
	/// Comma-separated for multi-node submission (e.g. "ws://rpc1:9944,ws://rpc2:9955").
	/// First URL is used for control (authorization, monitoring); all are used for submission.
	#[arg(long, default_value = "ws://127.0.0.1:9944", global = true)]
	ws_url: String,

	/// Node's P2P multiaddr for Bitswap retrieval (auto-discovered if omitted)
	#[arg(long, global = true)]
	p2p_multiaddr: Option<String>,

	/// Seed for authorizer account (must be in the runtime's Authorizer origin)
	#[arg(long, default_value = "//Alice", global = true)]
	authorizer_seed: String,

	/// Number of unique items per size (for bitswap read tests)
	#[arg(long, default_value = "512", global = true)]
	iterations: u32,

	/// WebSocket RPC connections for store submission (one async worker per connection, fed by
	/// bounded channels from the work reader; increase for remote RPCs)
	#[arg(long, default_value = "4", global = true)]
	submitters: usize,

	/// Number of steady-state blocks to measure per variant (excludes ramp-up/down)
	#[arg(long, default_value = "5", global = true)]
	target_blocks: u32,

	/// Block-capacity only: measured blocks worth of transactions per pipeline iteration (chunk
	/// size)
	#[arg(long, default_value = "20", global = true)]
	iteration_blocks: u32,

	/// Block-capacity `--variants mixed` only: seed for random payload-size draws (reproducible
	/// runs)
	#[arg(long, global = true)]
	mix_seed: Option<u64>,

	/// Output format
	#[arg(long, default_value = "text", global = true)]
	output: OutputFormat,

	/// JSON output file (flushed after every variant so partial results survive crashes)
	#[arg(long, global = true)]
	output_file: Option<PathBuf>,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
	Text,
	Json,
}

#[derive(Subcommand)]
enum Commands {
	/// Run throughput benchmarks (write capacity)
	Throughput {
		/// Which test: block-capacity
		#[arg(default_value = "block-capacity")]
		test: String,

		/// Comma-separated payload size labels (e.g. "1KB,128KB,1MB") or **MIXED** for a weighted
		/// real-world size mix. Omit to run all fixed sizes (no mixed).
		#[arg(long)]
		variants: Option<String>,
	},
	/// Run Bitswap read benchmarks
	Bitswap {
		/// Which test: b2
		#[arg(default_value = "b2")]
		test: String,

		/// Payload size in bytes for each stored item (default: 128KB)
		#[arg(long, default_value = "131072")]
		payload_size: usize,
	},
	/// Run HOP (Hand-off Protocol) stress tests
	Hop {
		/// Scenario: submit-only, full-cycle, group, pool-fill, mixed, errors, all
		#[arg(default_value = "all")]
		scenario: String,

		/// Number of items to submit
		#[arg(long, default_value = "100")]
		items: u32,

		/// Payload size in bytes (omit to sweep standard sizes for submit-only)
		#[arg(long)]
		payload_size: Option<usize>,

		/// Parallel submit/claim streams
		#[arg(long, default_value = "4")]
		concurrency: usize,

		/// Recipients per entry (for group scenario)
		#[arg(long, default_value = "10")]
		recipients: usize,

		/// Duration in seconds (for mixed scenario)
		#[arg(long, default_value = "30")]
		duration: u64,
	},
	/// Run all test suites (block-capacity + bitswap + hop)
	Full,
}

#[tokio::main]
async fn main() -> Result<()> {
	env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

	let cli = Cli::parse();

	// Parse comma-separated WS URLs. First URL is used for control operations
	// (authorization, chain info, monitoring); all URLs are used for submission.
	let ws_urls: Vec<String> = cli
		.ws_url
		.split(',')
		.map(|s| s.trim().to_string())
		.filter(|s| !s.is_empty())
		.collect();
	let control_url = &ws_urls[0];
	log::info!(
		"WS URLs: {} total (control: {control_url}{})",
		ws_urls.len(),
		if ws_urls.len() > 1 { format!(", submit: {}", ws_urls.join(", ")) } else { String::new() }
	);

	let ws_url_refs: Vec<&str> = ws_urls.iter().map(|s| s.as_str()).collect();

	// HOP command is pure RPC — skip subxt client, authorization, and chain limits.
	if let Commands::Hop { ref scenario, items, payload_size, concurrency, recipients, duration } =
		cli.command
	{
		return run_hop_standalone(
			&ws_url_refs,
			scenario,
			items,
			payload_size,
			concurrency,
			recipients,
			duration,
			&cli,
		)
		.await;
	}

	let client = client::connect(control_url).await?;

	let authorizer_secret_uri: subxt_signer::SecretUri = cli
		.authorizer_seed
		.parse()
		.map_err(|e| anyhow::anyhow!("Invalid authorizer seed: {e}"))?;
	let authorizer_signer = Keypair::from_uri(&authorizer_secret_uri)
		.map_err(|e| anyhow::anyhow!("Failed to create keypair: {e}"))?;

	let nonce_tracker = accounts::NonceTracker::new();

	// Initialize authorizer nonce
	let authorizer_account_id = authorizer_signer.public_key().to_account_id();
	log::info!("Initializing authorizer nonce from chain...");
	nonce_tracker.init_from_chain(&client, &authorizer_account_id).await?;
	log::info!("Authorizer nonce initialized");

	// Query environment info and chain limits
	log::info!("Querying environment info from RPC...");
	let env_info = EnvironmentInfo::query(&client, control_url).await?;
	log::info!("Environment info OK");
	if matches!(cli.output, OutputFormat::Text) {
		env_info.print_text();
	}
	log::info!("Querying chain limits (block weights, storage limits, store weight regression)...");
	let chain_limits = ChainLimits::query(&client, &authorizer_signer, &nonce_tracker).await?;
	log::info!("Chain limits OK");
	if matches!(cli.output, OutputFormat::Text) {
		chain_limits.print_text();
	}

	let mut all_results = Vec::new();
	let mut command_error = None;
	let cancel = Arc::new(AtomicBool::new(false));

	// Spawn Ctrl+C handler that sets the cancel flag instead of killing the process.
	// This lets the pipeline finish gracefully and produce partial results.
	{
		let cancel = cancel.clone();
		tokio::spawn(async move {
			tokio::signal::ctrl_c().await.ok();
			log::warn!("Ctrl+C received — stopping gracefully to collect partial results");
			cancel.store(true, Ordering::Relaxed);
			tokio::signal::ctrl_c().await.ok();
			log::warn!("Second Ctrl+C — force exit");
			std::process::exit(130);
		});
	}

	// Closure that stamps metadata and flushes results to --output-file after each variant.
	let flush = |results: &mut Vec<report::ScenarioResult>| {
		// Stamp chain_limits and environment onto the latest result.
		if let Some(last) = results.last_mut() {
			if last.chain_limits.is_none() {
				last.chain_limits = Some(chain_limits.clone());
			}
			if last.environment.is_none() {
				last.environment = Some(env_info.clone());
			}
		}
		if let Some(ref path) = cli.output_file {
			if let Ok(json) = serde_json::to_string_pretty(results) {
				if let Err(e) = std::fs::write(path, &json) {
					log::warn!("Failed to write results to {}: {e}", path.display());
				} else {
					log::info!(
						"Results flushed to {} ({} variants)",
						path.display(),
						results.len()
					);
				}
			}
		}
	};

	match cli.command {
		Commands::Throughput { ref test, ref variants } => {
			if let Err(e) = run_throughput(
				&client,
				&authorizer_signer,
				&nonce_tracker,
				&cli,
				test,
				variants.as_deref(),
				&chain_limits,
				&ws_url_refs,
				&mut all_results,
				&flush,
				&cancel,
			)
			.await
			{
				log::error!("Throughput command failed: {e}");
				command_error = Some(e);
			}
		},
		Commands::Bitswap { ref test, payload_size } => {
			if let Err(e) = run_bitswap(
				&client,
				&authorizer_signer,
				&nonce_tracker,
				&cli,
				test,
				payload_size,
				control_url,
				&mut all_results,
				&flush,
			)
			.await
			{
				log::error!("Bitswap command failed: {e}");
				command_error = Some(e);
			}
		},
		Commands::Hop {
			ref scenario,
			items,
			payload_size,
			concurrency,
			recipients,
			duration,
		} => {
			if let Err(e) = scenarios::hop::run_hop_sweep(
				&ws_url_refs,
				scenario,
				items,
				payload_size,
				concurrency,
				recipients,
				duration,
				&mut all_results,
				&flush,
				&cancel,
			)
			.await
			{
				log::error!("HOP command failed: {e}");
				command_error = Some(e);
			}
		},
		Commands::Full => {
			if let Err(e) = run_throughput(
				&client,
				&authorizer_signer,
				&nonce_tracker,
				&cli,
				"block-capacity",
				None,
				&chain_limits,
				&ws_url_refs,
				&mut all_results,
				&flush,
				&cancel,
			)
			.await
			{
				log::error!("Throughput command failed: {e}");
				command_error = Some(e);
			}
			if command_error.is_none() && !cancel.load(Ordering::Relaxed) {
				if let Err(e) = run_bitswap(
					&client,
					&authorizer_signer,
					&nonce_tracker,
					&cli,
					"b2",
					128 * 1024,
					control_url,
					&mut all_results,
					&flush,
				)
				.await
				{
					log::error!("Bitswap command failed: {e}");
					command_error = Some(e);
				}
			}
			if command_error.is_none() && !cancel.load(Ordering::Relaxed) {
				if let Err(e) = scenarios::hop::run_hop_sweep(
					&ws_url_refs,
					"all",
					100,
					None,
					4,
					10,
					30,
					&mut all_results,
					&flush,
					&cancel,
				)
				.await
				{
					log::error!("HOP command failed: {e}");
					command_error = Some(e);
				}
			}
		},
	}

	// Always print results (even partial / aborted) before exiting.
	match cli.output {
		OutputFormat::Text =>
			for result in &all_results {
				result.print_text();
			},
		OutputFormat::Json => {
			println!("{}", serde_json::to_string_pretty(&all_results)?);
		},
	}

	if all_results.len() > 1 && matches!(cli.output, OutputFormat::Text) {
		report::print_summary_table(&all_results);
	}

	if cancel.load(Ordering::Relaxed) {
		// Flush to file one last time before exiting.
		flush(&mut all_results);
		std::process::exit(130);
	}

	if let Some(e) = command_error {
		return Err(e);
	}

	Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_throughput(
	client: &subxt::OnlineClient<client::BulletinConfig>,
	authorizer_signer: &Keypair,
	nonce_tracker: &accounts::NonceTracker,
	cli: &Cli,
	test: &str,
	variants: Option<&str>,
	chain_limits: &ChainLimits,
	ws_urls: &[&str],
	results: &mut Vec<report::ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<report::ScenarioResult>),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	match test {
		"block-capacity" | "all" => {
			scenarios::throughput::run_block_capacity_sweep(
				client,
				authorizer_signer,
				nonce_tracker,
				ws_urls,
				chain_limits,
				cli.submitters,
				cli.target_blocks,
				cli.iteration_blocks,
				variants,
				cli.mix_seed,
				results,
				on_result,
				cancel,
			)
			.await?;
		},
		other => anyhow::bail!("Unknown throughput test: {other} (expected: block-capacity)"),
	}
	Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_bitswap(
	client: &subxt::OnlineClient<client::BulletinConfig>,
	authorizer_signer: &Keypair,
	nonce_tracker: &accounts::NonceTracker,
	cli: &Cli,
	test: &str,
	payload_size: usize,
	control_url: &str,
	results: &mut Vec<report::ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<report::ScenarioResult>),
) -> Result<()> {
	let multiaddr = match resolve_p2p_multiaddr(cli, control_url).await {
		Ok(r) => r,
		Err(e) => {
			log::warn!("Bitswap tests skipped: could not resolve P2P address: {e}");
			return Ok(());
		},
	};

	match test {
		"b2" => {
			let rs = scenarios::bitswap_read::run_b2_concurrent_read_sweep(
				client,
				authorizer_signer,
				nonce_tracker,
				&multiaddr,
				cli.iterations,
				payload_size,
				control_url,
			)
			.await?;
			for r in rs {
				results.push(r);
				on_result(results);
			}
		},
		other => anyhow::bail!("Unknown bitswap test: {other} (expected: b2)"),
	}

	Ok(())
}

/// Run HOP stress tests without subxt client or chain-limits setup.
#[allow(clippy::too_many_arguments)]
async fn run_hop_standalone(
	ws_urls: &[&str],
	scenario: &str,
	items: u32,
	payload_size: Option<usize>,
	concurrency: usize,
	recipients: usize,
	duration: u64,
	cli: &Cli,
) -> Result<()> {
	let mut all_results = Vec::new();
	let cancel = Arc::new(AtomicBool::new(false));

	{
		let cancel = cancel.clone();
		tokio::spawn(async move {
			tokio::signal::ctrl_c().await.ok();
			log::warn!("Ctrl+C received — stopping gracefully");
			cancel.store(true, Ordering::Relaxed);
			tokio::signal::ctrl_c().await.ok();
			std::process::exit(130);
		});
	}

	let flush = |results: &mut Vec<report::ScenarioResult>| {
		if let Some(ref path) = cli.output_file {
			if let Ok(json) = serde_json::to_string_pretty(results) {
				if let Err(e) = std::fs::write(path, &json) {
					log::warn!("Failed to write results to {}: {e}", path.display());
				}
			}
		}
	};

	scenarios::hop::run_hop_sweep(
		ws_urls,
		scenario,
		items,
		payload_size,
		concurrency,
		recipients,
		duration,
		&mut all_results,
		&flush,
		&cancel,
	)
	.await?;

	match cli.output {
		OutputFormat::Text =>
			for result in &all_results {
				result.print_text();
			},
		OutputFormat::Json => {
			println!("{}", serde_json::to_string_pretty(&all_results)?);
		},
	}

	if all_results.len() > 1 && matches!(cli.output, OutputFormat::Text) {
		report::print_summary_table(&all_results);
	}

	Ok(())
}

/// Resolve the node's P2P multiaddr from CLI args or RPC auto-discovery.
async fn resolve_p2p_multiaddr(
	cli: &Cli,
	control_url: &str,
) -> Result<litep2p::types::multiaddr::Multiaddr> {
	let multiaddr_str = match &cli.p2p_multiaddr {
		Some(addr) => bitswap::clean_multiaddr(addr),
		None => {
			log::info!("Auto-discovering P2P address via RPC...");
			let (peer_id_str, addresses) = client::discover_p2p_info(control_url).await?;
			log::info!("Node peer ID: {peer_id_str}");
			log::info!("Node listen addresses: {addresses:?}");

			let raw =
				addresses
					.iter()
					.find(|a| a.contains("/ws"))
					.or_else(|| addresses.first())
					.map(|a| {
						if a.contains("/p2p/") {
							a.clone()
						} else {
							format!("{a}/p2p/{peer_id_str}")
						}
					})
					.ok_or_else(|| anyhow::anyhow!("No P2P addresses discovered"))?;
			bitswap::clean_multiaddr(&raw)
		},
	};

	log::info!("Resolved P2P multiaddr: {multiaddr_str}");
	let multiaddr: litep2p::types::multiaddr::Multiaddr = multiaddr_str.parse()?;
	// Validate that the multiaddr contains a peer ID
	bitswap::BitswapClient::peer_id_from_multiaddr(&multiaddr)?;

	Ok(multiaddr)
}
