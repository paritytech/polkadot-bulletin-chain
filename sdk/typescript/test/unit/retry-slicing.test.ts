// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the retry layer in `BulletinClient.runSignedRetry` (driven via
 * the `submit(estimate, source)` API):
 *   - on `BulletinError(STORE_STALLED, cause: { finalizedIndices })`, the
 *     retry re-invokes `pipelineStore` skipping those specific original
 *     indices (NOT slicing by count — items can land non-contiguously
 *     under hijack races).
 *   - CIDs from all attempts are stitched into the final `cids: CID[]` in
 *     positional order.
 *   - Exponential backoff (1s/2s/4s) bounds the retry budget at 3.
 *   - Exactly-once broadcast: items that landed in a best block before
 *     STORE_STALLED fired (and thus are NOT in `finalizedIndices`) get
 *     deduped via a TBCH pre-check before the retry runs, so the retry
 *     does NOT re-broadcast them.
 */

import { beforeEach, describe, expect, it, vi } from "vitest"
import { blobFromItems } from "../../src/blob-source"
import * as pipelineModule from "../../src/pipeline"
import { BulletinError, ErrorCode, UploadStatus } from "../../src/types"

// Stub the underlying PAPI client so unit tests don't open a real
// connection. createClient(provider).getTypedApi(descriptor) ⇒ apiStub.
const STUB_API = { tx: { TransactionStorage: {} } }
vi.mock("polkadot-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("polkadot-api")>()
  return {
    ...actual,
    createClient: vi.fn(() => ({
      getTypedApi: () => STUB_API,
      submitAndWatch: () => ({ subscribe: () => ({ unsubscribe() {} }) }),
      destroy: () => {},
    })),
  }
})

// Cast helper for BulletinClient under test — we touch private impl.
async function makeClient() {
  const { BulletinClient } = await import("../../src/client")
  const signer = {
    publicKey: new Uint8Array(32),
    sign: async () => new Uint8Array(64),
  }
  return new BulletinClient({
    descriptor: {}, // any value — mocked getTypedApi ignores it
    // biome-ignore lint/suspicious/noExplicitAny: stubbed provider; pipelineStore is mocked
    providers: () => [{} as any],
    // biome-ignore lint/suspicious/noExplicitAny: structural stub
    uploadSigner: signer as any,
  })
}

// Build a fake CID-like object (mockClient does Blake2b CIDs; we just need
// unique placeholders that pass through pipelineStore unchanged).
function fakeCid(label: string): unknown {
  return { toString: () => `cid:${label}`, label }
}

// Route items through the sole submission API. estimateUpload computes the
// real per-item plan/CIDs offline (no chain); submit → runSignedRetry hits
// the mocked pipelineStore, exercising the retry/slicing path in
// runSignedRetry.
async function runSubmit(
  // biome-ignore lint/suspicious/noExplicitAny: BulletinClient under test
  client: any,
  items: { data: Uint8Array }[],
  onEvent?: (ev: { type: string; index: number }) => void,
) {
  const builder = client.submit(
    await client.estimateUpload(items),
    blobFromItems(items),
  )
  if (onEvent) builder.withCallback(onEvent)
  return builder.send()
}

interface MockCall {
  itemsLength: number
  precomputedCidsLength?: number
}

