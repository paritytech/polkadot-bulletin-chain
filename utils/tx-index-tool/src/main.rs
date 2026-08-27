// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Inspect and repair the indexed transaction storage of a Bulletin Chain database.
//!
//! Every command takes the node's rocksdb directory, typically
//! `<base-path>/chains/<chain-id>/db/full`. Reads can run against a node that is up by adding
//! `--live`; writing (`--apply`) needs the node stopped, because that takes rocksdb's
//! exclusive lock.

use clap::{Args, Parser, Subcommand};
use std::{path::PathBuf, process::ExitCode};
use tx_index_tool::{
	best_block, block_hash, chain, compute_storage_proof, diff_databases, dry_run, inspect_block,
	list_entries, merge_chain, parse_hex32, realign_all_corrupted, realign_from_body_index,
	repair_refcounts, repair_value, trace_hash, verify_seams, DbHash, DiffOptions, EntrySort,
	HashAlgo, ListOptions, OpenMode, Verdict,
};

/// Exit codes. Anything a script wants to branch on lives here.
mod exit {
	/// Clean, or the query found what it was looking for.
	pub const OK: u8 = 0;
	/// Found something that needs attention: drift, corruption, a failed proof.
	pub const FINDINGS: u8 = 2;
	/// Nothing matched — no such block, no such entry.
	pub const NOT_FOUND: u8 = 3;
	/// Bad usage.
	pub const USAGE: u8 = 64;
	/// I/O failure, including a database that can't be opened.
	pub const IO: u8 = 74;
}

#[derive(Parser)]
#[command(
	name = "tx-index-tool",
	about = "Inspect and repair a Bulletin Chain node's indexed transaction storage",
	long_about = None,
)]
struct Cli {
	#[command(subcommand)]
	command: Command,
}

#[derive(Args, Clone)]
struct DbArgs {
	/// Path to the rocksdb directory, e.g. <base-path>/chains/<chain-id>/db/full
	db: PathBuf,

	/// Attach lock-free as a rocksdb secondary — reads a database a running node holds open
	#[arg(long)]
	live: bool,

	/// Like --live, but with an explicit secondary state directory instead of a temporary one
	#[arg(long, value_name = "DIR", conflicts_with = "live")]
	secondary: Option<PathBuf>,
}

impl DbArgs {
	fn open_mode(&self) -> OpenMode {
		OpenMode::new(self.live, self.secondary.clone())
	}
}

