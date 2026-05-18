//! Assign extra coretime cores to a parachain via sudo on the relay chain.
//! Usage: assign_cores [relay_ws_url] [para_id] [num_extra_cores]

use anyhow::Result;
use subxt::{
	config::substrate::SubstrateConfig, dynamic::tx, ext::scale_value::Value, OnlineClient,
};

#[tokio::main]
async fn main() -> Result<()> {
	tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.init();

	let args: Vec<String> = std::env::args().collect();
	let relay_url = args.get(1).map(|s| s.as_str()).unwrap_or("ws://127.0.0.1:9942");
	let para_id: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2487);
	let num_cores: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);

	tracing::info!("Connecting to relay at {relay_url}...");
	let client = OnlineClient::<SubstrateConfig>::from_url(relay_url).await?;
	let alice = subxt_signer::sr25519::dev::alice();

	for core in 0..num_cores {
		let assign_call = tx(
			"Coretime",
			"assign_core",
			vec![
				Value::u128(core as u128),
				Value::u128(0),
				Value::unnamed_composite([Value::unnamed_composite([
					Value::named_variant("Task", [("0".to_string(), Value::u128(para_id as u128))]),
					Value::u128(57600),
				])]),
				Value::unnamed_variant("None", []),
			],
		);
		let sudo_call = tx("Sudo", "sudo", vec![assign_call.into_value()]);

		tracing::info!("Assigning core {core} to para {para_id}...");
		client
			.tx()
			.sign_and_submit_then_watch_default(&sudo_call, &alice)
			.await?
			.wait_for_finalized_success()
			.await?;
		tracing::info!("Core {core} assigned!");
	}

	tracing::info!("Done! {num_cores} extra cores assigned to para {para_id}");
	Ok(())
}
