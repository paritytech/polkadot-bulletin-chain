use crate::{
	benchmarking::inherent_benchmark_data,
	chain_spec,
	cli::{Cli, Subcommand},
	node_primitives::Block,
	service,
};
use frame_benchmarking_cli::{
	BenchmarkCmd, ExtrinsicFactory, SubstrateRemarkBuilder, SUBSTRATE_REFERENCE_HARDWARE,
};
use sc_cli::SubstrateCli;
use sc_network::config::NetworkBackendType;
use sc_service::PartialComponents;
use std::{sync::Arc, time::Duration};

/// Log target for this file.
const LOG_TARGET: &str = "command";

impl SubstrateCli for Cli {
	fn impl_name() -> String {
		"Polkadot Bulletin Chain Node".into()
	}

	fn impl_version() -> String {
		env!("SUBSTRATE_CLI_IMPL_VERSION").into()
	}

	fn description() -> String {
		env!("CARGO_PKG_DESCRIPTION").into()
	}

	fn author() -> String {
		env!("CARGO_PKG_AUTHORS").into()
	}

	fn support_url() -> String {
		"support.anonymous.an".into()
	}

	fn copyright_start_year() -> i32 {
		2017
	}

	fn load_spec(&self, id: &str) -> Result<Box<dyn sc_service::ChainSpec>, String> {
		Ok(match id {
			// TODO: put behind feature and remove dependencies
			"dev" | "rococo-dev" => Box::new(chain_spec::rococo_development_config()?),
			"local" | "rococo-local" => Box::new(chain_spec::rococo_local_testnet_config()?),
			"polkadot-dev" | "bulletin-polkadot-dev" => Box::new(chain_spec::bulletin_polkadot_development_config()?),
			"polkadot-local" | "bulletin-polkadot-local" => Box::new(chain_spec::bulletin_polkadot_local_testnet_config()?),
			"bulletin-polkadot" => Box::new(chain_spec::bulletin_polkadot_config()?),
			"" => return Err(
				"No chain_id or path specified! Either provide a path to the chain spec or specify chain_id: \
				Polkadot Live (bulletin-polkadot) \
				or Polkadot Dev/Local (bulletin-polkadot-dev, bulletin-polkadot-local) \
				or Rococo (dev, local, rococo-dev, rococo-local)"
					.into(),
			),
			path =>
				Box::new(chain_spec::ChainSpec::from_json_file(std::path::PathBuf::from(path))?),
		})
	}
}