#[derive(Subcommand)]
enum Command {
	/// List the stored transaction data, verifying each value against its content hash
	List {
		#[command(flatten)]
		db: DbArgs,
		#[command(flatten)]
		opts: ListArgs,
	},
	/// Show which stored transactions one block's body references
	Block {
		#[command(flatten)]
		db: DbArgs,
		/// Block number
		number: u32,
	},
	/// Recompute the storage proof the inherent provider would emit, and verify it
	///
	/// Give a block number to prove it directly, or --authoring/--current with
	/// --retention-period to have the target and its randomness resolved the way the node
	/// does: target = authoring - retention_period, randomness = hash(authoring - 1).
	Proof {
		#[command(flatten)]
		db: DbArgs,
		/// Block number to prove
		#[arg(required_unless_present_any = ["authoring", "current"])]
		number: Option<u32>,
		/// Resolve the target from the block being authored
		#[arg(long, value_name = "N", conflicts_with = "number", requires = "retention_period")]
		authoring: Option<u32>,
		/// Like --authoring, for the block that would come next after the chain head
		#[arg(
			long,
			conflicts_with_all = ["number", "authoring"],
			requires = "retention_period",
		)]
		current: bool,
		/// Retention period in blocks, as the runtime reports it
		#[arg(long, value_name = "BLOCKS")]
		retention_period: Option<u32>,
		/// Randomness selecting the chunk; defaults to the resolved parent hash, else zeroes
		#[arg(long, value_name = "HEX", value_parser = parse_hex32)]
		random: Option<DbHash>,
		/// The chunk root the chain recorded for this entry (`TransactionInfo::chunk_root`).
		/// Without it the check only proves the proof is consistent with the bytes on disk.
		#[arg(long, value_name = "HEX", value_parser = parse_hex32)]
		expect_root: Option<DbHash>,
	},
	/// Reconstruct the reference ledger for one content hash
	///
	/// col11 keeps a single counter, not a history, so a past value cannot be recovered. What
	/// this rebuilds is the ledger: every alive block that references the hash, what each
	/// contributes, and how the sum compares with the counter on disk. With --rpc-url it also
	/// asks the chain what happened at each of those blocks, which is what distinguishes a
	/// reference that was legitimately pruned from one that was released early.
	Trace {
		#[command(flatten)]
		db: DbArgs,
		/// The content hash to trace
		#[arg(value_parser = parse_hex32)]
		hash: DbHash,
		/// Node RPC endpoint to cross-check against, e.g. wss://host or ws://127.0.0.1:9944
		#[arg(long, value_name = "URL")]
		rpc_url: Option<String>,
		/// Also probe the renewal cadence outwards from the blocks already known, to find
		/// renewals this database no longer references
		#[arg(long, requires = "rpc_url")]
		probe_cadence: bool,
		/// Cap on how many block heights are queried over RPC
		#[arg(long, default_value_t = 400, value_name = "N", requires = "rpc_url")]
		chain_max_blocks: usize,
	},
	/// Compare two databases entry by entry
	Diff {
		#[command(flatten)]
		db: DbArgs,
		/// The database to compare against
		other: PathBuf,
		/// Also compare which blocks have an indexed body
		#[arg(long)]
		blocks: bool,
		/// Max differing entries to print; 0 lists all of them
		#[arg(long, default_value_t = 50)]
		limit: usize,
	},
	/// Diagnose or repair a specific past incident
	///
	/// These are RocksDB-era, kvdb-specific faults rather than general database questions, so
	/// each lives under the pull request that caused (or fixed) it.
	Incident {
		#[command(subcommand)]
		incident: Incident,
	},
	/// Overwrite one corrupted value with known-good bytes
	Repair {
		#[command(flatten)]
		db: DbArgs,
		/// Content hash of the entry to repair
		#[arg(value_parser = parse_hex32)]
		hash: DbHash,
		/// File holding the correct bytes
		file: PathBuf,
		/// Hash algorithm the data was stored under
		#[arg(long, default_value = "blake2b256", value_parser = HashAlgo::parse)]
		algo: HashAlgo,
		/// Write the repair (default: report what would happen)
		#[arg(long, conflicts_with_all = ["live", "secondary"])]
		apply: bool,
	},
}

/// A specific historical fault, named for its pull request.
#[derive(Subcommand)]
enum Incident {
	/// polkadot-sdk#12106 — kvdb collapsed N same-key refcount operations in one transaction
	/// into a single ±1, leaving counters short wherever a block referenced the same entry
	/// more than once
	#[command(name = "sdk-12106")]
	Sdk12106 {
		#[command(subcommand)]
		action: Sdk12106Action,
	},
	/// polkadot-bulletin-chain#574 — a trailing `(MultiSigner, MultiSignature, u64)` tuple
	/// moved the BODY_INDEX.header ↔ col11 split by 106-108 bytes, so stored values no longer
	/// hash to their key
	#[command(name = "bulletin-574")]
	Bulletin574 {
		#[command(subcommand)]
		action: Bulletin574Action,
	},
}

#[derive(Subcommand)]
enum Sdk12106Action {
	/// Report the counters the collapse left short; --apply sets them to their true reference
	/// count
	Drift {
		#[command(flatten)]
		db: DbArgs,
		/// Backfill the short counters (default: report the shortfall only)
		#[arg(long, conflicts_with_all = ["live", "secondary"])]
		apply: bool,
	},
}

#[derive(Subcommand)]
enum Bulletin574Action {
	/// Classify every indexed entry's BODY_INDEX.header <-> col11 seam
	///
	/// Tells apart the original mis-split (still executable, value does not hash) from a
	/// col11-only repair (value hashes, but the block can no longer be executed).
	Verify {
		#[command(flatten)]
		db: DbArgs,
	},
	/// Recover corrupted values by re-splitting BODY_INDEX.header ++ col11 at other offsets
	Realign {
		#[command(flatten)]
		db: DbArgs,
		/// Only this entry (default: every corrupted entry)
		#[arg(long, value_name = "HEX", value_parser = parse_hex32)]
		hash: Option<DbHash>,
		/// How many bytes either side of the current split to search
		#[arg(long, default_value_t = 200)]
		max_shift: u32,
		/// Write the recovered values (default: report what would happen)
		#[arg(long, conflicts_with_all = ["live", "secondary"])]
		apply: bool,
	},
}

impl Incident {
	fn db_args(&self) -> &DbArgs {
		match self {
			Incident::Sdk12106 { action: Sdk12106Action::Drift { db, .. } } |
			Incident::Bulletin574 { action: Bulletin574Action::Realign { db, .. } } |
			Incident::Bulletin574 { action: Bulletin574Action::Verify { db } } => db,
		}
	}
}

