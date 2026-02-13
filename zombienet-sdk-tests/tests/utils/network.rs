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

pub fn get_wasm_path(env_var: &str, default: &str) -> String {
	let path_str = env_or_default(env_var, default);
	resolve_binary_path(&path_str)
}

fn resolve_binary_path(path_str: &str) -> String {
	let path = PathBuf::from(path_str);
	if path.is_absolute() {
		return path_str.to_string();
	}
	// Try current directory first
	if let Ok(resolved) = std::env::current_dir()
		.map(|cwd| cwd.join(&path))
		.and_then(|p| p.canonicalize())
	{
		return resolved.to_string_lossy().to_string();
	}
	// Try CARGO_MANIFEST_DIR (workspace root during cargo test)
	if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
		let from_manifest = PathBuf::from(&manifest_dir).join("..").join(&path);
		if let Ok(resolved) = from_manifest.canonicalize() {
			return resolved.to_string_lossy().to_string();
		}
		// Also try directly from manifest dir
		let from_manifest_direct = PathBuf::from(&manifest_dir).join(&path);
		if let Ok(resolved) = from_manifest_direct.canonicalize() {
			return resolved.to_string_lossy().to_string();
		}
	}
	path_str.to_string()
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

pub fn verify_wasm_files() -> Result<()> {
	for (env_var, default, label) in [
		(OLD_RUNTIME_WASM_ENV, DEFAULT_OLD_RUNTIME_WASM, "Old runtime"),
		(BROKEN_RUNTIME_WASM_ENV, DEFAULT_BROKEN_RUNTIME_WASM, "Broken runtime"),
		(FIX_RUNTIME_WASM_ENV, DEFAULT_FIX_RUNTIME_WASM, "Fix runtime"),
	] {
		let path = get_wasm_path(env_var, default);
		log::info!("{} WASM: {}", label, path);
		if !PathBuf::from(&path).exists() {
			anyhow::bail!("{} WASM not found at '{}' (set {} to override)", label, path, env_var);
		}
	}
	Ok(())
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
		.with_global_settings(|gs| {
			let base_dir = std::env::var("ZOMBIENET_SDK_BASE_DIR")
				.unwrap_or_else(|_| format!("/tmp/zombienet-test-{}", std::process::id()));
			gs.with_base_dir(base_dir)
		})
		.build()
		.map_err(|errs| {
			let message = errs.into_iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ");
			anyhow!("config errs: {message}")
		})
}
