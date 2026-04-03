//! SCALE-encode [`RuntimeCall`] values for the Bulletin Westend parachain runtime.
//!
//! Output is the raw `RuntimeCall` encoding (pallet index + call index + args). Submitting a
//! transaction still requires wrapping this in a signed extrinsic (nonce, signature, extensions).
//!
//! ```text
//! cargo run -p encode-westend-calls -- session-set-keys --from-rotate-hex 0x...
//! cargo run -p encode-westend-calls -- session-set-keys --aura-secret-uri //YourAuraSeed
//! cargo run -p encode-westend-calls -- session-set-keys --aura-pub 5Dt6... --aura-secret-uri //Seed
//! cargo run -p encode-westend-calls -- session-set-keys --aura-pub 0x...
//! cargo run -p encode-westend-calls -- add-invulnerable --who 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY
//! ```

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use codec::{Decode, Encode};
use sp_consensus_aura::sr25519::AuthorityId;
use sp_core::{
	crypto::{Pair, Ss58Codec},
	proof_of_possession::statement_of_ownership,
	sr25519,
};
use sp_runtime::{traits::IdentifyAccount, AccountId32, MultiSigner};

use bulletin_westend_runtime::{Runtime, RuntimeCall, SessionKeys};

#[derive(Parser)]
#[command(name = "encode-westend-calls")]
#[command(about = "SCALE-encode bulletin-westend RuntimeCall payloads")]
struct Cli {
	/// Print only `0x`-prefixed hex (no labels)
	#[arg(long, global = true)]
	raw: bool,

	#[command(subcommand)]
	command: Commands,
}