impl Command {
	/// The database arguments every subcommand carries.
	fn db_args(&self) -> &DbArgs {
		match self {
			Command::List { db, .. } |
			Command::Block { db, .. } |
			Command::Proof { db, .. } |
			Command::Trace { db, .. } |
			Command::Diff { db, .. } |
			Command::Repair { db, .. } => db,
			Command::Incident { incident } => incident.db_args(),
		}
	}
}

#[derive(Args, Clone)]
struct ListArgs {
	/// Maximum entries to print; 0 lists everything
	#[arg(long, default_value_t = 50)]
	limit: usize,

	/// Leading bytes to hexdump per entry; 0 prints none
	#[arg(long, default_value_t = 16)]
	preview: usize,

	/// Ordering: block, size, refcount or hash
	#[arg(long, default_value = "block", value_parser = EntrySort::parse)]
	sort: EntrySort,

	/// Reverse the ordering
	#[arg(long)]
	desc: bool,

	/// Only entries whose value does not hash to its key
	#[arg(long)]
	corrupted_only: bool,

	/// Only entries no alive block references any more
	#[arg(long)]
	orphans_only: bool,

	/// Skip values smaller than this many bytes
	#[arg(long, default_value_t = 0, value_name = "BYTES")]
	min_size: usize,

	/// Only this content hash
	#[arg(long, value_name = "HEX", value_parser = parse_hex32)]
	hash: Option<DbHash>,

	/// Only the entries one block's body references
	#[arg(long, value_name = "NUMBER")]
	block: Option<u32>,

	/// Only entries first stored at or after this block
	#[arg(long, value_name = "NUMBER", conflicts_with = "no_blocks")]
	from_block: Option<u32>,

	/// Only entries first stored at or before this block
	#[arg(long, value_name = "NUMBER", conflicts_with = "no_blocks")]
	to_block: Option<u32>,

	/// Skip the BODY_INDEX pass, dropping the created / last-seen columns
	#[arg(long)]
	no_blocks: bool,
}

impl From<ListArgs> for ListOptions {
	fn from(a: ListArgs) -> Self {
		ListOptions {
			sort: a.sort,
			descending: a.desc,
			limit: (a.limit != 0).then_some(a.limit),
			preview_len: a.preview,
			corrupted_only: a.corrupted_only,
			orphans_only: a.orphans_only,
			min_size: a.min_size,
			hash_filter: a.hash,
			block_filter: a.block,
			from_block: a.from_block,
			to_block: a.to_block,
			resolve_blocks: !a.no_blocks,
		}
	}
}

fn main() -> ExitCode {
	let cli = Cli::parse();

	// Rocksdb keeps many SST files open at once, and a secondary instance is opened with
	// `max_open_files = -1`. The default soft limit (256 on macOS) is nowhere near enough, so
	// raise it the way the node does.
	if let Err(e) = fdlimit::raise_fd_limit() {
		eprintln!("warning: failed to raise file descriptor limit: {e}");
		eprintln!("         a large database may fail with \"Too many open files\";");
		eprintln!("         raise it in the shell instead, e.g. `ulimit -n 65536`.");
	}

	let db_args = cli.command.db_args();
	let mode = db_args.open_mode();

	let db = match tx_index_tool::open_database(&db_args.db, &mode) {
		Ok(db) => db,
		Err(e) => {
			eprintln!("cannot open kvdb at {}: {e}", db_args.db.display());
			return ExitCode::from(exit::IO);
		},
	};

	let code = if let Command::Diff { other, blocks, limit, .. } = &cli.command {
		// The comparison is the one command that needs two databases open at once.
		let other_mode = mode.sibling("b");
		match tx_index_tool::open_database(other, &other_mode) {
			Ok(other_db) => {
				let opts = DiffOptions { limit: (*limit != 0).then_some(*limit), blocks: *blocks };
				let code =
					report("comparison failed", diff_databases(&db, &other_db, &opts), |r| {
						if r.is_identical() {
							exit::OK
						} else {
							exit::FINDINGS
						}
					});
				drop(other_db);
				other_mode.cleanup();
				code
			},
			Err(e) => {
				eprintln!("cannot open kvdb at {}: {e}", other.display());
				exit::IO
			},
		}
	} else {
		print_best_block(&db);
		run(&db, cli.command)
	};
	drop(db);
	mode.cleanup();
	ExitCode::from(code)
}

