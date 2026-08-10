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
import { describe, expect, it, vi } from "vitest"
import {
  RENEW_REGISTRY,
  renewChecksum,
  resolveRenewShape,
} from "../../src/compat"

// test/unit/ → sdk/ — the same files `sdk/rust/src/compat.rs` embeds.
const metadataFile = (rel: string): Uint8Array =>
  readFileSync(fileURLToPath(new URL(`../../../${rel}`, import.meta.url)))

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
  // The real unsafe api decodes OpaqueMetadata as plain bytes — the mock
  // must mirror that shape exactly (a `{ asBytes }` stub hid a live bug).
  const metadataAtVersion = vi.fn(async () => metadataBytes)
  papiMock.current = {
    getTypedApi: () => ({ tx: { TransactionStorage: {} } }),
    getUnsafeApi: () => ({
      apis: { Metadata: { metadata_at_version: metadataAtVersion } },
      tx: {
        TransactionStorage: { renew: renewTs },
        DataRenewal: { renew: renewDr },
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
  return { client, renewTs, renewDr, metadataAtVersion, submitTx }
}

describe("compat registry", () => {
  it("pins the checksum of every committed snapshot", () => {
    const current = renewChecksum(metadataFile("metadata.scale"), "DataRenewal")
    expect(current && RENEW_REGISTRY.DataRenewal[current]).toBe("data-renewal")
    // Post-split runtimes host no TransactionStorage.renew at all.
    expect(renewChecksum(metadataFile("metadata.scale"))).toBeNull()

    const preSplit = renewChecksum(
      metadataFile("metadata-compat/transaction-storage-v1000016.scale"),
    )
    expect(preSplit && RENEW_REGISTRY.TransactionStorage[preSplit]).toBe(
      "transaction-ref",
    )

    const legacy = renewChecksum(
      metadataFile("metadata-compat/transaction-storage-v1000011.scale"),
    )
    expect(legacy && RENEW_REGISTRY.TransactionStorage[legacy]).toBe(
      "positional",
    )
  })

  it("rows are complete and resolution round-trips", () => {
    expect(Object.keys(RENEW_REGISTRY.DataRenewal)).toHaveLength(1)
    expect(Object.keys(RENEW_REGISTRY.TransactionStorage)).toHaveLength(2)
    expect(resolveRenewShape(metadataFile("metadata.scale"))).toBe(
      "data-renewal",
    )
    expect(
      resolveRenewShape(
        metadataFile("metadata-compat/transaction-storage-v1000016.scale"),
      ),
    ).toBe("transaction-ref")
    expect(
      resolveRenewShape(
        metadataFile("metadata-compat/transaction-storage-v1000011.scale"),
      ),
    ).toBe("positional")
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
    expect(metadataAtVersion).toHaveBeenCalledTimes(1)
  })
})