#[derive(Subcommand)]
enum Commands {
	/// `session.set_keys` — use hex from collator RPC `author_rotateKeys`, or a raw 32-byte Aura
	/// public key.
	SessionSetKeys {
		/// Hex from `author_rotateKeys` (SCALE-encoded `SessionKeys`)
		#[arg(long, conflicts_with = "aura_pub")]
		from_rotate_hex: Option<String>,

		/// Aura sr25519 public key: SS58 (e.g. `5Dt6...`) or `0x` + 32-byte hex
		#[arg(long, conflicts_with = "from_rotate_hex")]
		aura_pub: Option<String>,

		/// `proof` argument (hex). Ignored when `--aura-secret-uri` is set (proof is computed).
		#[arg(long, default_value = "0x")]
		proof_hex: String,

		/// Secret URI for the Aura key (e.g. `//Alice`). Derives `SessionKeys` and proof. The
		/// proof assumes the extrinsic signer is the account derived from this key
		/// (`MultiSigner::Sr25519(pair.public()).into_account()` — same SS58 as `--aura-pub`). If
		/// `--aura-pub` or `--from-rotate-hex` is set, it must match this secret.
		#[arg(long)]
		aura_secret_uri: Option<String>,
	},
	/// `collatorSelection.add_invulnerable` (privileged origin)
	AddInvulnerable {
		/// Account id (SS58 or `0x` + 32 bytes)
		#[arg(long)]
		who: String,
	},
	/// `collatorSelection.remove_invulnerable` (privileged origin)
	RemoveInvulnerable {
		#[arg(long)]
		who: String,
	},
	/// `collatorSelection.register_as_candidate` (signed by collator)
	RegisterAsCandidate,
	/// `collatorSelection.leave_intent` (signed by collator)
	LeaveIntent,
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>> {
	let s = s.strip_prefix("0x").unwrap_or(s);
	hex::decode(s).context("invalid hex")
}

fn parse_account(s: &str) -> Result<AccountId32> {
	if s.starts_with("0x") {
		let bytes = parse_hex_bytes(s)?;
		if bytes.len() != 32 {
			bail!("account id hex must be 32 bytes, got {}", bytes.len());
		}
		let mut arr = [0u8; 32];
		arr.copy_from_slice(&bytes);
		Ok(AccountId32::from(arr))
	} else {
		AccountId32::from_ss58check(s).map_err(|e| anyhow::anyhow!("invalid SS58: {e:?}"))
	}
}

/// Decode `sr25519::Public` for Aura session keys from SS58 or `0x` + 64 hex chars.
///
/// If the value starts with `0x` but is not valid hex (e.g. `0x5Grwva...` — SS58 with a mistaken
/// prefix), we parse the part after `0x` as SS58.
fn parse_aura_public(s: &str) -> Result<sr25519::Public> {
	if let Some(after_prefix) = s.strip_prefix("0x") {
		if !after_prefix.is_empty() && after_prefix.chars().all(|c| c.is_ascii_hexdigit()) {
			let bytes = hex::decode(after_prefix).context("invalid hex after 0x")?;
			if bytes.len() != 32 {
				bail!("aura public key hex must be 32 bytes, got {}", bytes.len());
			}
			let mut raw = [0u8; 32];
			raw.copy_from_slice(&bytes);
			return Ok(sr25519::Public::from_raw(raw));
		}
		if !after_prefix.is_empty() {
			return sr25519::Public::from_ss58check(after_prefix).map_err(|e| {
				anyhow::anyhow!(
					"invalid Aura key: not valid hex after 0x, and SS58 parse failed: {e:?}"
				)
			});
		}
		bail!("empty string after 0x");
	}
	sr25519::Public::from_ss58check(s).map_err(|e| anyhow::anyhow!("invalid Aura SS58: {e:?}"))
}

fn session_keys_from_inputs(
	from_rotate_hex: Option<String>,
	aura_pub: Option<String>,
	aura_secret_uri: Option<String>,
) -> Result<SessionKeys> {
	let pair_opt = aura_secret_uri
		.as_ref()
		.map(|u| {
			sr25519::Pair::from_string(u, None)
				.map_err(|e| anyhow::anyhow!("invalid --aura-secret-uri: {e:?}"))
		})
		.transpose()?;

	match (&from_rotate_hex, &aura_pub, &pair_opt) {
		(None, None, Some(pair)) => Ok(SessionKeys { aura: pair.public().into() }),
		(Some(h), None, None) => {
			let bytes = parse_hex_bytes(h)?;
			SessionKeys::decode(&mut &bytes[..]).context("decode SessionKeys from rotate hex")
		},
		(None, Some(h), None) => {
			let pk = parse_aura_public(h)?;
			Ok(SessionKeys { aura: pk.into() })
		},
		(Some(h), None, Some(pair)) => {
			let bytes = parse_hex_bytes(h)?;
			let keys = SessionKeys::decode(&mut &bytes[..]).context("decode SessionKeys")?;
			let derived: AuthorityId = pair.public().into();
			if derived != keys.aura {
				bail!("--aura-secret-uri does not match Aura key in --from-rotate-hex");
			}
			Ok(keys)
		},
		(None, Some(h), Some(pair)) => {
			let pk = parse_aura_public(h)?;
			if pk != pair.public() {
				bail!("--aura-secret-uri does not match --aura-pub");
			}
			Ok(SessionKeys { aura: pair.public().into() })
		},
		(Some(_), Some(_), _) => {
			bail!("use only one of --from-rotate-hex or --aura-pub (optionally with --aura-secret-uri for proof)")
		},
		_ => bail!(
			"provide --aura-secret-uri alone, or --from-rotate-hex, or --aura-pub (see --help)"
		),
	}
}

/// Proof for `session.set_keys`: SCALE-encoded tuple of per-key signatures. For Aura-only keys
/// this is `(sr25519::Signature,)`, i.e. a signature over `statement_of_ownership(owner)` where
/// `owner` is **`AccountId::encode()`** of the extrinsic signer (same bytes `pallet_session` passes
/// to [`OpaqueKeys::ownership_proof_is_valid`]).
fn session_proof_for_aura(keys: &SessionKeys, aura_pair: &sr25519::Pair) -> Result<Vec<u8>> {
	let derived: AuthorityId = aura_pair.public().into();
	if derived != keys.aura {
		bail!("--aura-secret-uri does not match the Aura public key in session keys");
	}
	let controller = MultiSigner::Sr25519(aura_pair.public()).into_account();
	let owner = controller.encode();
	let stmt = statement_of_ownership(&owner);
	let sig = aura_pair.sign(&stmt);
	// One session key → proof is a 1-tuple of `Signature` (SCALE-encoded).
	Ok((sig,).encode())
}

fn print_encoded(raw: bool, label: &str, call: &RuntimeCall) {
	let bytes = call.encode();
	let hex_str = format!("0x{}", hex::encode(&bytes));
	if raw {
		println!("{hex_str}");
	} else {
		println!("{label}");
		println!("  SCALE (RuntimeCall): {hex_str}");
		println!("  len: {} bytes", bytes.len());
	}
}

/// Prints Aura public SS58 — same as `--aura-pub` and as the extrinsic signer account derived from
/// this key (`MultiSigner::Sr25519(pair.public()).into_account()`).
fn print_session_keys_derived_from_secret(pair: &sr25519::Pair) {
	let aura_ss58 = pair.public().to_ss58check();
	println!("Derived from --aura-secret-uri (`--aura-pub` / signer account SS58):");
	println!("  {aura_ss58}");
}

fn main() -> Result<()> {
	let cli = Cli::parse();

	let call = match cli.command {
		Commands::SessionSetKeys { from_rotate_hex, aura_pub, proof_hex, aura_secret_uri } => {
			let keys =
				session_keys_from_inputs(from_rotate_hex, aura_pub, aura_secret_uri.clone())?;
			let proof = if let Some(ref uri) = aura_secret_uri {
				let pair = sr25519::Pair::from_string(uri, None)
					.map_err(|e| anyhow::anyhow!("invalid --aura-secret-uri: {e:?}"))?;
				if !cli.raw {
					print_session_keys_derived_from_secret(&pair);
				}
				session_proof_for_aura(&keys, &pair)?
			} else {
				parse_hex_bytes(&proof_hex).context("proof hex")?
			};
			RuntimeCall::Session(pallet_session::Call::<Runtime>::set_keys { keys, proof })
		},
		Commands::AddInvulnerable { who } => {
			let who = parse_account(&who)?;
			RuntimeCall::CollatorSelection(
				pallet_collator_selection::Call::<Runtime>::add_invulnerable { who },
			)
		},
		Commands::RemoveInvulnerable { who } => {
			let who = parse_account(&who)?;
			RuntimeCall::CollatorSelection(
				pallet_collator_selection::Call::<Runtime>::remove_invulnerable { who },
			)
		},
		Commands::RegisterAsCandidate => RuntimeCall::CollatorSelection(
			pallet_collator_selection::Call::<Runtime>::register_as_candidate {},
		),
		Commands::LeaveIntent => RuntimeCall::CollatorSelection(pallet_collator_selection::Call::<
			Runtime,
		>::leave_intent {}),
	};

	print_encoded(cli.raw, "Encoded call", &call);
	Ok(())
}
