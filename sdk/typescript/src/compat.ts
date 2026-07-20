// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Version dispatch for items that changed incompatibly across the live fleet.
 *
 * Mirror of the Rust SDK's `compat.rs` registry: dispatch is keyed by PAPI's
 * per-item checksum (the same construction descriptors use), computed from
 * the CONNECTED chain's metadata and looked up — identification first, never
 * trial-encoding, so overlapping candidate shapes cannot mis-select. An
 * unknown checksum fails closed — deliberately strict: even a wire-compatible
 * evolution of the item's type tree (e.g. an added enum variant) needs a new
 * registry row. Structure cannot see semantics: a change that keeps the type
 * tree identical but changes meaning would need an explicit spec-version
 * override row — none exist today.
 *
 * The pinned checksums derive from the same committed metadata files that
 * drive the Rust registry (`sdk/metadata.scale`, `sdk/metadata-compat/*`);
 * `test/unit/compat.test.ts` recomputes them from those files, so a snapshot
 * regeneration or a papi checksum-algorithm change breaks tests, not runtime.
 * Snapshot inventory: `sdk/metadata-compat/README.md`.
 */

import {
  getChecksumBuilder,
  getLookupFn,
} from "@polkadot-api/metadata-builders"
import { decAnyMetadata, unifyMetadata } from "@polkadot-api/substrate-bindings"
import { BulletinError, ErrorCode } from "./types.js"

/** Supported shapes of `TransactionStorage.renew` across the fleet. */
export type RenewShape = "transaction-ref" | "positional"

/** checksum → shape (see module docs for how keys are derived and verified). */
export const RENEW_REGISTRY: Readonly<Record<string, RenewShape>> = {
  // sdk/metadata.scale — current runtime: `renew(entry: TransactionRef)`.
  a4vk5ap2ldpq: "transaction-ref",
  // sdk/metadata-compat/transaction-storage-v1000011.scale —
  // bulletin-westend v1000011: positional `renew(block, index)`.
  eq2g3ci5e7ion: "positional",
}

/**
 * Checksum of `TransactionStorage.renew` in opaque metadata bytes; `null`
 * when the pallet or call is absent.
 */
export function renewChecksum(metadataBytes: Uint8Array): string | null {
  const unified = unifyMetadata(decAnyMetadata(metadataBytes))
  return getChecksumBuilder(getLookupFn(unified)).buildCall(
    "TransactionStorage",
    "renew",
  )
}

/**
 * Resolve the `renew` encoder shape for the connected chain. Fails closed on
 * an absent or unknown shape.
 */
export function resolveRenewShape(metadataBytes: Uint8Array): RenewShape {
  const checksum = renewChecksum(metadataBytes)
  if (checksum === null) {
    throw new BulletinError(
      "TransactionStorage.renew is not available on this chain",
      ErrorCode.UNSUPPORTED_OPERATION,
    )
  }
  const shape = RENEW_REGISTRY[checksum]
  if (!shape) {
    throw new BulletinError(
      `TransactionStorage.renew has an unsupported shape on this chain (checksum ${checksum}); this SDK release supports ${Object.keys(RENEW_REGISTRY).length} shape(s) — a newer runtime may need an SDK upgrade`,
      ErrorCode.UNSUPPORTED_OPERATION,
    )
  }
  return shape
}
