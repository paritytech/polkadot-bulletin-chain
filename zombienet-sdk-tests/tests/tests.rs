// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::uninlined_format_args)]

// Both suites compile together so shared `utils/` helpers don't trip dead-code warnings.
#[cfg(any(feature = "zombie-sync-tests", feature = "zombie-auto-renew-tests"))]
mod auto_renew_storage;
#[cfg(any(feature = "zombie-sync-tests", feature = "zombie-auto-renew-tests"))]
mod parachain_sync_storage;
#[cfg(any(feature = "zombie-sync-tests", feature = "zombie-auto-renew-tests"))]
mod utils;