/// Parse and run command line arguments
pub fn run() -> sc_cli::Result<()> {
	let cli = Cli::from_args();

	match &cli.subcommand {
		Some(Subcommand::Key(cmd)) => cmd.run(&cli),
		Some(Subcommand::BuildSpec(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.sync_run(|config| cmd.run(config.chain_spec, config.network))
		},
		Some(Subcommand::CheckBlock(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, import_queue, .. } =
					service::new_partial(&config)?;
				Ok((cmd.run(client, import_queue), task_manager))
			})
		},
		Some(Subcommand::ExportBlocks(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, .. } = service::new_partial(&config)?;
				Ok((cmd.run(client, config.database), task_manager))
			})
		},
		Some(Subcommand::ExportState(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, .. } = service::new_partial(&config)?;
				Ok((cmd.run(client, config.chain_spec), task_manager))
			})
		},
		Some(Subcommand::ImportBlocks(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, import_queue, .. } =
					service::new_partial(&config)?;
				Ok((cmd.run(client, import_queue), task_manager))
			})
		},
		Some(Subcommand::PurgeChain(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.sync_run(|config| cmd.run(config.database))
		},
		Some(Subcommand::Revert(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.async_run(|config| {
				let PartialComponents { client, task_manager, backend, .. } =
					service::new_partial(&config)?;
				let aux_revert = Box::new(|client: Arc<service::FullClient>, backend, blocks| {
					sc_consensus_babe::revert(client.clone(), backend, blocks)?;
					sc_consensus_grandpa::revert(client, blocks)?;
					Ok(())
				});
				Ok((cmd.run(client, backend, Some(aux_revert)), task_manager))
			})
		},
		Some(Subcommand::Benchmark(cmd)) => {
			let runner = cli.create_runner(cmd)?;

			runner.sync_run(|config| {
				// This switch needs to be in the client, since the client decides
				// which sub-commands it wants to support.
				match cmd {
					BenchmarkCmd::Pallet(cmd) => {
						if !cfg!(feature = "runtime-benchmarks") {
							return Err(
								"Runtime benchmarking wasn't enabled when building the node. \
							You can enable it with `--features runtime-benchmarks`."
									.into(),
							)
						}

						cmd.run_with_spec::<sp_runtime::traits::HashingFor<Block>, ()>(Some(
							config.chain_spec,
						))
					},
					BenchmarkCmd::Block(cmd) => {
						let PartialComponents { client, .. } = service::new_partial(&config)?;
						cmd.run(client)
					},
					#[cfg(not(feature = "runtime-benchmarks"))]
					BenchmarkCmd::Storage(_) => Err(
						"Storage benchmarking can be enabled with `--features runtime-benchmarks`."
							.into(),
					),
					#[cfg(feature = "runtime-benchmarks")]
					BenchmarkCmd::Storage(cmd) => {
						let PartialComponents { client, backend, .. } =
							service::new_partial(&config)?;
						let db = backend.expose_db();
						let storage = backend.expose_storage();
						let shared_cache = backend.expose_shared_trie_cache();

						cmd.run(config, client, db, storage, shared_cache)
					},
					BenchmarkCmd::Overhead(cmd) => {
						if cmd.params.runtime.is_some() {
							return Err(sc_cli::Error::Input(
								"Bulletin binary does not support `--runtime` flag for `benchmark overhead`. Please provide a chain spec or use the `frame-omni-bencher`."
									.into(),
							)
								.into())
						}

						cmd.run_with_default_builder_and_spec::<Block, ()>(
							Some(config.chain_spec),
						)
					},
					BenchmarkCmd::Extrinsic(cmd) => {
						let PartialComponents { client, .. } = service::new_partial(&config)?;
						// Register the *Remark* and *TKA* builders.
						let ext_factory = ExtrinsicFactory(vec![
							Box::new(SubstrateRemarkBuilder::new_from_client(client.clone())?),
						]);

						cmd.run(client, inherent_benchmark_data()?, Vec::new(), &ext_factory)
					},
					BenchmarkCmd::Machine(cmd) =>
						cmd.run(&config, SUBSTRATE_REFERENCE_HARDWARE.clone()),
				}
			})
		},
		Some(Subcommand::ChainInfo(cmd)) => {
			let runner = cli.create_runner(cmd)?;
			runner.sync_run(|config| cmd.run::<Block>(&config))
		},
		None => {
			let runner = cli.create_runner(&cli.run)?;
			runner.run_node_until_exit(|mut config| async move {
				// Override default idle connection timeout of 10 seconds to give IPFS clients more
				// time to query data over Bitswap. This is needed when manually adding our node
				// to a swarm of an IPFS node, because the IPFS node doesn't keep any active
				// substreams with us and our node closes a connection after
				// `idle_connection_timeout`.
				const IPFS_WORKAROUND_TIMEOUT: Duration = Duration::from_secs(3600);

				if config.network.idle_connection_timeout < IPFS_WORKAROUND_TIMEOUT {
					tracing::info!(
						target: LOG_TARGET,
						old = ?config.network.idle_connection_timeout,
						overriden_with = ?IPFS_WORKAROUND_TIMEOUT,
						"Overriding `config.network.idle_connection_timeout` to allow long-lived connections with IPFS nodes",

					);
					config.network.idle_connection_timeout = IPFS_WORKAROUND_TIMEOUT;
				}

				if config.network.ipfs_server {
					match config.network.network_backend {
						NetworkBackendType::Litep2p => (),
						NetworkBackendType::Libp2p => {
							return Err(
								"For `ipfs-server`, we expect only the `config.network.network_backend=litep2p` (`--network-backend=litep2p`) setting, because Bitswap support requires it!"
									.into(),
							)
						}
					}
				}

				service::new_full::<sc_network::Litep2pNetworkBackend>(config)
					.map_err(sc_cli::Error::Service)
			})
		},
	}
}
