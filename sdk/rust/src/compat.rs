// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Version dispatch for items that changed incompatibly across the live fleet.
//!
//! `../metadata.scale` (the crate-private `transaction::bulletin` module) tracks the
//! workspace runtime and is the SDK's canonical interface. Live chains lag it;
//! when an item the SDK uses changed shape incompatibly, its legacy encoding
//! lives here as a module generated from a snapshot of the oldest supported
//! chain with that shape, trimmed to the affected pallet:
//!
//! ```text
//! subxt metadata --url wss://<chain-rpc> --pallets <Pallet> -f bytes \
//!     > sdk/metadata-compat/<pallet>-v<spec_version>.scale
//! ```
//!
//! Dispatch is a registry keyed by subxt's per-item type-tree hash — the same
//! hash its codegen validation uses. Keys are derived at startup from the very
//! files that generate the encoders, so a key and its encoder cannot drift
//! apart; the connected chain's item is hashed with the same function and
//! looked up. An unknown hash fails closed with the observed shape rather
//! than guessing. Structure cannot see semantics: a change that keeps the
//! type tree identical but changes meaning would need an explicit
//! `(spec_name, version range)` override row — none exist today.
//!
//! Trimmed snapshots are safe only for pallet-local items; never encode
//! `RuntimeCall`-embedding calls (`Sudo.sudo`, `Utility.batch_all`) from one —
//! the reduced call enum cannot hash-match a live chain. Delete a snapshot,
//! its module, and its registry row once no supported chain needs it (see
//! `sdk/metadata-compat/README.md` for the inventory).

use crate::types::{Error, Result};
use codec::Decode;
use std::{collections::HashMap, sync::OnceLock};

/// `TransactionStorage` as of bulletin-westend v1000011: `renew` takes
/// positional `(block, index)` instead of a `TransactionRef`.
#[subxt::subxt(runtime_metadata_path = "../metadata-compat/transaction-storage-v1000011.scale")]
pub mod bulletin_v1000011 {}

/// `TransactionStorage` as of bulletin-westend v1000016 (pre renewal split):
/// `renew(entry: TransactionRef)` / `force_renew` still live in
/// `TransactionStorage` instead of `DataRenewal`.
#[subxt::subxt(runtime_metadata_path = "../metadata-compat/transaction-storage-v1000016.scale")]
pub mod bulletin_v1000016 {}

/// Encoders for the renewal calls, one per supported fleet shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenewAdapter {
	/// `DataRenewal.renew(entry: TransactionRef)` — the current runtime
	/// (`metadata.scale`, post renewal split).
	DataRenewal,
	/// `TransactionStorage.renew(entry: TransactionRef)` — pre-split chains
	/// (e.g. bulletin-westend v1000016), encoded via [`bulletin_v1000016`].
	TransactionRef,
	/// `TransactionStorage.renew(block, index)` — legacy chains (e.g.
	/// bulletin-westend v1000011), encoded via [`bulletin_v1000011`].
	Positional,
}

/// Resolve the encoder for the renewal calls on the connected chain by hashing
/// the live item — `DataRenewal.renew` when the pallet exists, otherwise
/// `TransactionStorage.renew` — and looking it up in the registry. Fails
/// closed: [`Error::UnsupportedOperation`] when neither pallet has `renew` or the shape is
/// unknown.
///
/// Keys pair the hosting pallet with the item hash: subxt's per-item hash
/// covers only the call's type tree, and the renewal split moved `renew`
/// across pallets without changing that tree, so the hash alone cannot tell
/// `DataRenewal.renew` from a pre-split `TransactionStorage.renew`.
pub fn renew_adapter(live: &subxt::Metadata) -> Result<RenewAdapter> {
	let (pallet, hash) = if let Some(hash) = live_call_hash(live, "DataRenewal", "renew") {
		("DataRenewal", hash)
	} else if let Some(hash) = live_call_hash(live, "TransactionStorage", "renew") {
		("TransactionStorage", hash)
	} else {
		return Err(Error::UnsupportedOperation("renew is not available on this chain".into()));
	};
	renew_registry().get(&(pallet, hash)).copied().ok_or_else(|| {
		Error::UnsupportedOperation(format!(
			"{pallet}.renew has an unsupported shape on this chain (item hash 0x{}); \
			 this SDK release supports {} shape(s) — a newer runtime may need an SDK upgrade",
			hex32(&hash),
			renew_registry().len()
		))
	})
}

