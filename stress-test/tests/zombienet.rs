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
		&["throughput", "block-capacity", "--variants", variant],
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

	let network = get_parachain_network().await?;
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
