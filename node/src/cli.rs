use crate::sc_cli::RunCmd;

#[derive(Debug, clap::Parser)]
pub struct Cli {
	#[command(subcommand)]
	pub subcommand: Option<Subcommand>,

	#[clap(flatten)]
	pub run: RunCmd,
}

#[derive(Debug, clap::Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Subcommand {
	/// Key management cli utilities
	#[command(subcommand)]
	Key(crate::sc_cli::KeySubcommand),

	/// Build a chain specification.
	BuildSpec(crate::sc_cli::BuildSpecCmd),

	/// Validate blocks.
	CheckBlock(crate::sc_cli::CheckBlockCmd),

	/// Export blocks.
	ExportBlocks(crate::sc_cli::ExportBlocksCmd),

	/// Export the state of a given block into a chain spec.
	ExportState(crate::sc_cli::ExportStateCmd),

	/// Import blocks.
	ImportBlocks(crate::sc_cli::ImportBlocksCmd),

	/// Remove the whole chain.
	PurgeChain(crate::sc_cli::PurgeChainCmd),

	/// Revert the chain to a previous state.
	Revert(crate::sc_cli::RevertCmd),

	/// Sub-commands concerned with benchmarking.
	#[command(subcommand)]
	Benchmark(crate::frame_benchmarking_cli::BenchmarkCmd),

	/// Db meta columns information.
	ChainInfo(crate::sc_cli::ChainInfoCmd),
}