/// `(pallet, hash) → adapter`, keys derived from the same committed metadata
/// that generated each adapter's encoder.
fn renew_registry() -> &'static HashMap<(&'static str, [u8; 32]), RenewAdapter> {
	static REGISTRY: OnceLock<HashMap<(&'static str, [u8; 32]), RenewAdapter>> = OnceLock::new();
	REGISTRY.get_or_init(|| {
		HashMap::from([
			(
				(
					"DataRenewal",
					committed_call_hash(
						include_bytes!("../../metadata.scale"),
						"DataRenewal",
						"renew",
					),
				),
				RenewAdapter::DataRenewal,
			),
			(
				(
					"TransactionStorage",
					committed_call_hash(
						include_bytes!("../../metadata-compat/transaction-storage-v1000016.scale"),
						"TransactionStorage",
						"renew",
					),
				),
				RenewAdapter::TransactionRef,
			),
			(
				(
					"TransactionStorage",
					committed_call_hash(
						include_bytes!("../../metadata-compat/transaction-storage-v1000011.scale"),
						"TransactionStorage",
						"renew",
					),
				),
				RenewAdapter::Positional,
			),
		])
	})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AutoRenewCall {
	Enable,
	Disable,
}

impl AutoRenewCall {
	pub fn call_name(self) -> &'static str {
		match self {
			Self::Enable => "enable_auto_renew",
			Self::Disable => "disable_auto_renew",
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AutoRenewAdapter {
	DataRenewal,
	TransactionStorage,
}

impl AutoRenewAdapter {
	fn pallet_name(self) -> &'static str {
		match self {
			Self::DataRenewal => "DataRenewal",
			Self::TransactionStorage => "TransactionStorage",
		}
	}
}

pub fn auto_renew_adapter(live: &subxt::Metadata, call: AutoRenewCall) -> Result<AutoRenewAdapter> {
	let adapter = match renew_adapter(live)? {
		RenewAdapter::DataRenewal => AutoRenewAdapter::DataRenewal,
		RenewAdapter::TransactionRef | RenewAdapter::Positional =>
			AutoRenewAdapter::TransactionStorage,
	};
	let pallet = adapter.pallet_name();
	let live_hash = live_call_hash(live, pallet, call.call_name()).ok_or_else(|| {
		Error::UnsupportedOperation(format!(
			"{} is not supported by this runtime",
			call.call_name()
		))
	})?;
	if auto_renew_registry().get(&(adapter, call)) != Some(&live_hash) {
		return Err(Error::UnsupportedOperation(format!(
			"{pallet}.{} has an unsupported shape on this chain (item hash 0x{}); \
			 a newer runtime may need an SDK upgrade",
			call.call_name(),
			hex32(&live_hash)
		)));
	}
	Ok(adapter)
}

fn auto_renew_registry() -> &'static HashMap<(AutoRenewAdapter, AutoRenewCall), [u8; 32]> {
	static REGISTRY: OnceLock<HashMap<(AutoRenewAdapter, AutoRenewCall), [u8; 32]>> =
		OnceLock::new();
	REGISTRY.get_or_init(|| {
		let current: &[u8] = include_bytes!("../../metadata.scale");
		let pre_split: &[u8] =
			include_bytes!("../../metadata-compat/transaction-storage-v1000016.scale");
		[AutoRenewCall::Enable, AutoRenewCall::Disable]
			.into_iter()
			.flat_map(|call| {
				[
					(
						(AutoRenewAdapter::DataRenewal, call),
						committed_call_hash(current, "DataRenewal", call.call_name()),
					),
					(
						(AutoRenewAdapter::TransactionStorage, call),
						committed_call_hash(pre_split, "TransactionStorage", call.call_name()),
					),
				]
			})
			.collect()
	})
}

fn live_call_hash(metadata: &subxt::Metadata, pallet: &str, call: &str) -> Option<[u8; 32]> {
	metadata.pallet_by_name(pallet).and_then(|p| p.call_hash(call))
}

/// Hash of `pallet.call` in a committed metadata asset. The asset is embedded
/// at compile time and validated by unit tests, so a failure here is a broken
/// build artifact, not a runtime condition.
fn committed_call_hash(scale: &[u8], pallet: &str, call: &str) -> [u8; 32] {
	let metadata = subxt::Metadata::decode(&mut &*scale).expect("committed metadata asset decodes");
	live_call_hash(&metadata, pallet, call).expect("committed metadata asset contains the item")
}

fn hex32(hash: &[u8; 32]) -> String {
	hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Every committed snapshot resolves to its own adapter. Registry keys
	/// derive from the same files that generate the encoders, so this also
	/// guards the snapshots against corruption or drift.
	#[test]
	fn registry_resolves_every_committed_snapshot() {
		let current =
			subxt::Metadata::decode(&mut &include_bytes!("../../metadata.scale")[..]).unwrap();
		assert_eq!(renew_adapter(&current).unwrap(), RenewAdapter::DataRenewal);

		let pre_split = subxt::Metadata::decode(
			&mut &include_bytes!("../../metadata-compat/transaction-storage-v1000016.scale")[..],
		)
		.unwrap();
		assert_eq!(renew_adapter(&pre_split).unwrap(), RenewAdapter::TransactionRef);

		let legacy = subxt::Metadata::decode(
			&mut &include_bytes!("../../metadata-compat/transaction-storage-v1000011.scale")[..],
		)
		.unwrap();
		assert_eq!(renew_adapter(&legacy).unwrap(), RenewAdapter::Positional);
	}

	#[test]
	fn auto_renew_adapter_per_committed_snapshot() {
		let current =
			subxt::Metadata::decode(&mut &include_bytes!("../../metadata.scale")[..]).unwrap();
		let pre_split = subxt::Metadata::decode(
			&mut &include_bytes!("../../metadata-compat/transaction-storage-v1000016.scale")[..],
		)
		.unwrap();
		let legacy = subxt::Metadata::decode(
			&mut &include_bytes!("../../metadata-compat/transaction-storage-v1000011.scale")[..],
		)
		.unwrap();

		for call in [AutoRenewCall::Enable, AutoRenewCall::Disable] {
			assert_eq!(auto_renew_adapter(&current, call).unwrap(), AutoRenewAdapter::DataRenewal);
			assert_eq!(
				auto_renew_adapter(&pre_split, call).unwrap(),
				AutoRenewAdapter::TransactionStorage
			);
			let error = auto_renew_adapter(&legacy, call).unwrap_err();
			assert_eq!(error.code(), "UNSUPPORTED_OPERATION");
			assert!(error.to_string().contains("not supported by this runtime"), "{error}");
		}
	}

	/// Three rows, three distinct keys — a duplicate hash would silently
	/// shrink the map and mask a mis-generated snapshot.
	#[test]
	fn registry_keys_are_distinct() {
		assert_eq!(renew_registry().len(), 3);
	}

	/// Runtimes built without `pallet-bulletin-data-renewal` and without the
	/// pre-split `TransactionStorage.renew` report `UNSUPPORTED_OPERATION`, so
	/// callers can tell "no renewal here" apart from a failed renewal.
	#[test]
	fn missing_renew_call_error() {
		use subxt::ext::frame_metadata::{RuntimeMetadata, RuntimeMetadataPrefixed};

		let mut prefixed =
			RuntimeMetadataPrefixed::decode(&mut &include_bytes!("../../metadata.scale")[..])
				.unwrap();
		let RuntimeMetadata::V16(metadata) = &mut prefixed.1 else {
			panic!("committed metadata is not V16");
		};
		metadata
			.pallets
			.retain(|pallet| pallet.name != "DataRenewal" && pallet.name != "TransactionStorage");
		let without_renewal = subxt::Metadata::try_from(prefixed).unwrap();

		let error = renew_adapter(&without_renewal).unwrap_err();

		assert!(matches!(error, Error::UnsupportedOperation(_)), "unexpected error: {error:?}");
		assert_eq!(error.code(), "UNSUPPORTED_OPERATION");
	}
}
