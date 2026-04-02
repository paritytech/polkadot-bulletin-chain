use anyhow::{Context, Result};
use bulletin_stress_test::report::ScenarioResult;
use tokio::io::{AsyncBufReadExt, BufReader};

#[allow(dead_code)]
pub struct CliOutput {
	pub results: Vec<ScenarioResult>,
	pub exit_code: i32,
	pub stderr: String,
}

/// Run the `bulletin-stress-test` binary as a subprocess with the given args.
///
/// The binary is located via `env!("CARGO_BIN_EXE_bulletin-stress-test")`, which
/// is set by `cargo test` when building the binary target.
///
/// `ws_url` can be a single URL or comma-separated list for multi-node submission.
///
/// Stderr is streamed in real-time via `log::info!` so progress is visible
/// during long-running tests (e.g. parachain bitswap with 512 items).
pub async fn run_stress_test(ws_url: &str, args: &[&str]) -> Result<CliOutput> {
	let bin = env!("CARGO_BIN_EXE_bulletin-stress-test");

	let mut cmd = tokio::process::Command::new(bin);
	cmd.arg("--ws-url").arg(ws_url).arg("--output").arg("json").args(args);

	// Forward RUST_LOG so the subprocess emits logs to stderr
	if let Ok(log_val) = std::env::var("RUST_LOG") {
		cmd.env("RUST_LOG", log_val);
	}

	log::info!("CLI runner: {bin} --ws-url {ws_url} --output json {}", args.join(" "));

	let mut child = cmd
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.spawn()
		.context("Failed to spawn bulletin-stress-test")?;

	// Stream stderr in real-time so progress is visible during the test
	let stderr_pipe = child.stderr.take().expect("stderr should be piped");
	let stderr_handle = tokio::spawn(async move {
		let reader = BufReader::new(stderr_pipe);
		let mut lines = reader.lines();
		let mut collected = String::new();
		while let Ok(Some(line)) = lines.next_line().await {
			log::info!("CLI: {line}");
			collected.push_str(&line);
			collected.push('\n');
		}
		collected
	});

	let output = child
		.wait_with_output()
		.await
		.context("Failed to wait for bulletin-stress-test")?;

	let stderr = stderr_handle.await.unwrap_or_default();
	let exit_code = output.status.code().unwrap_or(-1);

	if !output.status.success() {
		log::error!("CLI exited with code {exit_code}");
		return Ok(CliOutput { results: vec![], exit_code, stderr });
	}

	let stdout = String::from_utf8_lossy(&output.stdout);

	// Save raw JSON to a file for inspection
	let json_filename = format!("stress-test-results-{}.json", args.join("-"));
	let json_path = std::env::temp_dir().join(&json_filename);
	if let Err(e) = std::fs::write(&json_path, stdout.as_bytes()) {
		log::warn!("Failed to save raw JSON to {}: {e}", json_path.display());
	} else {
		log::info!("Raw JSON results saved to {}", json_path.display());
	}

	let results: Vec<ScenarioResult> = serde_json::from_str(&stdout).with_context(|| {
		format!("Failed to parse CLI JSON output: {}", &stdout[..stdout.len().min(500)])
	})?;

	Ok(CliOutput { results, exit_code, stderr })
}
