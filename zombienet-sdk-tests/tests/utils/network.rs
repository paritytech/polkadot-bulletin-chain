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

pub fn get_binary_path() -> String {
	let path_str = env_or_default(BINARY_PATH_ENV, DEFAULT_BINARY);
	resolve_binary_path(&path_str)
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
	log::info!("Relay binary: {}", relay_binary);
	verify_binary(&relay_binary)
		.context(format!("Relay binary '{}' ({})", relay_binary, RELAY_BINARY_PATH_ENV))?;

	let para_binary = get_parachain_binary_path();
	log::info!("Parachain binary: {}", para_binary);
	verify_binary(&para_binary)
		.context(format!("Parachain binary '{}' ({})", para_binary, PARACHAIN_BINARY_PATH_ENV))?;

	let chain_spec = get_parachain_chain_spec();
	log::info!("Chain spec: {}", chain_spec);
	if !PathBuf::from(&chain_spec).exists() {
		anyhow::bail!(
			"Chain spec not found at '{}' (set {} to override)",
			chain_spec,
			PARACHAIN_CHAIN_SPEC_ENV
		);
	}

	Ok(())
}

pub fn verify_solo_binary() -> Result<()> {
	let binary = get_binary_path();
	log::info!("Solo chain binary: {}", binary);
	verify_binary(&binary)
		.context(format!("Solo chain binary '{}' ({})", binary, BINARY_PATH_ENV))?;
	Ok(())
}

/// Single-node network with Alice as validator using bulletin-polkadot-local chain spec.
pub fn build_single_node_network_config(node_args: Vec<String>) -> Result<NetworkConfig> {
	let binary_path = get_binary_path();
	log::info!("Using binary: {}", binary_path);

	let args: Vec<_> = node_args.iter().map(|s| s.as_str().into()).collect();

	NetworkConfigBuilder::new()
		.with_relaychain(|relaychain| {
			relaychain
				.with_chain("bulletin-polkadot-local")
				.with_default_command(binary_path.as_str())
				.with_node(|node| node.with_name("alice").validator(true).with_args(args))
		})
		.with_global_settings(|global_settings| {
			if let Ok(val) = std::env::var("ZOMBIENET_SDK_BASE_DIR") {
				global_settings.with_base_dir(val)
			} else {
				global_settings
			}
		})
		.build()
		.map_err(|errs| {
			let message = errs.into_iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
			anyhow!("config errs: {message}")
		})
}

/// 3-validator network (Alice, Bob, Dave) - required for warp sync (needs 3 peers minimum).
pub fn build_three_node_network_config(node_args: Vec<String>) -> Result<NetworkConfig> {
	let binary_path = get_binary_path();
	log::info!("Using binary: {}", binary_path);

	let args: Vec<_> = node_args.iter().map(|s| s.as_str().into()).collect();
	let args_clone1 = args.clone();
	let args_clone2 = args.clone();

	NetworkConfigBuilder::new()
		.with_relaychain(|relaychain| {
			relaychain
				.with_chain("bulletin-polkadot-local")
				.with_default_command(binary_path.as_str())
				.with_node(|node| node.with_name("alice").validator(true).with_args(args))
				.with_node(|node| node.with_name("bob").validator(true).with_args(args_clone1))
				.with_node(|node| node.with_name("dave").validator(true).with_args(args_clone2))
		})
		.with_global_settings(|global_settings| {
			if let Ok(val) = std::env::var("ZOMBIENET_SDK_BASE_DIR") {
				global_settings.with_base_dir(val)
			} else {
				global_settings
			}
		})
		.build()
		.map_err(|errs| {
			let message = errs.into_iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
			anyhow!("config errs: {message}")
		})
}

/// Parachain network: 2 relay validators (alice, bob) + 1 collator.
pub fn build_parachain_network_config_single_collator(
	para_node_args: Vec<String>,
) -> Result<NetworkConfig> {
	let relay_binary = get_relay_binary_path();
	let para_binary = get_parachain_binary_path();
	let para_chain_spec = get_parachain_chain_spec();

	log::info!("Relay binary: {}", relay_binary);
	log::info!("Parachain binary: {}", para_binary);
	log::info!("Parachain chain spec: {}", para_chain_spec);

	let relay_args: Vec<_> = vec!["-lruntime=debug"].into_iter().map(|s| s.into()).collect();
	let relay_args2 = relay_args.clone();

	let para_args: Vec<_> = para_node_args.iter().map(|s| s.as_str().into()).collect();

	let relay_chain = get_relay_chain();
	let para_id = get_para_id();
	log::info!("Relay chain: {}", relay_chain);
	log::info!("Parachain ID: {}", para_id);

	NetworkConfigBuilder::new()
		.with_relaychain(|relaychain| {
			relaychain
				.with_chain(relay_chain.as_str())
				.with_default_command(relay_binary.as_str())
				.with_node(|node| node.with_name("alice").validator(true).with_args(relay_args))
				.with_node(|node| node.with_name("bob").validator(true).with_args(relay_args2))
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

/// Parachain network: 3 relay validators (for stable GRANDPA finality) + 1 collator.
pub fn build_parachain_network_config_three_relay_validators(
	para_node_args: Vec<String>,
) -> Result<NetworkConfig> {
	let relay_binary = get_relay_binary_path();
	let para_binary = get_parachain_binary_path();
	let para_chain_spec = get_parachain_chain_spec();

	log::info!("Relay binary: {}", relay_binary);
	log::info!("Parachain binary: {}", para_binary);
	log::info!("Parachain chain spec: {}", para_chain_spec);

	let relay_args: Vec<_> = vec!["-lruntime=debug"].into_iter().map(|s| s.into()).collect();
	let relay_args2 = relay_args.clone();
	let relay_args3 = relay_args.clone();

	let para_args: Vec<_> = para_node_args.iter().map(|s| s.as_str().into()).collect();

	let relay_chain = get_relay_chain();
	let para_id = get_para_id();
	log::info!("Relay chain: {}", relay_chain);
	log::info!("Parachain ID: {}", para_id);

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
