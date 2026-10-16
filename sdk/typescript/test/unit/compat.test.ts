// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * The compat registry's pinned checksums must derive from the same committed
 * metadata files that drive the Rust registry — recompute and compare, so a
 * snapshot regeneration or a papi checksum-algorithm change breaks here
 * rather than at runtime.
 */

import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { metadata } from "@polkadot-api/substrate-bindings"
import { describe, expect, it, vi } from "vitest"
import {
  AUTO_RENEW_REGISTRY,
  autoRenewPallet,
  RENEW_REGISTRY,
  type RenewPallet,
  renewalCallChecksum,
  renewalSupport,
} from "../../src/compat"
import { ErrorCode } from "../../src/types"

// test/unit/ → sdk/ — the same files `sdk/rust/src/compat.rs` embeds.
const metadataFile = (rel: string): Uint8Array =>
  readFileSync(fileURLToPath(new URL(`../../../${rel}`, import.meta.url)))

const CURRENT = "metadata.scale"
const PRE_SPLIT = "metadata-compat/transaction-storage-v1000016.scale"
const LEGACY = "metadata-compat/transaction-storage-v1000011.scale"

/**
 * A snapshot with the auto-renew calls removed from `pallet`: the shape of a
 * `TransactionRef` runtime from before those calls landed (they came after
 * `force_renew`). No such chain snapshot is committed.
 */
function withoutAutoRenewCalls(
  metadataBytes: Uint8Array,
  pallet: RenewPallet,
): Uint8Array {
  const decoded = metadata.dec(metadataBytes)
  if (decoded.metadata.tag !== "v16") throw new Error("expected V16 metadata")
  const v16 = decoded.metadata.value
  const callsType = v16.pallets.find((p) => p.name === pallet)?.calls?.type
  const callsEntry = v16.lookup.find((entry) => entry.id === callsType)
  if (callsEntry?.def.tag !== "variant") throw new Error("no call enum")
  callsEntry.def.value = callsEntry.def.value.filter(
    (variant) => !variant.name.endsWith("_auto_renew"),
  )
  return metadata.enc(decoded)
}

const contentHash = new Uint8Array(32).fill(2)
const contentHashHex = `0x${"02".repeat(32)}`

// Stub the PAPI client so the dispatch tests drive `client.renew()` without a
// chain: `createClient` hands back whatever `papiMock.current` holds.
const papiMock: { current: unknown } = { current: undefined }
vi.mock("polkadot-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("polkadot-api")>()
  return { ...actual, createClient: vi.fn(() => papiMock.current) }
})

/** A BulletinClient whose connected "chain" serves the given metadata bytes;
 *  the per-pallet `renew` spies capture the encoded arm, `submitTx`
 *  short-circuits submission. */
async function makeDispatchClient(metadataBytes: Uint8Array) {
  const renewTs = vi.fn(() => ({}) as never)
  const renewDr = vi.fn(() => ({}) as never)
  const forceRenewTs = vi.fn(() => ({}) as never)
  const enableTs = vi.fn(() => ({}) as never)
  const enableDr = vi.fn(() => ({}) as never)
  const disableTs = vi.fn(() => ({}) as never)
  const disableDr = vi.fn(() => ({}) as never)
  // The real unsafe api decodes OpaqueMetadata as plain bytes — the mock
  // must mirror that shape exactly (a `{ asBytes }` stub hid a live bug).
  const metadataAtVersion = vi.fn(async () => metadataBytes)
  papiMock.current = {
    getTypedApi: () => ({ tx: { TransactionStorage: {} } }),
    getUnsafeApi: () => ({
      apis: { Metadata: { metadata_at_version: metadataAtVersion } },
      tx: {
        TransactionStorage: {
          renew: renewTs,
          force_renew: forceRenewTs,
          enable_auto_renew: enableTs,
          disable_auto_renew: disableTs,
        },
        DataRenewal: {
          renew: renewDr,
          enable_auto_renew: enableDr,
          disable_auto_renew: disableDr,
        },
      },
    }),
    submitAndWatch: () => ({ subscribe: () => ({ unsubscribe() {} }) }),
    destroy: () => {},
  }
  const { BulletinClient } = await import("../../src/client")
  const client = new BulletinClient({
    descriptor: {},
    // biome-ignore lint/suspicious/noExplicitAny: stubbed provider
    providers: () => [{} as any],
  })
  const submitTx = vi.fn(async () => ({ blockHash: "0xb", txHash: "0xt" }))
  ;(client as unknown as { submitTx: unknown }).submitTx = submitTx
  return {
    client,
    renewTs,
    renewDr,
    forceRenewTs,
    enableTs,
    enableDr,
    disableTs,
    disableDr,
    metadataAtVersion,
    submitTx,
  }
}

