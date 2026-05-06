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

	/// Node's P2P multiaddr(s) for Bitswap retrieval (comma-separated for
	/// multi-peer, auto-discovered if omitted)
	#[arg(long, global = true, value_delimiter = ',')]
	p2p_multiaddr: Vec<String>,

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
		/// Which test: block-capacity, sequential-upload
		#[arg(default_value = "block-capacity")]
		test: String,

		/// Comma-separated payload size labels (e.g. "1KB,128KB,1MB") or **MIXED** for a weighted
		/// real-world size mix. Omit to run all fixed sizes (no mixed).
		#[arg(long)]
		variants: Option<String>,

		/// Total upload size in bytes (sequential-upload only, default: 20MB)
		#[arg(long, default_value = "20971520")]
		total_size: usize,

		/// Per-transaction chunk size in bytes (sequential-upload only, default: 32KB)
		#[arg(long, default_value = "32768")]
		chunk_size: usize,

		/// Number of parallel upload instances, each with a different account
		/// (sequential-upload only, default: 1)
		#[arg(long, default_value = "1")]
		instances: usize,
	},
	/// Run Bitswap read benchmarks
	Bitswap {
		/// Which test: b2, bulk-read
		#[arg(default_value = "b2")]
		test: String,

		/// Payload size in bytes for each stored item (default: 128KB, b2 only)
		#[arg(long, default_value = "131072")]
		payload_size: usize,

		/// Target data size to download in bytes (bulk-read only, default: 1GB)
		#[arg(long, default_value = "1073741824")]
		read_size: u64,

		/// Number of concurrent Bitswap clients (bulk-read only, default: 16)
		#[arg(long, default_value = "16")]
		read_concurrency: usize,

		/// Minimum item size in bytes to include (bulk-read only, default: 0)
		#[arg(long, default_value = "0")]
		min_size: u32,

		/// Maximum item size in bytes to include (bulk-read only, default: 16MB)
		#[arg(long, default_value = "16777216")]
		max_size: u32,

		/// CIDs per wantlist request (bulk-read only, 1=single, max 16, default: 1)
		#[arg(long, default_value = "1")]
		batch_size: usize,
	},
	/// Renew stress test — upload data then spam renew calls
	Renew {
		/// Number of items to store first (default: 512 + buffer)
		#[arg(long, default_value = "520")]
		store_count: usize,

		/// Chunk size per stored item in bytes (default: 32KB)
		#[arg(long, default_value = "32768")]
		chunk_size: usize,

		/// Number of blocks to fill with renew calls (default: 10)
		#[arg(long, default_value = "10")]
		target_blocks: u32,
	},
	/// Run all test suites (block-capacity + bitswap)
	Full,
}

