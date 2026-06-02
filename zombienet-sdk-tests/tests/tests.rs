// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::uninlined_format_args)]

// All suites compile together so shared `utils/` helpers don't trip dead-code warnings.
#[cfg(any(
	feature = "zombie-sync-tests",
	feature = "zombie-auto-renew-tests",
	feature = "zombie-hop-tests"
))]
mod auto_renew_storage;
#[cfg(any(
	feature = "zombie-sync-tests",
	feature = "zombie-auto-renew-tests",
	feature = "zombie-hop-tests"
))]
mod hop_promotion_storage;
#[cfg(any(
	feature = "zombie-sync-tests",
	feature = "zombie-auto-renew-tests",
	feature = "zombie-hop-tests"
))]
mod parachain_sync_storage;
#[cfg(any(
	feature = "zombie-sync-tests",
	feature = "zombie-auto-renew-tests",
	feature = "zombie-hop-tests"
))]
mod utils;