describe("compat registry", () => {
  it("pins the checksum of every committed snapshot", () => {
    const current = renewalCallChecksum(
      metadataFile(CURRENT),
      "DataRenewal",
      "renew",
    )
    expect(current && RENEW_REGISTRY.DataRenewal[current]).toBe("data-renewal")
    // Post-split runtimes host no TransactionStorage.renew at all.
    expect(
      renewalCallChecksum(metadataFile(CURRENT), "TransactionStorage", "renew"),
    ).toBeNull()

    const preSplit = renewalCallChecksum(
      metadataFile(PRE_SPLIT),
      "TransactionStorage",
      "renew",
    )
    expect(preSplit && RENEW_REGISTRY.TransactionStorage[preSplit]).toBe(
      "transaction-ref",
    )

    const legacy = renewalCallChecksum(
      metadataFile(LEGACY),
      "TransactionStorage",
      "renew",
    )
    expect(legacy && RENEW_REGISTRY.TransactionStorage[legacy]).toBe(
      "positional",
    )
  })

  it("pins the auto-renew checksums of the snapshots that carry the calls", () => {
    for (const call of ["enable_auto_renew", "disable_auto_renew"] as const) {
      const current = renewalCallChecksum(
        metadataFile(CURRENT),
        "DataRenewal",
        call,
      )
      expect(AUTO_RENEW_REGISTRY.DataRenewal).toContain(current)
      const preSplit = renewalCallChecksum(
        metadataFile(PRE_SPLIT),
        "TransactionStorage",
        call,
      )
      expect(AUTO_RENEW_REGISTRY.TransactionStorage).toContain(preSplit)
      // Positional runtimes predate auto-renew.
      expect(
        renewalCallChecksum(metadataFile(LEGACY), "TransactionStorage", call),
      ).toBeNull()
    }
  })

  it("rows are complete and resolution round-trips", () => {
    expect(Object.keys(RENEW_REGISTRY.DataRenewal)).toHaveLength(1)
    expect(Object.keys(RENEW_REGISTRY.TransactionStorage)).toHaveLength(2)
    expect(renewalSupport(metadataFile(CURRENT)).shape).toBe("data-renewal")
    expect(renewalSupport(metadataFile(PRE_SPLIT)).shape).toBe(
      "transaction-ref",
    )
    expect(renewalSupport(metadataFile(LEGACY)).shape).toBe("positional")
  })

  // Auto-renew rides on the pallet hosting `renew`: `DataRenewal` after the
  // renewal split, `TransactionStorage` before it.
  it("resolves the auto-renew pallet from the renew pallet", () => {
    expect(
      autoRenewPallet(
        renewalSupport(metadataFile(CURRENT)),
        "enable_auto_renew",
      ),
    ).toBe("DataRenewal")
    expect(
      autoRenewPallet(
        renewalSupport(metadataFile(PRE_SPLIT)),
        "disable_auto_renew",
      ),
    ).toBe("TransactionStorage")
  })

  // Callers (e.g. the console UI) hide auto-renew on UNSUPPORTED_OPERATION.
  it("rejects auto-renew on runtimes without the calls", () => {
    const withoutCalls = renewalSupport(
      withoutAutoRenewCalls(metadataFile(PRE_SPLIT), "TransactionStorage"),
    )
    expect(withoutCalls.shape).toBe("transaction-ref")
    expect(() =>
      autoRenewPallet(withoutCalls, "enable_auto_renew"),
    ).toThrowError(
      expect.objectContaining({
        code: ErrorCode.UNSUPPORTED_OPERATION,
        message: "enable_auto_renew is not supported by this runtime",
      }),
    )
    expect(() =>
      autoRenewPallet(
        renewalSupport(metadataFile(LEGACY)),
        "disable_auto_renew",
      ),
    ).toThrowError(
      expect.objectContaining({
        code: ErrorCode.UNSUPPORTED_OPERATION,
        message: "disable_auto_renew is not supported by this runtime",
      }),
    )
  })
})

