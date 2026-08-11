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

/** Supported shapes of the renewal calls across the fleet. */
export type RenewShape = "data-renewal" | "transaction-ref" | "positional"

/** Pallets that may host the renewal calls, newest first. */
const RENEW_PALLETS = ["DataRenewal", "TransactionStorage"] as const
export type RenewPallet = (typeof RENEW_PALLETS)[number]

/**
 * pallet → checksum → shape (see module docs for how keys are derived and
 * verified). Keys pair the hosting pallet with the checksum: PAPI's per-item
 * checksum covers only the call's type tree, and the renewal split moved
 * `renew` across pallets without changing that tree, so the same checksum
 * appears under both pallets and means a different encoder in each.
 */
export const RENEW_REGISTRY: Readonly<
  Record<RenewPallet, Readonly<Record<string, RenewShape>>>
> = {
  DataRenewal: {
    // sdk/metadata.scale — current runtime, post renewal split:
    // `DataRenewal.renew(entry: TransactionRef)`.
    a4vk5ap2ldpq: "data-renewal",
  },
  TransactionStorage: {
    // sdk/metadata-compat/transaction-storage-v1000016.scale — pre-split
    // chains: `TransactionStorage.renew(entry: TransactionRef)`.
    a4vk5ap2ldpq: "transaction-ref",
    // sdk/metadata-compat/transaction-storage-v1000011.scale —
    // bulletin-westend v1000011: positional `renew(block, index)`.
    eq2g3ci5e7ion: "positional",
  },
}

/**
 * Checksum of `<pallet>.renew` in opaque metadata bytes; `null` when the
 * pallet or call is absent.
 */
export function renewChecksum(
  metadataBytes: Uint8Array,
  pallet: RenewPallet = "TransactionStorage",
): string | null {
  const unified = unifyMetadata(decAnyMetadata(metadataBytes))
  return getChecksumBuilder(getLookupFn(unified)).buildCall(pallet, "renew")
}

/**
 * Resolve the `renew` encoder shape for the connected chain: the newest
 * pallet hosting a `renew` call decides. Fails closed on an absent or
 * unknown shape.
 */
export function resolveRenewShape(metadataBytes: Uint8Array): RenewShape {
  const unified = unifyMetadata(decAnyMetadata(metadataBytes))
  const builder = getChecksumBuilder(getLookupFn(unified))
  for (const pallet of RENEW_PALLETS) {
    const checksum = builder.buildCall(pallet, "renew")
    if (checksum === null) continue
    const shape = RENEW_REGISTRY[pallet][checksum]
    if (!shape) {
      throw new BulletinError(
        `${pallet}.renew has an unsupported shape on this chain (checksum ${checksum}); this SDK release supports ${RENEW_PALLETS.flatMap((p) => Object.keys(RENEW_REGISTRY[p])).length} shape(s) — a newer runtime may need an SDK upgrade`,
        ErrorCode.UNSUPPORTED_OPERATION,
      )
    }
    return shape
  }
  throw new BulletinError(
    "renew is not available on this chain",
    ErrorCode.UNSUPPORTED_OPERATION,
  )
}
