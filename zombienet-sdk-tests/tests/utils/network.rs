// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Network configuration builders, binary path resolution, and binary verification.

use super::config::*;
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use zombienet_sdk::{LocalFileSystem, Network, NetworkConfig, NetworkConfigBuilder};

pub async fn initialize_network(
	config: NetworkConfig,
) -> Result<Network<LocalFileSystem>, anyhow::Error> {
	let spawn_fn = zombienet_sdk::environment::get_spawn_fn();
	let network = spawn_fn(config).await?;
	network.detach().await;
	Ok(network)
}

pub fn env_or_default(var: &str, default: &str) -> String {
	std::env::var(var).unwrap_or_else(|_| default.to_string())
}

/// Insert `--database=<backend>` (from [`DB_BACKEND_ENV`]) into parachain node args,
/// before the `--` embedded-relay separator. No-op when the env var is unset or empty.
pub fn with_db_backend<T>(mut args: Vec<T>) -> Vec<T>
where
	T: for<'a> From<&'a str> + PartialEq,
{
	if let Ok(backend) = std::env::var(DB_BACKEND_ENV) {
		if !backend.is_empty() {
			let separator = T::from("--");
			let pos = args.iter().position(|a| *a == separator).unwrap_or(args.len());
			args.insert(pos, T::from(format!("--database={backend}").as_str()));
		}
	}
	args
}

pub fn get_relay_binary_path() -> String {
	let path_str = env_or_default(RELAY_BINARY_PATH_ENV, DEFAULT_RELAY_BINARY);
	resolve_binary_path(&path_str)
}

pub fn get_parachain_binary_path() -> String {
	let path_str = env_or_default(PARACHAIN_BINARY_PATH_ENV, DEFAULT_PARACHAIN_BINARY);
	resolve_binary_path(&path_str)
}

pub fn get_parachain_chain_spec() -> String {
	let path_str = env_or_default(PARACHAIN_CHAIN_SPEC_ENV, DEFAULT_PARACHAIN_CHAIN_SPEC);
	resolve_binary_path(&path_str)
}

pub fn get_relay_chain() -> String {
	env_or_default(RELAY_CHAIN_ENV, DEFAULT_RELAY_CHAIN)
}

pub fn get_para_id() -> u32 {
	std::env::var(PARA_ID_ENV)
		.ok()
		.and_then(|v| v.parse().ok())
		.unwrap_or(DEFAULT_PARA_ID)
}

pub fn get_parachain_chain_id() -> String {
	env_or_default(PARACHAIN_CHAIN_ID_ENV, DEFAULT_PARACHAIN_CHAIN_ID)
}

fn resolve_binary_path(path_str: &str) -> String {
	let path = PathBuf::from(path_str);
	if path.is_absolute() {
		path_str.to_string()
	} else {
		std::env::current_dir()
			.map(|cwd| cwd.join(&path))
			.and_then(|p| p.canonicalize())
			.map(|p| p.to_string_lossy().to_string())
			.unwrap_or_else(|_| path_str.to_string())
	}
}

fn require_env_var(env_var: &str) -> Result<String> {
	std::env::var(env_var).map_err(|_| anyhow!("{} env var is not set", env_var))
}

fn verify_binary(path: &str) -> Result<()> {
	let output = std::process::Command::new(path)
		.arg("--version")
		.output()
		.context(format!("Failed to execute '{}'", path))?;
	if !output.status.success() {
		anyhow::bail!("'{}' exited with status: {}", path, output.status);
	}
	Ok(())
}

pub fn verify_ldb_tool() -> Result<String> {
	let ldb_path = require_env_var(LDB_PATH_ENV)?;
	std::process::Command::new(&ldb_path)
		.arg("--help")
		.output()
		.context(format!("Failed to execute '{}' (set via {})", ldb_path, LDB_PATH_ENV))?;
	Ok(ldb_path)
}

pub fn verify_parachain_binaries() -> Result<()> {
	let relay_binary = get_relay_binary_path();
	tracing::info!("Relay binary: {}", relay_binary);
	verify_binary(&relay_binary)
		.context(format!("Relay binary '{}' ({})", relay_binary, RELAY_BINARY_PATH_ENV))?;

	let para_binary = get_parachain_binary_path();
	tracing::info!("Parachain binary: {}", para_binary);
	verify_binary(&para_binary)
		.context(format!("Parachain binary '{}' ({})", para_binary, PARACHAIN_BINARY_PATH_ENV))?;

	let chain_spec = get_parachain_chain_spec();
	tracing::info!("Chain spec: {}", chain_spec);
	if !PathBuf::from(&chain_spec).exists() {
		anyhow::bail!(
			"Chain spec not found at '{}' (set {} to override)",
			chain_spec,
			PARACHAIN_CHAIN_SPEC_ENV
		);
	}

	Ok(())
}