#[tokio::main]
async fn main() -> Result<()> {
	tracing_subscriber::fmt()
		.with_writer(std::io::stderr)
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.init();

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
	tracing::info!(
		"WS URLs: {} total (control: {control_url}{})",
		ws_urls.len(),
		if ws_urls.len() > 1 { format!(", submit: {}", ws_urls.join(", ")) } else { String::new() }
	);

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
	tracing::info!("Initializing authorizer nonce from chain...");
	nonce_tracker.init_from_chain(&client, &authorizer_account_id).await?;
	tracing::info!("Authorizer nonce initialized");

	// Query environment info and chain limits
	tracing::info!("Querying environment info from RPC...");
	let env_info = EnvironmentInfo::query(&client, control_url).await?;
	tracing::info!("Environment info OK");
	if matches!(cli.output, OutputFormat::Text) {
		env_info.print_text();
	}
	tracing::info!(
		"Querying chain limits (block weights, storage limits, store weight regression)..."
	);
	let chain_limits = ChainLimits::query(&client, &authorizer_signer, &nonce_tracker).await?;
	tracing::info!("Chain limits OK");
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
			tracing::warn!("Ctrl+C received — stopping gracefully to collect partial results");
			cancel.store(true, Ordering::Relaxed);
			tokio::signal::ctrl_c().await.ok();
			tracing::warn!("Second Ctrl+C — force exit");
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
					tracing::warn!("Failed to write results to {}: {e}", path.display());
				} else {
					tracing::info!(
						"Results flushed to {} ({} variants)",
						path.display(),
						results.len()
					);
				}
			}
		}
	};

	let ws_url_refs: Vec<&str> = ws_urls.iter().map(|s| s.as_str()).collect();

	match cli.command {
		Commands::Throughput { ref test, ref variants, total_size, chunk_size, instances } =>
			if let Err(e) = run_throughput(
				&client,
				&authorizer_signer,
				&nonce_tracker,
				&cli,
				test,
				variants.as_deref(),
				&chain_limits,
				&ws_url_refs,
				total_size,
				chunk_size,
				instances,
				&mut all_results,
				&flush,
				&cancel,
			)
			.await
			{
				tracing::error!("Throughput command failed: {e}");
				command_error = Some(e);
			},
		Commands::Bitswap {
			ref test,
			payload_size,
			read_size,
			read_concurrency,
			min_size,
			max_size,
			batch_size,
		} => {
			if let Err(e) = run_bitswap(
				&client,
				&authorizer_signer,
				&nonce_tracker,
				&cli,
				test,
				payload_size,
				read_size,
				read_concurrency,
				min_size,
				max_size,
				batch_size,
				control_url,
				&mut all_results,
				&flush,
			)
			.await
			{
				tracing::error!("Bitswap command failed: {e}");
				command_error = Some(e);
			}
		},
		Commands::Renew { store_count, chunk_size, target_blocks } => {
			if let Err(e) = scenarios::renew::run_renew_stress(
				&client,
				&authorizer_signer,
				&nonce_tracker,
				&ws_url_refs,
				&chain_limits,
				store_count,
				chunk_size,
				target_blocks,
				&mut all_results,
				&flush,
			)
			.await
			{
				tracing::error!("Renew command failed: {e}");
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
				20 * 1024 * 1024,
				32 * 1024,
				1,
				&mut all_results,
				&flush,
				&cancel,
			)
			.await
			{
				tracing::error!("Throughput command failed: {e}");
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
					1024 * 1024 * 1024,
					16,
					0,
					16 * 1024 * 1024,
					1,
					control_url,
					&mut all_results,
					&flush,
				)
				.await
				{
					tracing::error!("Bitswap command failed: {e}");
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
	total_size: usize,
	chunk_size: usize,
	instances: usize,
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
		"sequential-upload" => {
			scenarios::throughput::run_sequential_upload(
				client,
				authorizer_signer,
				nonce_tracker,
				ws_urls,
				chain_limits,
				total_size,
				chunk_size,
				instances,
				results,
				on_result,
			)
			.await?;
		},
		other => anyhow::bail!(
			"Unknown throughput test: {other} (expected: block-capacity, sequential-upload)"
		),
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
	read_size: u64,
	read_concurrency: usize,
	min_size: u32,
	max_size: u32,
	batch_size: usize,
	control_url: &str,
	results: &mut Vec<report::ScenarioResult>,
	on_result: &dyn Fn(&mut Vec<report::ScenarioResult>),
) -> Result<()> {
	let multiaddrs = match resolve_p2p_multiaddrs(cli, control_url).await {
		Ok(r) => r,
		Err(e) => {
			tracing::warn!("Bitswap tests skipped: could not resolve P2P address: {e}");
			return Ok(());
		},
	};

	match test {
		"b2" => {
			let rs = scenarios::bitswap_read::run_b2_concurrent_read_sweep(
				client,
				authorizer_signer,
				nonce_tracker,
				&multiaddrs[0],
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
		"bulk-read" => {
			let r = scenarios::bitswap_bulk_read::run_bulk_read(
				client,
				&multiaddrs,
				read_size,
				read_concurrency,
				min_size,
				max_size,
				batch_size.clamp(1, 16),
				control_url,
			)
			.await?;
			results.push(r);
			on_result(results);
		},
		other => anyhow::bail!("Unknown bitswap test: {other} (expected: b2, bulk-read)"),
	}

	Ok(())
}

/// Resolve P2P multiaddrs from CLI args or RPC auto-discovery.
async fn resolve_p2p_multiaddrs(
	cli: &Cli,
	control_url: &str,
) -> Result<Vec<litep2p::types::multiaddr::Multiaddr>> {
	let addrs = if !cli.p2p_multiaddr.is_empty() {
		cli.p2p_multiaddr
			.iter()
			.map(|addr| {
				let cleaned = bitswap::clean_multiaddr(addr);
				let ma: litep2p::types::multiaddr::Multiaddr = cleaned.parse()?;
				bitswap::BitswapClient::peer_id_from_multiaddr(&ma)?;
				Ok(ma)
			})
			.collect::<Result<Vec<_>>>()?
	} else {
		tracing::info!("Auto-discovering P2P address via RPC...");
		let (peer_id_str, addresses) = client::discover_p2p_info(control_url).await?;
		tracing::info!("Node peer ID: {peer_id_str}");
		tracing::info!("Node listen addresses: {addresses:?}");

		let raw = addresses
			.iter()
			.find(|a| a.contains("/ws"))
			.or_else(|| addresses.first())
			.map(|a| if a.contains("/p2p/") { a.clone() } else { format!("{a}/p2p/{peer_id_str}") })
			.ok_or_else(|| anyhow::anyhow!("No P2P addresses discovered"))?;
		let cleaned = bitswap::clean_multiaddr(&raw);
		let ma: litep2p::types::multiaddr::Multiaddr = cleaned.parse()?;
		bitswap::BitswapClient::peer_id_from_multiaddr(&ma)?;
		vec![ma]
	};

	for (i, ma) in addrs.iter().enumerate() {
		tracing::info!("P2P peer {i}: {ma}");
	}

	Ok(addrs)
}