describe("submit() retry slicing", () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it("re-invokes pipelineStore skipping finalized indices on STORE_STALLED", async () => {
    const calls: MockCall[] = []
    const pipelineSpy = vi
      .spyOn(pipelineModule, "pipelineStore")
      // First attempt: pretend items 0,1,2 finalized then bail.
      .mockImplementationOnce(async (_api, _signer, items) => {
        calls.push({ itemsLength: items.length })
        const err = new BulletinError("stalled", ErrorCode.STORE_STALLED, {
          finalized: 3,
          finalizedIndices: new Set([0, 1, 2]),
        })
        throw err
      })
      // Second attempt: succeed with the remaining items.
      // biome-ignore lint/suspicious/noExplicitAny: minimal mock surface
      .mockImplementationOnce(async (_api, _signer, items, config: any) => {
        calls.push({
          itemsLength: items.length,
          precomputedCidsLength: config.precomputedCids?.length,
        })
        return {
          cids: items.map((_, i) => fakeCid(`r${i}`)),
          failed: [],
        } as never
      })

    const client = await makeClient()
    const items = Array.from({ length: 7 }, (_, i) => ({
      data: new Uint8Array([i]),
    }))
    const result = await runSubmit(client, items)

    expect(pipelineSpy).toHaveBeenCalledTimes(2)
    expect(calls).toEqual([
      { itemsLength: 7 }, // first attempt: full set
      { itemsLength: 4, precomputedCidsLength: 4 }, // second: 7 - 3 = 4
    ])
    expect(result.cids).toHaveLength(7)
  })

  it("gives up after 3 retries (4 total attempts)", async () => {
    const pipelineSpy = vi
      .spyOn(pipelineModule, "pipelineStore")
      .mockImplementation(async () => {
        throw new BulletinError("stalled", ErrorCode.STORE_STALLED, {
          finalized: 0,
          finalizedIndices: new Set<number>(),
        })
      })

    const client = await makeClient()
    const items = [{ data: new Uint8Array([1]) }]

    await expect(runSubmit(client, items)).rejects.toMatchObject({
      code: ErrorCode.STORE_STALLED,
    })

    // 1 initial + 3 retries = 4 attempts total
    expect(pipelineSpy).toHaveBeenCalledTimes(4)
  }, 20_000) // 1s + 2s + 4s = 7s minimum of backoff

  it("throws UPLOAD_INCOMPLETE when the pipeline completes with failed items", async () => {
    // The pipeline resolves (it completed the rest) but reports items 1 and 3
    // as permanently failed — submit() must surface that, not return all cids.
    const pipelineSpy = vi
      .spyOn(pipelineModule, "pipelineStore")
      .mockImplementationOnce(async (_api, _signer, items) => {
        return {
          cids: items.map((_, i) => fakeCid(`r${i}`)),
          failed: [1, 3],
        } as never
      })

    const client = await makeClient()
    const items = Array.from({ length: 5 }, (_, i) => ({
      data: new Uint8Array([i]),
    }))

    await expect(runSubmit(client, items)).rejects.toMatchObject({
      code: ErrorCode.UPLOAD_INCOMPLETE,
      cause: { failedIndices: [1, 3] },
    })
    // Completed-with-failures is not a stall — no retry.
    expect(pipelineSpy).toHaveBeenCalledTimes(1)
  })

  it("non-stall BulletinError propagates without retry", async () => {
    const pipelineSpy = vi
      .spyOn(pipelineModule, "pipelineStore")
      .mockImplementation(async () => {
        throw new BulletinError("transient", ErrorCode.TRANSACTION_FAILED)
      })

    const client = await makeClient()
    const items = [{ data: new Uint8Array([1]) }]

    await expect(runSubmit(client, items)).rejects.toMatchObject({
      code: ErrorCode.TRANSACTION_FAILED,
    })
    expect(pipelineSpy).toHaveBeenCalledTimes(1)
  })

  it("non-BulletinError errors get wrapped in TRANSACTION_FAILED", async () => {
    vi.spyOn(pipelineModule, "pipelineStore").mockImplementation(async () => {
      throw new Error("network is on fire")
    })

    const client = await makeClient()
    const items = [{ data: new Uint8Array([1]) }]

    await expect(runSubmit(client, items)).rejects.toMatchObject({
      code: ErrorCode.TRANSACTION_FAILED,
      message: expect.stringContaining("network is on fire"),
    })
  })

  it("handles non-contiguous finalization (hijack-race scenario)", async () => {
    // Regression test: previously the retry sliced by COUNT, assuming
    // finalized items were always indices [0..N). Under hijack races,
    // finalization can be sparse (e.g. items 8..15 finalize while 0..7
    // are stuck in pool). The retry must use the explicit
    // `finalizedIndices` set, not a count, or it would (a) re-submit
    // already-finalized items and (b) drop the stuck ones.
    const calls: Array<{ itemsLength: number; firstDataByte: number }> = []
    vi.spyOn(pipelineModule, "pipelineStore")
      .mockImplementationOnce(async (_api, _signer, items) => {
        calls.push({
          itemsLength: items.length,
          firstDataByte: (
            await (items[0] as { getData(): Promise<Uint8Array> }).getData()
          )[0] as number,
        })
        // Pretend items 8..15 finalized (middle 8 of 16); items 0..7
        // and 15..15 are still pending.
        const finalizedIndices = new Set([8, 9, 10, 11, 12, 13, 14, 15])
        throw new BulletinError("stalled", ErrorCode.STORE_STALLED, {
          finalized: finalizedIndices.size,
          finalizedIndices,
        })
      })
      // biome-ignore lint/suspicious/noExplicitAny: minimal mock surface
      .mockImplementationOnce(async (_api, _signer, items, _config: any) => {
        calls.push({
          itemsLength: items.length,
          firstDataByte: (
            await (items[0] as { getData(): Promise<Uint8Array> }).getData()
          )[0] as number,
        })
        return {
          cids: items.map((_, i) => fakeCid(`r${i}`)),
          failed: [],
        } as never
      })

    const client = await makeClient()
    // 16 items where item.data[0] === index so we can verify which were resent.
    const items = Array.from({ length: 16 }, (_, i) => ({
      data: new Uint8Array([i]),
    }))
    const result = await runSubmit(client, items)

    expect(calls).toHaveLength(2)
    expect(calls[0]).toEqual({ itemsLength: 16, firstDataByte: 0 })
    // Retry: only items 0..7 (8 items), NOT including the 8 finalized ones.
    expect(calls[1]).toEqual({ itemsLength: 8, firstDataByte: 0 })
    expect(result.cids).toHaveLength(16)
  })

  it("translates per-attempt indices to absolute indices in onEvent", async () => {
    vi.spyOn(pipelineModule, "pipelineStore")
      .mockImplementationOnce(async (_api, _signer, items, config) => {
        // emit ItemStarted for indices 0..items.length-1 inside this attempt
        config.onEvent?.({
          // biome-ignore lint/suspicious/noExplicitAny: minimal event surface
          type: "item_started" as any,
          index: 0,
          total: items.length,
          cid: fakeCid("a") as never,
        })
        throw new BulletinError("stalled", ErrorCode.STORE_STALLED, {
          finalized: 2,
          finalizedIndices: new Set([0, 1]),
        })
      })
      .mockImplementationOnce(async (_api, _signer, items, config) => {
        // emit ItemStarted for index 0 of the SECOND attempt → should
        // surface as absolute index = alreadyFinalized + 0 = 2
        config.onEvent?.({
          // biome-ignore lint/suspicious/noExplicitAny: minimal event surface
          type: "item_started" as any,
          index: 0,
          total: items.length,
          cid: fakeCid("b") as never,
        })
        return {
          cids: items.map((_, i) => fakeCid(`x${i}`)),
          failed: [],
        } as never
      })

    const events: Array<{ index: number }> = []
    const client = await makeClient()
    const items = Array.from({ length: 5 }, (_, i) => ({
      data: new Uint8Array([i]),
    }))
    await runSubmit(client, items, (ev: { index: number }) =>
      events.push({ index: ev.index }),
    )

    expect(events).toEqual([{ index: 0 }, { index: 2 }])
  })

  it("dedups via TBCH before retry: items already on chain are not re-broadcast", async () => {
    // Scenario: 7 items broadcast.
    //   - First pipelineStore attempt finalizes items 0, 1.
    //   - Item 2 landed in a best block but the watchdog fired STORE_STALLED
    //     before finalization — so it is NOT in `finalizedIndices`.
    //   - Items 3..6 are still pending.
    //
    // Between the stall and the retry, the chain finalizes item 2's block.
    // The retry path must:
    //   * Query TBCH for items not in finalizedIndices.
    //   * Find item 2 on chain → mark as finalized, emit ItemFinalized.
    //   * Invoke pipelineStore with ONLY items 3..6 (4 items), NEVER
    //     re-broadcasting item 2.
    const calls: Array<{ itemsLength: number }> = []

    // Mock readStoredAt by call order — runSignedRetry queries each
    // pending item's TBCH once on retry, in original-index order.
    // Return an entry only for the FIRST call (item 2's hash), undefined
    // for items 3..6.
    const callOrder: string[] = []
    vi.spyOn(pipelineModule, "readStoredAt").mockImplementation(
      // biome-ignore lint/suspicious/noExplicitAny: minimal type surface
      async (_api: any, contentHashHex: string) => {
        callOrder.push(contentHashHex)
        if (callOrder.length === 1) {
          return { blockNumber: 100, transactionIndex: 7 }
        }
        return undefined
      },
    )

    vi.spyOn(pipelineModule, "pipelineStore")
      .mockImplementationOnce(async (_api, _signer, items) => {
        calls.push({ itemsLength: items.length })
        throw new BulletinError("stalled", ErrorCode.STORE_STALLED, {
          finalized: 2,
          finalizedIndices: new Set([0, 1]),
        })
      })
      .mockImplementationOnce(async (_api, _signer, items, _config: any) => {
        calls.push({ itemsLength: items.length })
        return {
          cids: items.map((_, i) => fakeCid(`r${i}`)),
          failed: [],
        } as never
      })

    const events: Array<{ type: string; index: number }> = []
    const client = await makeClient()
    const items = Array.from({ length: 7 }, (_, i) => ({
      data: new Uint8Array([i]),
    }))
    await runSubmit(client, items, (ev: { type: string; index: number }) =>
      events.push({ type: ev.type, index: ev.index }),
    )

    expect(calls).toEqual([
      { itemsLength: 7 }, // first attempt: all 7
      { itemsLength: 4 }, // retry: items 0,1 (from stall) + 2 (from TBCH) excluded → 4 left
    ])
    // Synthetic ItemFinalized for item 2 emitted by the TBCH dedup path.
    const finalizedFromTbch = events.filter(
      (e) => e.type === UploadStatus.ItemFinalized && e.index === 2,
    )
    expect(finalizedFromTbch).toHaveLength(1)
    // 5 TBCH queries should have happened (items 2..6 in order; items
    // 0,1 already in finalizedOriginal from the stall and not queried).
    expect(callOrder).toHaveLength(5)
  })
})
