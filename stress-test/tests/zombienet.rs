mod zombienet_common;

use anyhow::Result;
use bulletin_stress_test::report::ScenarioResult;
use std::sync::Arc;
use tokio::sync::OnceCell;
use zombienet_common::expectations::Expectation;
use zombienet_sdk::LocalFileSystem;

/// Shared parachain network — spawned once, reused across all throughput variant tests.
/// Using `--test-threads=1` ensures tests run sequentially on the same network.
static PARACHAIN_NETWORK: OnceCell<Arc<zombienet_sdk::Network<LocalFileSystem>>> =
	OnceCell::const_new();

async fn get_parachain_network() -> Result<Arc<zombienet_sdk::Network<LocalFileSystem>>> {
	let network = PARACHAIN_NETWORK
		.get_or_try_init(|| async {
			let n = zombienet_common::spawn_parachain_network_multi_node().await?;
			Ok::<_, anyhow::Error>(Arc::new(n))
		})
		.await?;
	Ok(network.clone())
}

/// Run a single throughput variant against the shared parachain network.
async fn run_variant(variant: &str, expectation: &Expectation) -> Result<()> {
	let _ = tracing_subscriber::fmt().with_env_filter("info").try_init();

	let network = get_parachain_network().await?;
	let rpc1_url = zombienet_common::get_node_ws_url(&network, "rpc-1")?;
	let rpc2_url = zombienet_common::get_node_ws_url(&network, "rpc-2")?;
	let ws_urls = format!("{rpc1_url},{rpc2_url}");

	let output = zombienet_common::cli_runner::run_stress_test(
		&ws_urls,
		&[
			"--target-blocks",
			"20",
			"--submitters",
			"16",
			"throughput",
			"block-capacity",
			"--variants",
			variant,
		],
	)
	.await?;

	assert_eq!(output.exit_code, 0, "CLI should exit successfully for variant {variant}");

	validate_single_result(&output.results, expectation, "parachain");

	Ok(())
}

/// Validate a single variant result against its expectation.
fn validate_single_result(results: &[ScenarioResult], exp: &Expectation, chain: &str) {
	let result = results.iter().find(|r| r.payload_size == exp.payload_size);

	match (exp.expected, result) {
		(Some((min_avg, min_peak)), Some(r)) => {
			assert!(r.total_confirmed > 0, "{chain} [{}]: confirmed=0, expected > 0", exp.label);
			assert!(
				r.avg_tx_per_block >= min_avg,
				"{chain} [{}]: avg={:.1}, expected >= {:.1}",
				exp.label,
				r.avg_tx_per_block,
				min_avg
			);
			assert!(
				r.peak_tx_per_block >= min_peak,
				"{chain} [{}]: peak={}, expected >= {}",
				exp.label,
				r.peak_tx_per_block,
				min_peak
			);
			tracing::info!(
				"{chain} [{}]: PASS — avg={:.1}, peak={}, confirmed={}",
				exp.label,
				r.avg_tx_per_block,
				r.peak_tx_per_block,
				r.total_confirmed,
			);
		},
		(Some(_), None) => {
			panic!("{chain} [{}]: no result found (expected success)", exp.label);
		},
		(None, Some(r)) if r.total_confirmed > 0 => {
			tracing::info!(
				"{chain} [{}]: UNEXPECTED SUCCESS — confirmed={}, avg={:.1}, peak={}",
				exp.label,
				r.total_confirmed,
				r.avg_tx_per_block,
				r.peak_tx_per_block,
			);
		},
		(None, Some(_)) => {
			tracing::info!("{chain} [{}]: expected rejection, confirmed=0 — OK", exp.label);
		},
		(None, None) => {
			tracing::info!("{chain} [{}]: expected rejection, no result — OK", exp.label);
		},
	}
}

// ============================================================================
// Parachain throughput tests — one per payload size variant.
// All share a single zombienet network (spawned on first use).
// MUST run with --test-threads=1.
// ============================================================================

macro_rules! throughput_variant_test {
	($test_name:ident, $variant:expr, $idx:expr) => {
		#[tokio::test(flavor = "multi_thread")]
		async fn $test_name() -> Result<()> {
			let exp = &zombienet_common::expectations::parachain::EXPECTATIONS[$idx];
			assert_eq!(exp.label, $variant, "expectation index mismatch");
			run_variant($variant, exp).await
		}
	};
}

throughput_variant_test!(test_parachain_throughput_1kb, "1KB", 0);
throughput_variant_test!(test_parachain_throughput_4kb, "4KB", 1);
throughput_variant_test!(test_parachain_throughput_32kb, "32KB", 2);
throughput_variant_test!(test_parachain_throughput_128kb, "128KB", 3);
throughput_variant_test!(test_parachain_throughput_512kb, "512KB", 4);
throughput_variant_test!(test_parachain_throughput_1mb, "1MB", 5);
throughput_variant_test!(test_parachain_throughput_2mb, "2MB", 6);
throughput_variant_test!(test_parachain_throughput_2050kb, "2050KB", 7);
throughput_variant_test!(test_parachain_throughput_4mb, "4MB", 8);
throughput_variant_test!(test_parachain_throughput_5mb, "5MB", 9);
throughput_variant_test!(test_parachain_throughput_7mb, "7MB", 10);
throughput_variant_test!(test_parachain_throughput_7_5mb, "7.5MB", 11);
throughput_variant_test!(test_parachain_throughput_8mb, "8MB", 12);
throughput_variant_test!(test_parachain_throughput_10mb, "10MB", 13);

