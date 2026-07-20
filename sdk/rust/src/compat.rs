// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Version dispatch for items that changed incompatibly across the live fleet.
//!
//! `../metadata.scale` (the [`crate::transaction::bulletin`] module) tracks the
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

/// Encoders for `TransactionStorage.renew`, one per supported fleet shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenewAdapter {
	/// `renew(entry: TransactionRef)` — the current runtime (`metadata.scale`).
	TransactionRef,
	/// `renew(block, index)` — legacy chains (e.g. bulletin-westend v1000011),
	/// encoded via [`bulletin_v1000011`].
	Positional,
}

/// Resolve the encoder for `TransactionStorage.renew` on the connected chain
/// by hashing the live item and looking it up in the registry. Fails closed
/// on an absent or unknown shape.
pub fn renew_adapter(live: &subxt::Metadata) -> Result<RenewAdapter> {
	let Some(hash) = live_call_hash(live, "TransactionStorage", "renew") else {
		return Err(Error::RenewalFailed(
			"TransactionStorage.renew is not available on this chain".into(),
		));
	};
	renew_registry().get(&hash).copied().ok_or_else(|| {
		Error::RenewalFailed(format!(
			"TransactionStorage.renew has an unsupported shape on this chain (item hash 0x{}); \
			 this SDK release supports {} shape(s) — a newer runtime may need an SDK upgrade",
			hex32(&hash),
			renew_registry().len()
		))
	})
}

/// `hash → adapter`, keys derived from the same committed metadata that
/// generated each adapter's encoder.
fn renew_registry() -> &'static HashMap<[u8; 32], RenewAdapter> {
	static REGISTRY: OnceLock<HashMap<[u8; 32], RenewAdapter>> = OnceLock::new();
	REGISTRY.get_or_init(|| {
		HashMap::from([
			(
				committed_call_hash(
					include_bytes!("../../metadata.scale"),
					"TransactionStorage",
					"renew",
				),
				RenewAdapter::TransactionRef,
			),
			(
				committed_call_hash(
					include_bytes!("../../metadata-compat/transaction-storage-v1000011.scale"),
					"TransactionStorage",
					"renew",
				),
				RenewAdapter::Positional,
			),
		])
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
		assert_eq!(renew_adapter(&current).unwrap(), RenewAdapter::TransactionRef);

		let legacy = subxt::Metadata::decode(
			&mut &include_bytes!("../../metadata-compat/transaction-storage-v1000011.scale")[..],
		)
		.unwrap();
		assert_eq!(renew_adapter(&legacy).unwrap(), RenewAdapter::Positional);
	}

	/// Two rows, two distinct keys — a duplicate hash would silently shrink
	/// the map and mask a mis-generated snapshot.
	#[test]
	fn registry_keys_are_distinct() {
		assert_eq!(renew_registry().len(), 2);
	}
}