/// Print a report and turn it into an exit code. Every subcommand has this shape: run the
/// operation, render it, and say whether what it found needs attention.
fn report<R: std::fmt::Display>(
	what: &str,
	outcome: std::io::Result<R>,
	code: impl FnOnce(&R) -> u8,
) -> u8 {
	match outcome {
		Ok(r) => {
			print!("{r}");
			code(&r)
		},
		Err(e) => fail(what, &e),
	}
}

/// As `report`, for the operations that return `Ok(None)` when the thing asked about is absent.
fn report_opt<R: std::fmt::Display>(
	what: &str,
	missing: &str,
	outcome: std::io::Result<Option<R>>,
	code: impl FnOnce(&R) -> u8,
) -> u8 {
	match outcome {
		Ok(Some(r)) => {
			print!("{r}");
			code(&r)
		},
		Ok(None) => {
			eprintln!("{missing}");
			exit::NOT_FOUND
		},
		Err(e) => fail(what, &e),
	}
}

fn run(db: &kvdb_rocksdb::Database, command: Command) -> u8 {
	match command {
		Command::List { opts, .. } => {
			if let (Some(from), Some(to)) = (opts.from_block, opts.to_block) {
				if from > to {
					eprintln!("--from-block #{from} is after --to-block #{to}");
					return exit::USAGE;
				}
			}
			report("listing failed", list_entries(db, &opts.into()), |r| {
				// An empty result means "not found" only when a specific entry or block was
				// asked about. For a predicate like --corrupted-only, empty is the good news.
				let targeted = r.options.hash_filter.is_some() || r.options.block_filter.is_some();
				if r.corrupted > 0 || r.values_corrupted > 0 {
					exit::FINDINGS
				} else if r.matched == 0 && targeted {
					exit::NOT_FOUND
				} else {
					exit::OK
				}
			})
		},
		Command::Block { number, .. } => report_opt(
			"inspect failed",
			&format!("no block #{number} in the database"),
			inspect_block(db, number),
			|_| exit::OK,
		),
		Command::Trace { hash, rpc_url, probe_cadence, chain_max_blocks, .. } => {
			let traced = trace_hash(db, hash).and_then(|mut report| {
				if let Some(url) = &rpc_url {
					// The chain is asked only about the blocks the database already pointed at,
					// plus cadence probes — never the whole live set, which on a long retention
					// period is far too many heights to enumerate.
					let blocks = report.referring_blocks();
					let facts = chain::fetch(url, hash, &blocks, probe_cadence, chain_max_blocks)?;
					merge_chain(&mut report, facts);
				}
				Ok(report)
			});
			report("trace failed", traced, |r| match r.verdict() {
				Verdict::Absent => exit::NOT_FOUND,
				v if v.is_finding() => exit::FINDINGS,
				_ =>
					if r.released().is_empty() && r.spurious().is_empty() {
						exit::OK
					} else {
						exit::FINDINGS
					},
			})
		},
		Command::Proof {
			number,
			authoring,
			current,
			retention_period,
			random,
			expect_root,
			..
		} => {
			let (number, randomness) = match resolve_proof_target(
				db,
				number,
				authoring,
				current,
				retention_period,
				random,
			) {
				Ok(resolved) => resolved,
				Err(message) => {
					eprintln!("{message}");
					return exit::USAGE;
				},
			};
			report_opt(
				"proof computation failed",
				&format!("no indexed body to prove at block #{number}"),
				compute_storage_proof(db, number, randomness, expect_root),
				|r| if r.is_good() { exit::OK } else { exit::FINDINGS },
			)
		},
		Command::Repair { hash, file, algo, apply, .. } => {
			let data = match std::fs::read(&file) {
				Ok(data) => data,
				Err(e) => {
					eprintln!("failed to read {}: {e}", file.display());
					return exit::IO;
				},
			};
			report("repair failed", repair_value(db, hash, algo, &data, apply), |r| {
				if r.hash_matches {
					exit::OK
				} else {
					exit::FINDINGS
				}
			})
		},
		// Handled in `main`, which opens the second database.
		Command::Diff { .. } => unreachable!("diff is dispatched before run()"),
		Command::Incident { incident } => match incident {
			Incident::Sdk12106 { action: Sdk12106Action::Drift { apply: true, .. } } =>
				report("refcount backfill failed", repair_refcounts(db, true), |_| exit::OK),
			Incident::Sdk12106 { action: Sdk12106Action::Drift { apply: false, .. } } =>
				report("drift scan failed", dry_run(db), |r| {
					if r.is_clean() {
						exit::OK
					} else {
						exit::FINDINGS
					}
				}),
			Incident::Bulletin574 { action: Bulletin574Action::Verify { .. } } =>
				report("seam verification failed", verify_seams(db), |r| {
					if r.is_clean() {
						exit::OK
					} else {
						exit::FINDINGS
					}
				}),
			Incident::Bulletin574 {
				action: Bulletin574Action::Realign { hash: Some(hash), max_shift, apply, .. },
			} =>
				report("realign failed", realign_from_body_index(db, hash, max_shift, apply), |r| {
					if r.matched_shift.is_some() {
						exit::OK
					} else {
						exit::FINDINGS
					}
				}),
			Incident::Bulletin574 {
				action: Bulletin574Action::Realign { hash: None, max_shift, apply, .. },
			} => report("realign failed", realign_all_corrupted(db, max_shift, apply), |r| {
				if r.unrecovered() == 0 {
					exit::OK
				} else {
					exit::FINDINGS
				}
			}),
		},
	}
}

