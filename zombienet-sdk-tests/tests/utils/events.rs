// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Block-event helpers.

use subxt::{config::substrate::SubstrateConfig, events::Events};

/// Count `TransactionStorage` events of the given variant in a block.
pub fn count_event(events: &Events<SubstrateConfig>, variant: &str) -> u32 {
	events
		.iter()
		.filter_map(|e| e.ok())
		.filter(|e| e.pallet_name() == "TransactionStorage" && e.variant_name() == variant)
		.count() as u32
}
