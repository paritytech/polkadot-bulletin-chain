// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::uninlined_format_args)]

// Both suites always compile so shared `utils/` helpers don't appear unused under whichever
// feature is off. The feature flags still gate the test binary from running under a bare
// `cargo test --workspace`; runtime selection of which tests execute is done via cargo-test
// filters (see the `just test-zombienet-*` recipes).
#[cfg(any(feature = "zombie-sync-tests", feature = "zombie-auto-renew-tests"))]
mod auto_renew_storage;
#[cfg(any(feature = "zombie-sync-tests", feature = "zombie-auto-renew-tests"))]
mod parachain_sync_storage;
#[cfg(any(feature = "zombie-sync-tests", feature = "zombie-auto-renew-tests"))]
mod utils;