/// Work out which block to prove and with what randomness, mirroring
/// `sp_transaction_storage_proof::registration::new_data_provider`: authoring block `N` proves
/// `N - retention_period` using the parent hash — `hash(N - 1)` — as randomness.
///
/// A bare block number keeps the old behaviour: prove exactly that block, with `--random` or
/// zeroes.
fn resolve_proof_target(
	db: &kvdb_rocksdb::Database,
	number: Option<u32>,
	authoring: Option<u32>,
	current: bool,
	retention_period: Option<u32>,
	random: Option<DbHash>,
) -> Result<(u32, [u8; 32]), String> {
	let explicit = random.map(|h| h.to_fixed_bytes());

	// Plain `proof <db> <number>`.
	if let Some(number) = number {
		return Ok((number, explicit.unwrap_or([0u8; 32])));
	}

	let authoring = match (authoring, current) {
		(Some(n), _) => n,
		(None, true) => match best_block(db).map_err(|e| format!("cannot read best block: {e}"))? {
			// The next block to be authored on top of the current head.
			Some((best, _)) => best + 1,
			None => return Err("no best block in META, so --current cannot resolve".into()),
		},
		(None, false) => return Err("give a block number, or --authoring N / --current".into()),
	};
	// clap guarantees this is present alongside --authoring / --current.
	let retention = retention_period.ok_or("--retention-period is required")?;

	let target =
		authoring.checked_sub(retention).ok_or_else(|| {
			format!("authoring #{authoring} is below the retention period {retention}: nothing to prove")
		})?;
	if target == 0 {
		return Err(format!(
			"authoring #{authoring} with retention {retention} targets block 0 — too early for \
			 the chain to owe a proof"
		));
	}

	let parent = authoring - 1;
	let randomness = match explicit {
		Some(bytes) => bytes,
		None => match block_hash(db, parent).map_err(|e| format!("cannot read #{parent}: {e}"))? {
			Some(hash) => hash.to_fixed_bytes(),
			None => return Err(format!("block #{parent} is not in this database")),
		},
	};

	println!(
		"authoring #{authoring}, retention {retention} → proving #{target}, randomness = \
		 hash(#{parent})\n"
	);
	Ok((target, randomness))
}

/// Print a failed database operation, adding the descriptor-limit hint when that is what went
/// wrong. Rocksdb surfaces it mid-scan ("While open a file for random read: … Too many open
/// files"), which is opaque unless you know a secondary instance keeps every SST open.
fn fail(context: &str, e: &std::io::Error) -> u8 {
	eprintln!("{context}: {e}");
	if e.to_string().contains("Too many open files") {
		eprintln!();
		eprintln!("This is the file-descriptor soft limit, not database corruption.");
		eprintln!("Rocksdb holds many SST files open at once — unlimited in --live mode.");
		eprintln!("Raise it and retry:  ulimit -n 65536");
	}
	exit::IO
}

/// Print the chain head, so every report says which database state it describes.
fn print_best_block(db: &kvdb_rocksdb::Database) {
	match best_block(db) {
		Ok(Some((number, hash))) => println!("best block: #{number} ({hash:?})\n"),
		Ok(None) => println!("best block: <no META entry>\n"),
		Err(e) => eprintln!("warning: cannot read best block from META: {e}\n"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use clap::CommandFactory;

	#[test]
	fn cli_definition_is_valid() {
		Cli::command().debug_assert();
	}
}
