use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use subxt::{
	ext::scale_value::{Composite, Primitive, Value, ValueDef, Variant},
	OnlineClient,
};
use subxt_signer::sr25519::Keypair;

use crate::{
	accounts::NonceTracker,
	client::{BulletinConfig, BulletinExtrinsicParamsBuilder},
	report::TheoreticalLimits,
};

/// Redact `--authorizer-seed` value from a command-line args list.
fn redact_seed(args: Vec<String>) -> String {
	let mut result = Vec::with_capacity(args.len());
	let mut skip_next = false;
	for arg in &args {
		if skip_next {
			result.push("<redacted>".to_string());
			skip_next = false;
		} else if arg == "--authorizer-seed" {
			result.push(arg.clone());
			skip_next = true;
		} else if arg.starts_with("--authorizer-seed=") {
			result.push("--authorizer-seed=<redacted>".to_string());
		} else {
			result.push(arg.clone());
		}
	}
	result.join(" ")
}

/// Environment metadata captured at test startup for reproducibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentInfo {
	/// Node implementation name (e.g. "Polkadot Bulletin Chain Node").
	pub node_name: String,
	/// Node implementation version.
	pub node_version: String,
	/// Chain name (e.g. "Bulletin Westend").
	pub chain_name: String,
	/// Runtime spec version.
	pub runtime_spec_version: u32,
	/// Runtime transaction version.
	pub runtime_transaction_version: u32,
	/// WebSocket URL used to connect.
	pub ws_url: String,
	/// Stress-test crate version.
	pub stress_test_version: String,
	/// Full command line used to invoke the test.
	pub command_line: String,
}

impl EnvironmentInfo {
	/// Query environment info from a live node.
	pub async fn query(client: &OnlineClient<BulletinConfig>, ws_url: &str) -> Result<Self> {
		use jsonrpsee::{core::client::ClientT, ws_client::WsClientBuilder};

		let rpc = WsClientBuilder::default().build(ws_url).await?;
		let node_name: String = rpc.request("system_name", jsonrpsee::rpc_params![]).await?;
		let node_version: String = rpc.request("system_version", jsonrpsee::rpc_params![]).await?;
		let chain_name: String = rpc.request("system_chain", jsonrpsee::rpc_params![]).await?;

		let rv = client.runtime_version();

		Ok(Self {
			node_name,
			node_version,
			chain_name,
			runtime_spec_version: rv.spec_version,
			runtime_transaction_version: rv.transaction_version,
			ws_url: ws_url.to_string(),
			stress_test_version: env!("CARGO_PKG_VERSION").to_string(),
			command_line: redact_seed(std::env::args().collect()),
		})
	}

	/// Print a human-readable summary.
	pub fn print_text(&self) {
		println!();
		println!("{}", "=".repeat(72));
		println!(" ENVIRONMENT");
		println!("{}", "=".repeat(72));
		println!(" Node                | {} {}", self.node_name, self.node_version);
		println!(" Chain               | {}", self.chain_name);
		println!(
			" Runtime             | spec_version={}, tx_version={}",
			self.runtime_spec_version, self.runtime_transaction_version
		);
		println!(" WS URL              | {}", self.ws_url);
		println!(" Stress-test version | {}", self.stress_test_version);
		println!(" Command line        | {}", self.command_line);
		println!("{}", "=".repeat(72));
		println!();
	}
}

/// Runtime constants queried from a live chain. Replaces all hardcoded
/// weight/length/count constants so the stress-test auto-calibrates after
/// runtime upgrades.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainLimits {
	/// Normal-class max weight budget (ref_time).
	/// Source: `System::BlockWeights.per_class.normal.max_total.ref_time`
	pub max_normal_weight: u64,
	/// Per-extrinsic base weight (ref_time).
	/// Source: `System::BlockWeights.per_class.normal.base_extrinsic.ref_time`
	pub extrinsic_base_weight: u64,
	/// Normal-class max block length in bytes.
	/// Source: `System::BlockLength.max.normal`
	pub normal_block_length: u64,
	/// Hard limit on storage extrinsics per block.
	/// Source: `TransactionStorage::MaxBlockTransactions`
	pub max_block_transactions: u32,
	/// Max payload size per transaction.
	/// Source: `TransactionStorage::MaxTransactionSize`
	pub max_transaction_size: u32,
	/// Store extrinsic total weight (intercept from linear regression).
	/// Source: `TransactionPaymentApi::query_info regression (intercept)`
	pub store_weight_base: u64,
	/// Store extrinsic per-byte weight (slope from linear regression).
	/// Source: `TransactionPaymentApi::query_info regression (slope)`
	pub store_weight_per_byte: u64,
	/// Encoding overhead per extrinsic (signature + address + extensions).
	/// Source: `Measured: encoded_tx_len - payload_size`
	pub extrinsic_length_overhead: u64,
}