// ============================================================================
// Sequential nonce upload test
// ============================================================================

/// Single-core network (no elastic scaling, no assign_cores).
static SINGLE_CORE_NETWORK: OnceCell<Arc<zombienet_sdk::Network<LocalFileSystem>>> =
	OnceCell::const_new();

async fn get_single_core_network() -> Result<Arc<zombienet_sdk::Network<LocalFileSystem>>> {
	let network = SINGLE_CORE_NETWORK
		.get_or_try_init(|| async {
			let n = zombienet_common::spawn_single_core_network().await?;
			Ok::<_, anyhow::Error>(Arc::new(n))
		})
		.await?;
	Ok(network.clone())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_parachain_sequential_upload() -> Result<()> {
	let _ = tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.try_init();

	let network = get_single_core_network().await?;
	let rpc1_url = zombienet_common::get_node_ws_url(&network, "rpc-1")?;
	let rpc2_url = zombienet_common::get_node_ws_url(&network, "rpc-2")?;
	let ws_urls = format!("{rpc1_url},{rpc2_url}");

	let output = zombienet_common::cli_runner::run_stress_test(
		&ws_urls,
		&["--target-blocks", "5", "throughput", "sequential-upload"],
	)
	.await?;

	assert_eq!(output.exit_code, 0, "CLI should exit successfully");
	assert!(!output.results.is_empty(), "Should have at least 1 result");

	let result = &output.results[0];
	let expected_txs = (20 * 1024 * 1024) / (32 * 1024); // 640

	// ALL txs must be confirmed — no partial success.
	assert_eq!(
		result.total_confirmed, expected_txs as u64,
		"sequential-upload: confirmed={}, expected ALL {} txs",
		result.total_confirmed, expected_txs
	);

	// Timing: 10MB / ~8MB per block = 2 blocks × 6s = 12s.
	// Allow generous 60s for authorization + signing + finalization overhead.
	assert!(
		result.duration.as_secs() <= 300,
		"sequential-upload took {}s, expected < 300s",
		result.duration.as_secs()
	);

	let gap_repairs = result.submission_stats.as_ref().and_then(|s| s.gap_repairs).unwrap_or(0);
	tracing::info!(
		"sequential-upload: PASS — confirmed={}/{}, tps={:.1}, duration={:.1}s, gap_repairs={}",
		result.total_confirmed,
		expected_txs,
		result.throughput_tps,
		result.duration.as_secs_f64(),
		gap_repairs,
	);

	Ok(())
}

// ============================================================================
// Bitswap read tests (B2: concurrent multi-client)
// ============================================================================

fn validate_bitswap_results(results: &[ScenarioResult], chain: &str) {
	assert!(!results.is_empty(), "{chain}: bitswap test returned no results");

	let mut failures = Vec::new();

	for r in results {
		let successful = r.successful_reads.unwrap_or(0);
		let failed = r.failed_reads.unwrap_or(0);
		let rps = r.reads_per_sec.unwrap_or(0.0);
		let verified = r.data_verified.unwrap_or(false);

		if successful == 0 {
			failures.push(format!("  [{}]: successful_reads=0", r.name));
		}
		if failed != 0 {
			failures.push(format!("  [{}]: failed_reads={failed}, expected 0", r.name));
		}
		if rps <= 0.0 {
			failures.push(format!("  [{}]: reads_per_sec={rps:.1}, expected > 0", r.name));
		}
		if !verified {
			failures.push(format!("  [{}]: data_verified=false", r.name));
		}

		let status = if successful > 0 && failed == 0 && verified { "PASS" } else { "FAIL" };
		tracing::info!(
			"{chain} [{name}]: {status} — reads={successful}/{total}, rps={rps:.1}, verified={verified}",
			name = r.name,
			total = r.total_reads.unwrap_or(0),
		);
	}

	assert!(
		failures.is_empty(),
		"{chain} bitswap read validation failures:\n{}",
		failures.join("\n")
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_parachain_bitswap_read() -> Result<()> {
	let _ = tracing_subscriber::fmt().with_env_filter("info").try_init();

	let network = get_single_core_network().await?;
	// Bitswap reads go to a collator (has --ipfs-server); store txs go via RPC nodes.
	let ws_url = zombienet_common::get_node_ws_url(&network, "collator-1")?;

	let output = zombienet_common::cli_runner::run_stress_test(
		&ws_url,
		&["bitswap", "b2", "--iterations", "128"],
	)
	.await?;

	assert_eq!(output.exit_code, 0, "CLI should exit successfully");
	assert_eq!(output.results.len(), 7, "Expected 7 results (concurrency 1..64)");
	validate_bitswap_results(&output.results, "parachain");

	Ok(())
}

// ============================================================================
// Bulk Bitswap read test (upload first, then read)
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_parachain_bitswap_bulk_read() -> Result<()> {
	let _ = tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.try_init();

	let network = get_single_core_network().await?;
	let rpc1_url = zombienet_common::get_node_ws_url(&network, "rpc-1")?;
	let collator_url = zombienet_common::get_node_ws_url(&network, "collator-1")?;

	// Phase 1: Upload ~100MB with mixed payload sizes via block-capacity.
	let rpc2_url = zombienet_common::get_node_ws_url(&network, "rpc-2")?;
	let ws_urls = format!("{rpc1_url},{rpc2_url}");
	tracing::info!("Phase 1: uploading ~100MB of mixed-size data...");
	let upload_output = zombienet_common::cli_runner::run_stress_test(
		&ws_urls,
		&[
			"--submitters", "8",
			"--target-blocks", "15",
			"throughput",
			"block-capacity",
			"--variants", "32KB,128KB,1MB",
		],
	)
	.await?;
	assert_eq!(
		upload_output.exit_code, 0,
		"Upload should succeed (exit code {})",
		upload_output.exit_code
	);
	tracing::info!("Phase 1 complete: data uploaded");

	// Phase 2: Read the data back via Bitswap bulk-read.
	// Discover the collator's P2P address and rewrite to 127.0.0.1
	// (the node reports its LAN IP which doesn't work from a subprocess).
	let (peer_id, addresses) =
		bulletin_stress_test::client::discover_p2p_info(&collator_url).await?;
	let p2p_port = addresses
		.iter()
		.find_map(|a| {
			// Parse "/ip4/.../tcp/PORT/ws" and extract PORT
			let parts: Vec<&str> = a.split('/').collect();
			parts.iter().position(|&p| p == "tcp").and_then(|i| parts.get(i + 1))
				.and_then(|p| p.parse::<u16>().ok())
		})
		.ok_or_else(|| anyhow::anyhow!("No TCP port in collator P2P addresses: {addresses:?}"))?;
	let p2p_multiaddr = format!("/ip4/127.0.0.1/tcp/{p2p_port}/ws/p2p/{peer_id}");
	tracing::info!("Phase 2: reading data via Bitswap at {p2p_multiaddr}...");

	let read_output = zombienet_common::cli_runner::run_stress_test(
		&collator_url,
		&[
			"--p2p-multiaddr",
			&p2p_multiaddr,
			"bitswap",
			"bulk-read",
			"--read-size",
			"1073741824",  // 1 GB round-robin
			"--read-concurrency",
			"16",
		],
	)
	.await?;
	assert_eq!(
		read_output.exit_code, 0,
		"Bulk read should succeed (exit code {})",
		read_output.exit_code
	);
	assert!(!read_output.results.is_empty(), "Should have at least 1 result");

	let result = &read_output.results[0];
	let successful = result.successful_reads.unwrap_or(0);
	let total = result.total_reads.unwrap_or(0);
	let reads_per_sec = result.reads_per_sec.unwrap_or(0.0);
	let mb_per_sec = result.read_bytes_per_sec.unwrap_or(0.0) / 1048576.0;

	tracing::info!(
		"Bulk read results:\n  \
		 Reads: {successful}/{total}\n  \
		 Throughput: {reads_per_sec:.1} reads/s, {mb_per_sec:.1} MB/s\n  \
		 Duration: {:.1}s",
		result.duration.as_secs_f64(),
	);

	assert_eq!(
		successful, total,
		"All reads must succeed: {successful}/{total}"
	);

	Ok(())
}

// ============================================================================
// Renew stress test
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_parachain_renew_stress() -> Result<()> {
	let _ = tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.try_init();

	let network = get_single_core_network().await?;
	let rpc1_url = zombienet_common::get_node_ws_url(&network, "rpc-1")?;

	// Upload 520 items (32KB each), then renew for 5 blocks.
	let output = zombienet_common::cli_runner::run_stress_test(
		&rpc1_url,
		&[
			"renew",
			"--store-count", "520",
			"--chunk-size", "32768",
			"--target-blocks", "5",
		],
	)
	.await?;

	assert_eq!(
		output.exit_code, 0,
		"Renew stress should succeed (exit code {})",
		output.exit_code
	);
	assert!(!output.results.is_empty(), "Should have at least 1 result");

	let result = &output.results[0];
	tracing::info!(
		"Renew stress: confirmed={}/{}, tps={:.1}, duration={:.1}s, avg={:.1} tx/block",
		result.total_confirmed,
		result.total_submitted,
		result.throughput_tps,
		result.duration.as_secs_f64(),
		result.avg_tx_per_block,
	);

	// At least some renewals should succeed.
	assert!(
		result.total_confirmed > 0,
		"Should have confirmed renewals"
	);

	Ok(())
}