/// Parachain network: 3 relay validators (for stable GRANDPA finality) + 1 collator.
pub fn build_parachain_network_config_three_relay_validators(
	para_node_args: Vec<String>,
) -> Result<NetworkConfig> {
	let relay_binary = get_relay_binary_path();
	let para_binary = get_parachain_binary_path();
	let para_chain_spec = get_parachain_chain_spec();

	tracing::info!("Relay binary: {}", relay_binary);
	tracing::info!("Parachain binary: {}", para_binary);
	tracing::info!("Parachain chain spec: {}", para_chain_spec);

	let relay_args: Vec<_> = vec!["-lruntime=debug"].into_iter().map(|s| s.into()).collect();
	let relay_args2 = relay_args.clone();
	let relay_args3 = relay_args.clone();

	let para_node_args = with_db_backend(para_node_args);
	let para_args: Vec<_> = para_node_args.iter().map(|s| s.as_str().into()).collect();

	let relay_chain = get_relay_chain();
	let para_id = get_para_id();
	tracing::info!("Relay chain: {}", relay_chain);
	tracing::info!("Parachain ID: {}", para_id);

	NetworkConfigBuilder::new()
		.with_relaychain(|relaychain| {
			relaychain
				.with_chain(relay_chain.as_str())
				.with_default_command(relay_binary.as_str())
				.with_node(|node| node.with_name("alice").validator(true).with_args(relay_args))
				.with_node(|node| node.with_name("bob").validator(true).with_args(relay_args2))
				.with_node(|node| node.with_name("charlie").validator(true).with_args(relay_args3))
		})
		.with_parachain(|parachain| {
			parachain
				.with_id(para_id)
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
			let message = errs.into_iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
			anyhow!("config errs: {message}")
		})
}

/// 3 relay validators + 3 collators, all collators using the same `para_node_args`.
pub fn build_parachain_network_config_three_collators(
	para_node_args: Vec<String>,
) -> Result<NetworkConfig> {
	let relay_binary = get_relay_binary_path();
	let para_binary = get_parachain_binary_path();
	let para_chain_spec = get_parachain_chain_spec();

	let relay_args: Vec<_> = vec!["-lruntime=debug"].into_iter().map(|s| s.into()).collect();
	let relay_args2 = relay_args.clone();
	let relay_args3 = relay_args.clone();

	let para_node_args = with_db_backend(para_node_args);
	let para_args: Vec<_> = para_node_args.iter().map(|s| s.as_str().into()).collect();
	let para_args2 = para_args.clone();
	let para_args3 = para_args.clone();

	let relay_chain = get_relay_chain();
	let para_id = get_para_id();

	NetworkConfigBuilder::new()
		.with_relaychain(|relaychain| {
			relaychain
				.with_chain(relay_chain.as_str())
				.with_default_command(relay_binary.as_str())
				.with_node(|node| node.with_name("alice").validator(true).with_args(relay_args))
				.with_node(|node| node.with_name("bob").validator(true).with_args(relay_args2))
				.with_node(|node| node.with_name("charlie").validator(true).with_args(relay_args3))
		})
		.with_parachain(|parachain| {
			parachain
				.with_id(para_id)
				.with_chain_spec_path(para_chain_spec.as_str())
				.cumulus_based(true)
				.with_collator(|c| {
					c.with_name("collator-1")
						.validator(true)
						.with_command(para_binary.as_str())
						.with_args(para_args)
				})
				.with_collator(|c| {
					c.with_name("collator-2")
						.validator(true)
						.with_command(para_binary.as_str())
						.with_args(para_args2)
				})
				.with_collator(|c| {
					c.with_name("collator-3")
						.validator(true)
						.with_command(para_binary.as_str())
						.with_args(para_args3)
				})
		})
		.with_global_settings(|gs| match std::env::var("ZOMBIENET_SDK_BASE_DIR") {
			Ok(val) => gs.with_base_dir(val),
			_ => gs,
		})
		.build()
		.map_err(|errs| {
			let message = errs.into_iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
			anyhow!("config errs: {message}")
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	// Single test: it mutates the process-wide env var.
	#[test]
	fn with_db_backend_inserts_before_relay_separator() {
		std::env::set_var(DB_BACKEND_ENV, "paritydb");
		let args: Vec<String> =
			vec!["--ipfs-server".into(), "--".into(), "--network-backend=libp2p".into()];
		assert_eq!(
			with_db_backend(args),
			vec!["--ipfs-server", "--database=paritydb", "--", "--network-backend=libp2p"]
		);

		let no_separator: Vec<String> = vec!["--sync=warp".into()];
		assert_eq!(with_db_backend(no_separator), vec!["--sync=warp", "--database=paritydb"]);

		std::env::set_var(DB_BACKEND_ENV, "");
		let args: Vec<String> = vec!["--ipfs-server".into()];
		assert_eq!(with_db_backend(args), vec!["--ipfs-server"]);
		std::env::remove_var(DB_BACKEND_ENV);
	}
}