impl ChainLimits {
	/// Query all chain limits from a live node.
	pub async fn query(
		client: &OnlineClient<BulletinConfig>,
		signer: &Keypair,
		nonce_tracker: &NonceTracker,
	) -> Result<Self> {
		tracing::info!("ChainLimits: querying block limits...");
		let (max_normal_weight, extrinsic_base_weight, normal_block_length) =
			query_block_limits(client).await?;
		tracing::info!("ChainLimits: querying storage limits...");
		let (max_block_transactions, max_transaction_size) = query_storage_limits(client).await?;
		tracing::info!("ChainLimits: measuring store weight (builds + queries 2 txs)...");
		let (store_weight_base, store_weight_per_byte, extrinsic_length_overhead) =
			measure_store_weight(client, signer, nonce_tracker).await?;
		tracing::info!("ChainLimits: all queries complete");

		Ok(Self {
			max_normal_weight,
			extrinsic_base_weight,
			normal_block_length,
			max_block_transactions,
			max_transaction_size,
			store_weight_base,
			store_weight_per_byte,
			extrinsic_length_overhead,
		})
	}

	/// Compute theoretical block capacity for a given payload size.
	pub fn compute_theoretical_limits(&self, payload_size: usize) -> TheoreticalLimits {
		let weight_per_tx =
			self.store_weight_base + self.store_weight_per_byte * payload_size as u64;
		let length_per_tx = payload_size as u64 + self.extrinsic_length_overhead;

		let weight_cap = self.max_normal_weight / weight_per_tx;
		let length_cap = self.normal_block_length / length_per_tx;
		let count_cap = self.max_block_transactions as u64;

		let effective_cap = weight_cap.min(length_cap).min(count_cap);
		let bottleneck = if effective_cap == count_cap {
			format!("MaxBlockTxs ({})", self.max_block_transactions)
		} else if effective_cap == length_cap {
			"block length".to_string()
		} else {
			"block weight".to_string()
		};

		TheoreticalLimits { weight_cap, length_cap, count_cap, effective_cap, bottleneck }
	}

	/// Estimate how many store txs of `payload_size` bytes fit in one block.
	pub fn estimate_block_capacity(&self, payload_size: usize) -> usize {
		self.compute_theoretical_limits(payload_size).effective_cap as usize
	}

	/// Print a human-readable summary of the queried limits.
	pub fn print_text(&self) {
		println!();
		println!("{}", "=".repeat(72));
		println!(" CHAIN LIMITS (queried from runtime)");
		println!("{}", "=".repeat(72));
		println!(
			" max_normal_weight       | {:>15} | System::BlockWeights…normal.max_total",
			self.max_normal_weight
		);
		println!(
			" extrinsic_base_weight   | {:>15} | System::BlockWeights…normal.base_extrinsic",
			self.extrinsic_base_weight
		);
		println!(
			" normal_block_length     | {:>15} | System::BlockLength.max.normal",
			self.normal_block_length
		);
		println!(
			" max_block_transactions  | {:>15} | TransactionStorage::MaxBlockTransactions",
			self.max_block_transactions
		);
		println!(
			" max_transaction_size    | {:>15} | TransactionStorage::MaxTransactionSize",
			self.max_transaction_size
		);
		println!(
			" store_weight_base       | {:>15} | TransactionPaymentApi regression (intercept)",
			self.store_weight_base
		);
		println!(
			" store_weight_per_byte   | {:>15} | TransactionPaymentApi regression (slope)",
			self.store_weight_per_byte
		);
		println!(
			" extrinsic_length_overhead| {:>14} | Measured: encoded_tx_len - payload_size",
			self.extrinsic_length_overhead
		);
		println!("{}", "=".repeat(72));
		println!();
	}
}

// ---------------------------------------------------------------------------
// Internal query helpers
// ---------------------------------------------------------------------------