describe("client renew dispatch (mocked PAPI)", () => {
  it("encodes the DataRenewal arm on current metadata", async () => {
    const { client, renewTs, renewDr, submitTx } = await makeDispatchClient(
      metadataFile("metadata.scale"),
    )
    await client.renew({ block: 7, index: 3 }).send()
    expect(renewDr).toHaveBeenCalledWith({
      entry: { type: "Position", value: { block: 7, index: 3 } },
    })
    expect(renewTs).not.toHaveBeenCalled()
    expect(submitTx).toHaveBeenCalledTimes(1)
  })

  it("encodes the TransactionStorage arm on pre-split metadata", async () => {
    const { client, renewTs, renewDr } = await makeDispatchClient(
      metadataFile("metadata-compat/transaction-storage-v1000016.scale"),
    )
    await client.renew({ block: 7, index: 3 }).send()
    expect(renewTs).toHaveBeenCalledWith({
      entry: { type: "Position", value: { block: 7, index: 3 } },
    })
    expect(renewDr).not.toHaveBeenCalled()
  })

  it("encodes the positional arm on legacy metadata", async () => {
    const { client, renewTs, renewDr } = await makeDispatchClient(
      metadataFile("metadata-compat/transaction-storage-v1000011.scale"),
    )
    await client.renew({ block: 7, index: 3 }).send()
    expect(renewTs).toHaveBeenCalledWith({ block: 7, index: 3 })
    expect(renewDr).not.toHaveBeenCalled()
  })

  it("resolves the shape once per client", async () => {
    const { client, metadataAtVersion } = await makeDispatchClient(
      metadataFile("metadata.scale"),
    )
    await client.renew({ block: 1, index: 0 }).send()
    await client.renew({ block: 2, index: 0 }).send()
    await client.enableAutoRenew(contentHash).send()
    expect(metadataAtVersion).toHaveBeenCalledTimes(1)
  })

  it("encodes DataRenewal auto-renew calls on current metadata", async () => {
    const { client, enableDr, disableDr, enableTs, submitTx } =
      await makeDispatchClient(metadataFile(CURRENT))
    await client.enableAutoRenew(contentHash).send()
    await client.disableAutoRenew(contentHash).send()
    expect(enableDr).toHaveBeenCalledWith({ content_hash: contentHashHex })
    expect(disableDr).toHaveBeenCalledWith({ content_hash: contentHashHex })
    expect(enableTs).not.toHaveBeenCalled()
    expect(submitTx).toHaveBeenCalledTimes(2)
  })

  it("encodes TransactionStorage auto-renew calls on pre-split metadata", async () => {
    const { client, enableTs, disableTs, enableDr } = await makeDispatchClient(
      metadataFile(PRE_SPLIT),
    )
    await client.enableAutoRenew(contentHash).send()
    await client.disableAutoRenew(contentHash).send()
    expect(enableTs).toHaveBeenCalledWith({ content_hash: contentHashHex })
    expect(disableTs).toHaveBeenCalledWith({ content_hash: contentHashHex })
    expect(enableDr).not.toHaveBeenCalled()
  })

  // A missing auto-renew call must not block the other renewal calls.
  it("rejects auto-renew but still renews without the auto-renew calls", async () => {
    const { client, enableTs, forceRenewTs, submitTx } =
      await makeDispatchClient(
        withoutAutoRenewCalls(metadataFile(PRE_SPLIT), "TransactionStorage"),
      )
    await expect(
      client.enableAutoRenew(contentHash).send(),
    ).rejects.toMatchObject({ code: ErrorCode.UNSUPPORTED_OPERATION })
    expect(enableTs).not.toHaveBeenCalled()
    expect(submitTx).not.toHaveBeenCalled()

    await client.forceRenew(contentHash).send()
    expect(forceRenewTs).toHaveBeenCalledWith({
      entry: { type: "ContentHash", value: contentHashHex },
    })
  })

  it("rejects auto-renew on legacy metadata", async () => {
    const { client, disableTs, submitTx } = await makeDispatchClient(
      metadataFile(LEGACY),
    )
    await expect(
      client.disableAutoRenew(contentHash).send(),
    ).rejects.toMatchObject({ code: ErrorCode.UNSUPPORTED_OPERATION })
    expect(disableTs).not.toHaveBeenCalled()
    expect(submitTx).not.toHaveBeenCalled()
  })
})
