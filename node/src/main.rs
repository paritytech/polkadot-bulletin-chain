//! Polkadot Bulletin Chain node.

#![warn(missing_docs)]

mod chain_spec;
#[macro_use]
mod service;
mod benchmarking;
mod cli;
mod command;
mod fake_runtime_api;
mod node_primitives;
mod rpc;

fn main() -> sc_cli::Result<()> {
	command::run()
}