/// Query `System::BlockWeights` and `System::BlockLength` from metadata constants.
async fn query_block_limits(client: &OnlineClient<BulletinConfig>) -> Result<(u64, u64, u64)> {
	// BlockWeights
	let block_weights_addr = subxt::dynamic::constant("System", "BlockWeights");
	let block_weights = client
		.constants()
		.at(&block_weights_addr)
		.context("Failed to query System::BlockWeights")?;
	let bw = block_weights.to_value()?;

	// Navigate: per_class.normal.max_total (Option<Weight>) → ref_time
	let normal_class = find_named_field(&bw.value, "per_class")
		.and_then(|v| find_named_field(&v.value, "normal"))
		.ok_or_else(|| anyhow!("Cannot navigate BlockWeights.per_class.normal"))?;

	let max_total_ref_time = find_named_field(&normal_class.value, "max_total")
		.and_then(|v| unwrap_option_some(&v.value))
		.and_then(|v| find_named_field(&v.value, "ref_time"))
		.and_then(|v| value_to_u64(&v.value))
		.ok_or_else(|| anyhow!("Cannot extract max_total.ref_time from BlockWeights"))?;

	let base_extrinsic_ref_time = find_named_field(&normal_class.value, "base_extrinsic")
		.and_then(|v| find_named_field(&v.value, "ref_time"))
		.and_then(|v| value_to_u64(&v.value))
		.ok_or_else(|| anyhow!("Cannot extract base_extrinsic.ref_time from BlockWeights"))?;

	// BlockLength
	let block_length_addr = subxt::dynamic::constant("System", "BlockLength");
	let block_length = client
		.constants()
		.at(&block_length_addr)
		.context("Failed to query System::BlockLength")?;
	let bl = block_length.to_value()?;

	let normal_length = find_named_field(&bl.value, "max")
		.and_then(|v| find_named_field(&v.value, "normal"))
		.and_then(|v| value_to_u64(&v.value))
		.ok_or_else(|| anyhow!("Cannot extract BlockLength.max.normal"))?;

	tracing::info!(
		"Block limits: max_normal_weight={max_total_ref_time}, base_extrinsic={base_extrinsic_ref_time}, normal_length={normal_length}"
	);

	Ok((max_total_ref_time, base_extrinsic_ref_time, normal_length))
}

/// Query `TransactionStorage::MaxBlockTransactions` and `MaxTransactionSize`.
async fn query_storage_limits(client: &OnlineClient<BulletinConfig>) -> Result<(u32, u32)> {
	let max_block_txs_addr = subxt::dynamic::constant("TransactionStorage", "MaxBlockTransactions");
	let max_block_txs = client
		.constants()
		.at(&max_block_txs_addr)
		.context("Failed to query TransactionStorage::MaxBlockTransactions")?;
	let max_block_txs_val = max_block_txs.to_value()?;
	let max_block_transactions = value_to_u64(&max_block_txs_val.value)
		.ok_or_else(|| anyhow!("Cannot decode MaxBlockTransactions"))?
		as u32;

	let max_tx_size_addr = subxt::dynamic::constant("TransactionStorage", "MaxTransactionSize");
	let max_tx_size = client
		.constants()
		.at(&max_tx_size_addr)
		.context("Failed to query TransactionStorage::MaxTransactionSize")?;
	let max_tx_size_val = max_tx_size.to_value()?;
	let max_transaction_size = value_to_u64(&max_tx_size_val.value)
		.ok_or_else(|| anyhow!("Cannot decode MaxTransactionSize"))?
		as u32;

	tracing::info!(
		"Storage limits: max_block_transactions={max_block_transactions}, max_transaction_size={max_transaction_size}"
	);

	Ok((max_block_transactions, max_transaction_size))
}

/// Build two store txs with different payload sizes, call
/// `TransactionPaymentApi_query_info` on each, and derive per-byte weight
/// via linear regression. Also measures extrinsic encoding overhead.
async fn measure_store_weight(
	client: &OnlineClient<BulletinConfig>,
	signer: &Keypair,
	nonce_tracker: &NonceTracker,
) -> Result<(u64, u64, u64)> {
	let account_id = signer.public_key().to_account_id();

	// Build two signed transactions with different payload sizes.
	let size_small: usize = 1024;
	let size_large: usize = 10 * 1024;

	let (encoded_small, len_small) = build_store_tx(client, signer, nonce_tracker, size_small)
		.await
		.context("Failed to build small store tx")?;
	nonce_tracker.rollback(&account_id);

	let (encoded_large, _len_large) = build_store_tx(client, signer, nonce_tracker, size_large)
		.await
		.context("Failed to build large store tx")?;
	nonce_tracker.rollback(&account_id);

	// Extrinsic length overhead = encoded_len - payload_size.
	// Use the small tx (overhead is constant regardless of payload).
	let extrinsic_length_overhead = len_small.saturating_sub(size_small) as u64;

	// Call TransactionPaymentApi_query_info for each.
	let latest = client.blocks().at_latest().await?.hash();

	let weight_small = query_tx_weight(client, &encoded_small, latest).await?;
	let weight_large = query_tx_weight(client, &encoded_large, latest).await?;

	// Linear regression: weight = base + per_byte * payload_size
	let per_byte = weight_large.saturating_sub(weight_small) /
		(size_large as u64).saturating_sub(size_small as u64);
	let base = weight_small.saturating_sub(per_byte * size_small as u64);

	tracing::info!(
		"Store weight regression: base={base}, per_byte={per_byte}, overhead={extrinsic_length_overhead}"
	);

	Ok((base, per_byte, extrinsic_length_overhead))
}

