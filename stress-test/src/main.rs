use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use std::{
	path::{Path, PathBuf},
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

	/// Generate an HTML chart of throughput over time
	#[arg(long, global = true)]
	chart: Option<PathBuf>,
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
	/// Run all test suites (block-capacity + bitswap)
	Full,
	/// Run a test plan (YAML file or built-in).
	Plan {
		/// Path to a YAML plan file. If omitted, runs the built-in quick performance test
		/// (1KB 10min, MIXED 10min, 2MB 10min).
		#[arg(long)]
		file: Option<PathBuf>,
	},
	/// Generate charts from an existing JSON results file.
	Chart {
		/// Path to the JSON results file.
		input: PathBuf,
		/// Output HTML file. Defaults to the input path with .html extension.
		#[arg(long)]
		output: Option<PathBuf>,
	},
}

#[tokio::main]
async fn main() -> Result<()> {
	env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

	let mut cli = Cli::parse();

	// Append timestamp to output filenames (e.g. results.json → results_2026-04-20_15h.json).
	let ts = {
		use std::time::SystemTime;
		let secs = SystemTime::now()
			.duration_since(SystemTime::UNIX_EPOCH)
			.unwrap_or_default()
			.as_secs();
		// Simple UTC formatting without chrono: YYYY-MM-DD_HHh
		let days = secs / 86400;
		let h = (secs % 86400) / 3600;
		// Days since epoch → date (good enough approximation using chrono-free math)
		let (y, m, d) = days_to_ymd(days);
		format!("{y:04}-{m:02}-{d:02}_{h:02}h")
	};
	if let Some(ref path) = cli.output_file {
		cli.output_file = Some(append_timestamp(path, &ts));
	}
	if let Some(ref path) = cli.chart {
		cli.chart = Some(append_timestamp(path, &ts));
	}

	// Handle chart-only command early (no RPC needed).
	if let Commands::Chart { ref input, ref output } = cli.command {
		let json = std::fs::read_to_string(input)
			.map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", input.display()))?;
		let results: Vec<report::ScenarioResult> = serde_json::from_str(&json)
			.map_err(|e| anyhow::anyhow!("Failed to parse {}: {e}", input.display()))?;
		let chart_path = output.clone().unwrap_or_else(|| input.with_extension("html"));
		bulletin_stress_test::chart::generate_chart(&results, &chart_path)?;
		return Ok(());
	}

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

	let ws_url_refs: Vec<&str> = ws_urls.iter().map(|s| s.as_str()).collect();

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
				None,
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
		Commands::Plan { ref file } => {
			let test_plan = match file {
				Some(path) => {
					log::info!("Loading plan from {}", path.display());
					match bulletin_stress_test::plan::load_from_file(path) {
						Ok(p) => p,
						Err(e) => {
							log::error!("Failed to load plan: {e}");
							command_error = Some(e);
							bulletin_stress_test::plan::quick_performance_test()
						},
					}
				},
				None => {
					log::info!("Running built-in quick performance test plan");
					bulletin_stress_test::plan::quick_performance_test()
				},
			};

			if command_error.is_none() {
				let total_entries = test_plan.steps.len();
				for (i, entry) in test_plan.steps.iter().enumerate() {
					if cancel.load(Ordering::Relaxed) {
						break;
					}
					log::info!(
						"=== Plan entry {}/{}: {} ===",
						i + 1,
						total_entries,
						entry.description(),
					);

					let err = match entry {
						bulletin_stress_test::plan::PlanEntry::Single(step) =>
							run_plan_step(
								step,
								&client,
								&authorizer_signer,
								&nonce_tracker,
								&cli,
								&chain_limits,
								&ws_url_refs,
								control_url,
								&mut all_results,
								&flush,
								&cancel,
							)
							.await
							.err(),
						bulletin_stress_test::plan::PlanEntry::Parallel { parallel } => {
							let futs: Vec<_> = parallel
								.iter()
								.map(|step| {
									run_parallel_step(
										step.scenario,
										client.clone(),
										authorizer_signer.clone(),
										nonce_tracker.clone(),
										chain_limits.clone(),
										ws_url_refs.iter().map(|s| s.to_string()).collect(),
										control_url.to_string(),
										cancel.clone(),
										step.submitters.unwrap_or(cli.submitters),
										step.target_blocks.unwrap_or(cli.target_blocks),
										step.iteration_blocks.unwrap_or(cli.iteration_blocks),
										step.mix_seed.or(cli.mix_seed),
										step.variants.clone(),
										step.payload_size.unwrap_or(128 * 1024),
									)
								})
								.collect();

							let outcomes = futures::future::join_all(futs).await;
							let mut first_error = None;
							for (results, err) in outcomes {
								all_results.extend(results);
								flush(&mut all_results);
								if let Err(e) = err {
									if first_error.is_none() {
										first_error = Some(e);
									}
								}
							}
							first_error
						},
					};

					if let Some(e) = err {
						log::error!("Plan entry {}/{} failed: {e}", i + 1, total_entries);
						command_error = Some(e);
						break;
					}
				}
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
				None,
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
		},
		Commands::Chart { .. } => unreachable!("handled above"),
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

	// Generate chart: explicit --chart path, or derive from --output-file.
	let chart_path = cli.chart.clone().or_else(|| {
		cli.output_file.as_ref().map(|p| p.with_extension("html"))
	});
	if let Some(ref path) = chart_path {
		if let Err(e) = bulletin_stress_test::chart::generate_chart(&all_results, path) {
			log::error!("Failed to generate chart: {e}");
		}
	}

	if cancel.load(Ordering::Relaxed) {
		flush(&mut all_results);
		if let Some(ref path) = chart_path {
			let _ = bulletin_stress_test::chart::generate_chart(&all_results, path);
		}
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
	on_result: &(dyn Fn(&mut Vec<report::ScenarioResult>) + Send + Sync),
	cancel: &Arc<AtomicBool>,
	overrides: Option<&bulletin_stress_test::plan::PlanStep>,
) -> Result<()> {
	let target_blocks = overrides.and_then(|o| o.target_blocks).unwrap_or(cli.target_blocks);
	let submitters = overrides.and_then(|o| o.submitters).unwrap_or(cli.submitters);
	let iteration_blocks = overrides.and_then(|o| o.iteration_blocks).unwrap_or(cli.iteration_blocks);
	let mix_seed = overrides.and_then(|o| o.mix_seed).or(cli.mix_seed);

	match test {
		"block-capacity" | "all" => {
			scenarios::throughput::run_block_capacity_sweep(
				client,
				authorizer_signer,
				nonce_tracker,
				ws_urls,
				chain_limits,
				submitters,
				target_blocks,
				iteration_blocks,
				variants,
				mix_seed,
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
	on_result: &(dyn Fn(&mut Vec<report::ScenarioResult>) + Send + Sync),
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

/// Execute a single plan step (used by sequential plan execution).
#[allow(clippy::too_many_arguments)]
async fn run_plan_step(
	step: &bulletin_stress_test::plan::PlanStep,
	client: &subxt::OnlineClient<client::BulletinConfig>,
	authorizer_signer: &Keypair,
	nonce_tracker: &accounts::NonceTracker,
	cli: &Cli,
	chain_limits: &ChainLimits,
	ws_urls: &[&str],
	control_url: &str,
	results: &mut Vec<report::ScenarioResult>,
	on_result: &(dyn Fn(&mut Vec<report::ScenarioResult>) + Send + Sync),
	cancel: &Arc<AtomicBool>,
) -> Result<()> {
	match step.scenario {
		bulletin_stress_test::plan::Scenario::Throughput => run_throughput(
			client,
			authorizer_signer,
			nonce_tracker,
			cli,
			"block-capacity",
			step.variants.as_deref(),
			chain_limits,
			ws_urls,
			results,
			on_result,
			cancel,
			Some(step),
		)
		.await,
		bulletin_stress_test::plan::Scenario::Bitswap => run_bitswap(
			client,
			authorizer_signer,
			nonce_tracker,
			cli,
			"b2",
			step.payload_size.unwrap_or(128 * 1024),
			control_url,
			results,
			on_result,
		)
		.await,
	}
}

/// Bitswap execution without CLI dependency (for parallel plan tasks).
async fn run_bitswap_inner(
	client: &subxt::OnlineClient<client::BulletinConfig>,
	authorizer_signer: &Keypair,
	nonce_tracker: &accounts::NonceTracker,
	payload_size: usize,
	control_url: &str,
	results: &mut Vec<report::ScenarioResult>,
) -> Result<()> {
	let multiaddr_str = {
		log::info!("Auto-discovering P2P address via RPC...");
		let (peer_id_str, addresses) = client::discover_p2p_info(control_url).await?;
		let raw = addresses
			.iter()
			.find(|a| a.contains("/ws"))
			.or_else(|| addresses.first())
			.map(|a| if a.contains("/p2p/") { a.clone() } else { format!("{a}/p2p/{peer_id_str}") })
			.ok_or_else(|| anyhow::anyhow!("No P2P addresses discovered"))?;
		bitswap::clean_multiaddr(&raw)
	};
	let multiaddr: litep2p::types::multiaddr::Multiaddr = multiaddr_str.parse()?;
	bitswap::BitswapClient::peer_id_from_multiaddr(&multiaddr)?;

	let rs = scenarios::bitswap_read::run_b2_concurrent_read_sweep(
		client,
		authorizer_signer,
		nonce_tracker,
		&multiaddr,
		512,
		payload_size,
		control_url,
	)
	.await?;
	results.extend(rs);
	Ok(())
}

/// Execute a single step in a parallel group (owns all data, `'static` safe).
#[allow(clippy::too_many_arguments)]
async fn run_parallel_step(
	scenario: bulletin_stress_test::plan::Scenario,
	client: subxt::OnlineClient<client::BulletinConfig>,
	authorizer_signer: Keypair,
	nonce_tracker: accounts::NonceTracker,
	chain_limits: ChainLimits,
	ws_url_refs: Vec<String>,
	control_url: String,
	cancel: Arc<AtomicBool>,
	submitters: usize,
	target_blocks: u32,
	iteration_blocks: u32,
	mix_seed: Option<u64>,
	variants: Option<String>,
	payload_size: usize,
) -> (Vec<report::ScenarioResult>, Result<()>) {
	fn noop(_: &mut Vec<report::ScenarioResult>) {}

	let mut step_results = Vec::new();
	let ws_refs: Vec<&str> = ws_url_refs.iter().map(|s| s.as_str()).collect();
	let err = match scenario {
		bulletin_stress_test::plan::Scenario::Throughput =>
			scenarios::throughput::run_block_capacity_sweep(
				&client,
				&authorizer_signer,
				&nonce_tracker,
				&ws_refs,
				&chain_limits,
				submitters,
				target_blocks,
				iteration_blocks,
				variants.as_deref(),
				mix_seed,
				&mut step_results,
				&(noop as fn(&mut Vec<report::ScenarioResult>)),
				&cancel,
			)
			.await,
		bulletin_stress_test::plan::Scenario::Bitswap =>
			run_bitswap_inner(
				&client,
				&authorizer_signer,
				&nonce_tracker,
				payload_size,
				&control_url,
				&mut step_results,
			)
			.await,
	};
	(step_results, err)
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

/// Insert a timestamp before the file extension: `foo.json` → `foo_2026-04-20_15h.json`.
fn append_timestamp(path: &Path, ts: &str) -> PathBuf {
	let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
	let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("json");
	let new_name = format!("{stem}_{ts}.{ext}");
	path.with_file_name(new_name)
}

/// Convert days since Unix epoch to (year, month, day). No leap-second precision needed.
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
	// Algorithm from http://howardhinnant.github.io/date_algorithms.html
	let z = days + 719468;
	let era = z / 146097;
	let doe = z - era * 146097;
	let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
	let y = yoe + era * 400;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let d = doy - (153 * mp + 2) / 5 + 1;
	let m = if mp < 10 { mp + 3 } else { mp - 9 };
	let y = if m <= 2 { y + 1 } else { y };
	(y, m, d)
}
