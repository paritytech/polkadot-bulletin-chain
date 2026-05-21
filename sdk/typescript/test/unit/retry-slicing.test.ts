// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Tests for the retry layer in `AsyncBulletinClient.uploadItemsImpl`:
 *   - on `BulletinError(STORE_STALLED, cause: { finalized: N })`, the retry
 *     re-invokes `pipelineStore` with `items.slice(N)`.
 *   - CIDs from all attempts are stitched into the final `cids: CID[]` in
 *     positional order.
 *   - Exponential backoff (1s/2s/4s) bounds the retry budget at 3.
 */

import { beforeEach, describe, expect, it, vi } from "vitest"
import * as pipelineModule from "../../src/pipeline"
import { BulletinError, ErrorCode } from "../../src/types"

// Cast helper for AsyncBulletinClient under test — we touch private impl.
async function makeClient() {
  const { AsyncBulletinClient } = await import("../../src/async-client")
  const signer = {
    publicKey: new Uint8Array(32),
    sign: async () => new Uint8Array(64),
  }
  const apiStub = { tx: { TransactionStorage: {} } }
  // biome-ignore lint/suspicious/noExplicitAny: tests touch private method directly
  return new AsyncBulletinClient(
    apiStub as any,
    signer as any,
    async () => ({}) as never,
  )
}

// Build a fake CID-like object (mockClient does Blake2b CIDs; we just need
// unique placeholders that pass through pipelineStore unchanged).
function fakeCid(label: string): unknown {
  return { toString: () => `cid:${label}`, label }
}

interface MockCall {
  itemsLength: number
  precomputedCidsLength?: number
}

describe("uploadItemsImpl retry slicing", () => {
  beforeEach(() => {
    vi.restoreAllMocks()
  })

  it("re-invokes pipelineStore with items.slice(finalized) on STORE_STALLED", async () => {
    const calls: MockCall[] = []
    const pipelineSpy = vi
      .spyOn(pipelineModule, "pipelineStore")
      // First attempt: pretend 3 items finalized then bail.
      .mockImplementationOnce(async (_api, _signer, items) => {
        calls.push({ itemsLength: items.length })
        const err = new BulletinError("stalled", ErrorCode.STORE_STALLED, {
          finalized: 3,
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
        return { cids: items.map((_, i) => fakeCid(`r${i}`)) } as never
      })

    const client = await makeClient()
    const items = Array.from({ length: 7 }, (_, i) => ({
      data: new Uint8Array([i]),
    }))
    // biome-ignore lint/suspicious/noExplicitAny: invoking private method
    const result = await (client as any).uploadItemsImpl(
      items,
      "finalized",
      undefined,
      false,
    )

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
        })
      })

    const client = await makeClient()
    const items = [{ data: new Uint8Array([1]) }]

    // biome-ignore lint/suspicious/noExplicitAny: invoking private method
    await expect(
      (client as any).uploadItemsImpl(items, "finalized", undefined, false),
    ).rejects.toMatchObject({ code: ErrorCode.STORE_STALLED })

    // 1 initial + 3 retries = 4 attempts total
    expect(pipelineSpy).toHaveBeenCalledTimes(4)
  }, 20_000) // 1s + 2s + 4s = 7s minimum of backoff

  it("non-stall BulletinError propagates without retry", async () => {
    const pipelineSpy = vi
      .spyOn(pipelineModule, "pipelineStore")
      .mockImplementation(async () => {
        throw new BulletinError("transient", ErrorCode.TRANSACTION_FAILED)
      })

    const client = await makeClient()
    const items = [{ data: new Uint8Array([1]) }]

    // biome-ignore lint/suspicious/noExplicitAny: invoking private method
    await expect(
      (client as any).uploadItemsImpl(items, "finalized", undefined, false),
    ).rejects.toMatchObject({ code: ErrorCode.TRANSACTION_FAILED })
    expect(pipelineSpy).toHaveBeenCalledTimes(1)
  })

  it("non-BulletinError errors get wrapped in TRANSACTION_FAILED", async () => {
    vi.spyOn(pipelineModule, "pipelineStore").mockImplementation(async () => {
      throw new Error("network is on fire")
    })

    const client = await makeClient()
    const items = [{ data: new Uint8Array([1]) }]

    // biome-ignore lint/suspicious/noExplicitAny: invoking private method
    await expect(
      (client as any).uploadItemsImpl(items, "finalized", undefined, false),
    ).rejects.toMatchObject({
      code: ErrorCode.TRANSACTION_FAILED,
      message: expect.stringContaining("network is on fire"),
    })
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
        return { cids: items.map((_, i) => fakeCid(`x${i}`)) } as never
      })

    const events: Array<{ index: number }> = []
    const client = await makeClient()
    const items = Array.from({ length: 5 }, (_, i) => ({
      data: new Uint8Array([i]),
    }))
    // biome-ignore lint/suspicious/noExplicitAny: invoking private method
    await (client as any).uploadItemsImpl(
      items,
      "finalized",
      (ev: { index: number }) => events.push({ index: ev.index }),
      false,
    )

    expect(events).toEqual([{ index: 0 }, { index: 2 }])
  })
})