/// Build (but don't submit) a signed store transaction. Returns (encoded_bytes, len).
async fn build_store_tx(
	client: &OnlineClient<BulletinConfig>,
	signer: &Keypair,
	nonce_tracker: &NonceTracker,
	payload_size: usize,
) -> Result<(Vec<u8>, usize)> {
	use subxt::dynamic::{tx, Value as DynValue};

	let account_id = signer.public_key().to_account_id();
	let data = vec![0u8; payload_size];
	let call = tx("TransactionStorage", "store", vec![DynValue::from_bytes(&data)]);
	let nonce = nonce_tracker.next_nonce(&account_id);
	let params = BulletinExtrinsicParamsBuilder::new().nonce(nonce).build();
	let signed = client.tx().create_signed(&call, signer, params).await?;
	let encoded = signed.encoded().to_vec();
	let len = encoded.len();
	Ok((encoded, len))
}

/// Call `TransactionPaymentApi_query_info` and extract `weight.ref_time`.
async fn query_tx_weight(
	client: &OnlineClient<BulletinConfig>,
	encoded_tx: &[u8],
	at: subxt::utils::H256,
) -> Result<u64> {
	use subxt::ext::codec::{Compact, Decode, Encode};

	// Parameters: (Extrinsic, u32). `encoded_tx` is already the SCALE encoding
	// of UncheckedExtrinsic (compact length prefix + inner bytes), so we
	// concatenate it directly with the u32 length parameter.
	let mut call_data = Vec::new();
	call_data.extend_from_slice(encoded_tx);
	(encoded_tx.len() as u32).encode_to(&mut call_data);

	let result: Vec<u8> = client
		.runtime_api()
		.at(at)
		.call_raw("TransactionPaymentApi_query_info", Some(&call_data))
		.await
		.context("TransactionPaymentApi_query_info call failed")?;

	// RuntimeDispatchInfo layout:
	//   Weight { ref_time: Compact<u64>, proof_size: Compact<u64> }
	//   DispatchClass (1 byte enum)
	//   Balance (Compact<u128>)
	// We only need ref_time (first Compact<u64>).
	let mut cursor = &result[..];
	let ref_time = Compact::<u64>::decode(&mut cursor)
		.context("Failed to decode ref_time from RuntimeDispatchInfo")?
		.0;

	Ok(ref_time)
}

// ---------------------------------------------------------------------------
// scale_value navigation helpers (generic over type context T)
// ---------------------------------------------------------------------------

/// Navigate into a named field of a Composite ValueDef.
fn find_named_field<'a, T>(def: &'a ValueDef<T>, field_name: &str) -> Option<&'a Value<T>> {
	match def {
		ValueDef::Composite(Composite::Named(fields)) => {
			for (name, val) in fields {
				if name == field_name {
					return Some(val);
				}
			}
			None
		},
		_ => None,
	}
}

/// Unwrap an Option variant: Some(value) → &value, None → None.
fn unwrap_option_some<T>(def: &ValueDef<T>) -> Option<&Value<T>> {
	match def {
		ValueDef::Variant(Variant { name, values: Composite::Unnamed(vals) }) if name == "Some" =>
			vals.first(),
		ValueDef::Variant(Variant { name, values: Composite::Named(fields) }) if name == "Some" =>
			fields.first().map(|(_, v)| v),
		_ => None,
	}
}

/// Extract a u64 from a Primitive ValueDef.
pub(crate) fn value_to_u64<T>(def: &ValueDef<T>) -> Option<u64> {
	match def {
		ValueDef::Primitive(Primitive::U128(n)) => Some(*n as u64),
		ValueDef::Primitive(Primitive::U256(bytes)) => {
			// Little-endian u256 — take low 8 bytes as u64.
			Some(u64::from_le_bytes(bytes[..8].try_into().ok()?))
		},
		_ => None,
	}
}
