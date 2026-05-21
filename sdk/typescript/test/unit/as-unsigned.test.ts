// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Tests for the `asUnsigned()` builder path. Routes through PAPI's
 * `submitAndWatch` (Observable) — emits ItemStarted before subscribing,
 * ItemInBlock at `txBestBlocksState.found=true`, and ItemFinalized at
 * `finalized`. Resolves at whichever matches `waitFor`.
 */

import { describe, expect, it, vi } from "vitest"
import { BulletinError, ErrorCode, UploadStatus } from "../../src/types"

interface ScriptedEvent {
  type: string
  found?: boolean
  block?: { hash: string; number: number }
}

/**
 * Build a client backed by a scripted `submitAndWatch` mock. Each call
 * to `submitAndWatch(bareTx)` will replay the same scripted events on
 * the next microtask.
 */
async function makeClient(
  events: ScriptedEvent[],
  options: { withSigner?: boolean } = {},
) {
  const { AsyncBulletinClient } = await import("../../src/async-client")
  const apiStub = {
    tx: {
      TransactionStorage: {
        store: vi.fn(() => ({
          getBareTx: async () => new Uint8Array([1, 2, 3]),
        })),
        store_with_cid_config: vi.fn(() => ({
          getBareTx: async () => new Uint8Array([4, 5, 6]),
        })),
      },
    },
  }
  // biome-ignore lint/suspicious/noExplicitAny: minimal observable stub
  const submitAndWatch: any = vi.fn(() => ({
    subscribe(observer: {
      next: (ev: unknown) => void
      error: (err: unknown) => void
    }) {
      queueMicrotask(() => {
        for (const ev of events) observer.next(ev)
      })
      return { unsubscribe: () => {} }
    },
  }))
  const signer = options.withSigner
    ? { publicKey: new Uint8Array(32), sign: async () => new Uint8Array(64) }
    : undefined
  // biome-ignore lint/suspicious/noExplicitAny: tests touch private internals
  const client = new AsyncBulletinClient(
    apiStub as any,
    signer as any,
    submitAndWatch,
  )
  return { client, submitAndWatch }
}

const FINALIZED_SCRIPT: ScriptedEvent[] = [
  { type: "txBestBlocksState", found: true, block: { hash: "0xabc", number: 42 } },
  { type: "finalized", block: { hash: "0xabc", number: 42 } },
]

describe("UploadBuilder.asUnsigned()", () => {
  it("uses submitAndWatch (not chainHead pipeline)", async () => {
    const { client, submitAndWatch } = await makeClient(FINALIZED_SCRIPT)
    const { cids } = await client
      .upload([{ data: new Uint8Array([7, 8, 9]) }])
      .asUnsigned()
      .send()
    expect(submitAndWatch).toHaveBeenCalledTimes(1)
    expect(cids).toHaveLength(1)
  })

  it("emits ItemStarted + ItemInBlock + ItemFinalized through the callback", async () => {
    const { client } = await makeClient(FINALIZED_SCRIPT)
    const events: Array<{ type: UploadStatus; blockNumber?: number }> = []
    await client
      .upload([{ data: new Uint8Array([1, 2]) }])
      .asUnsigned()
      .withCallback((ev) => {
        events.push({
          type: ev.type,
          blockNumber: "blockNumber" in ev ? ev.blockNumber : undefined,
        })
      })
      .send()
    expect(events.map((e) => e.type)).toEqual([
      UploadStatus.ItemStarted,
      UploadStatus.ItemInBlock,
      UploadStatus.ItemFinalized,
    ])
    expect(events[1]?.blockNumber).toBe(42)
    expect(events[2]?.blockNumber).toBe(42)
  })

  it("submits N items in parallel, one subscription per item", async () => {
    const { client, submitAndWatch } = await makeClient(FINALIZED_SCRIPT)
    const items = Array.from({ length: 5 }, (_, i) => ({
      data: new Uint8Array([i]),
    }))
    const events: Array<{ type: UploadStatus; index: number }> = []

    const { cids } = await client
      .upload(items)
      .asUnsigned()
      .withCallback((ev) => events.push({ type: ev.type, index: ev.index }))
      .send()

    expect(cids).toHaveLength(5)
    expect(submitAndWatch).toHaveBeenCalledTimes(5)
    // 5 items × 3 events each = 15
    expect(events).toHaveLength(15)
    for (let i = 0; i < 5; i++) {
      const forIndex = events.filter((e) => e.index === i)
      expect(forIndex.map((e) => e.type)).toEqual([
        UploadStatus.ItemStarted,
        UploadStatus.ItemInBlock,
        UploadStatus.ItemFinalized,
      ])
    }
  })

  it("resolves at txBestBlocksState when waitFor='in_block'", async () => {
    const { client } = await makeClient([
      { type: "txBestBlocksState", found: true, block: { hash: "0xabc", number: 42 } },
      // finalized would come later — promise should already resolve before this
    ])
    const events: Array<{ type: UploadStatus; blockNumber?: number }> = []

    const { cids } = await client
      .upload([{ data: new Uint8Array([1]) }])
      .asUnsigned()
      .withWaitFor("in_block")
      .withCallback((ev) =>
        events.push({
          type: ev.type,
          blockNumber: "blockNumber" in ev ? ev.blockNumber : undefined,
        }),
      )
      .send()

    expect(cids).toHaveLength(1)
    expect(events.map((e) => e.type)).toEqual([
      UploadStatus.ItemStarted,
      UploadStatus.ItemInBlock,
    ])
    expect(events[1]?.blockNumber).toBe(42)
  })

  it("emits InBlock + Finalized with distinct block info when waitFor='finalized'", async () => {
    const { client } = await makeClient([
      { type: "txBestBlocksState", found: true, block: { hash: "0xabc", number: 42 } },
      { type: "finalized", block: { hash: "0xdef", number: 50 } },
    ])
    const events: Array<{ type: UploadStatus; blockNumber?: number }> = []

    await client
      .upload([{ data: new Uint8Array([1]) }])
      .asUnsigned()
      .withWaitFor("finalized")
      .withCallback((ev) =>
        events.push({
          type: ev.type,
          blockNumber: "blockNumber" in ev ? ev.blockNumber : undefined,
        }),
      )
      .send()

    expect(events).toEqual([
      { type: UploadStatus.ItemStarted, blockNumber: undefined },
      { type: UploadStatus.ItemInBlock, blockNumber: 42 },
      { type: UploadStatus.ItemFinalized, blockNumber: 50 },
    ])
  })

  it("rejects on 'invalid' event with TRANSACTION_FAILED + ItemFailed", async () => {
    const { client } = await makeClient([{ type: "invalid" }])
    const events: UploadStatus[] = []
    await expect(
      client
        .upload([{ data: new Uint8Array([1]) }])
        .asUnsigned()
        .withCallback((ev) => events.push(ev.type))
        .send(),
    ).rejects.toMatchObject({ code: ErrorCode.TRANSACTION_FAILED })
    expect(events).toEqual([UploadStatus.ItemStarted, UploadStatus.ItemFailed])
  })

  it("rejects on 'dropped' event with TRANSACTION_FAILED + ItemFailed", async () => {
    const { client } = await makeClient([{ type: "dropped" }])
    const events: UploadStatus[] = []
    await expect(
      client
        .upload([{ data: new Uint8Array([1]) }])
        .asUnsigned()
        .withCallback((ev) => events.push(ev.type))
        .send(),
    ).rejects.toBeInstanceOf(BulletinError)
    expect(events).toEqual([UploadStatus.ItemStarted, UploadStatus.ItemFailed])
  })
})

describe("Signer-less client", () => {
  it("can be constructed without a signer", async () => {
    const { client } = await makeClient(FINALIZED_SCRIPT, { withSigner: false })
    expect(client.signer).toBeUndefined()
  })

  it("asUnsigned() works on a signer-less client", async () => {
    const { client } = await makeClient(FINALIZED_SCRIPT, { withSigner: false })
    const { cids } = await client
      .upload([{ data: new Uint8Array([1]) }])
      .asUnsigned()
      .send()
    expect(cids).toHaveLength(1)
  })

  it("signed upload() throws UNSUPPORTED_OPERATION on a signer-less client", async () => {
    const { client, submitAndWatch } = await makeClient(FINALIZED_SCRIPT, {
      withSigner: false,
    })
    await expect(
      client.upload([{ data: new Uint8Array([1]) }]).send(),
    ).rejects.toMatchObject({ code: ErrorCode.UNSUPPORTED_OPERATION })
    expect(submitAndWatch).not.toHaveBeenCalled()
  })
})

describe("Missing submitAndWatch", () => {
  it("asUnsigned() throws UNSUPPORTED_OPERATION when constructed without submitAndWatch", async () => {
    const { AsyncBulletinClient } = await import("../../src/async-client")
    const apiStub = {
      tx: {
        TransactionStorage: {
          store: () => ({ getBareTx: async () => new Uint8Array([1]) }),
        },
      },
    }
    // biome-ignore lint/suspicious/noExplicitAny: test stub
    const client = new AsyncBulletinClient(apiStub as any, undefined, undefined)
    await expect(
      client.upload([{ data: new Uint8Array([1]) }]).asUnsigned().send(),
    ).rejects.toMatchObject({ code: ErrorCode.UNSUPPORTED_OPERATION })
  })
})

describe("ensureAuthorized() + asUnsigned()", () => {
  /**
   * Build a client with a scripted `Authorizations.getValue` so we can
   * test the preimage pre-flight without a chain.
   */
  async function makePreimageClient(opts: {
    auth?: { extent: { transactions: number; bytes: bigint }; expiration: number } | undefined
    currentBlock?: number
    onQuery?: (scope: unknown) => void
  }) {
    const { AsyncBulletinClient } = await import("../../src/async-client")
    // biome-ignore lint/suspicious/noExplicitAny: minimal observable stub
    const submitAndWatch: any = vi.fn(() => ({
      subscribe(observer: { next: (ev: unknown) => void }) {
        queueMicrotask(() => {
          for (const ev of FINALIZED_SCRIPT) observer.next(ev)
        })
        return { unsubscribe: () => {} }
      },
    }))
    const apiStub = {
      tx: {
        TransactionStorage: {
          store: () => ({ getBareTx: async () => new Uint8Array([1]) }),
        },
      },
      query: {
        TransactionStorage: {
          Authorizations: {
            getValue: async (scope: unknown) => {
              opts.onQuery?.(scope)
              return opts.auth
            },
          },
        },
        System: {
          Number: {
            getValue: async () => opts.currentBlock ?? 100,
          },
        },
      },
    }
    // biome-ignore lint/suspicious/noExplicitAny: test stub
    const client = new AsyncBulletinClient(
      apiStub as any,
      undefined,
      submitAndWatch,
    )
    return { client, submitAndWatch }
  }

  it("queries Preimage scope (not Account)", async () => {
    const queried: unknown[] = []
    const { client } = await makePreimageClient({
      auth: { extent: { transactions: 1, bytes: 1024n }, expiration: 1_000_000 },
      onQuery: (s) => queried.push(s),
    })
    await client
      .upload([
        { data: new Uint8Array([1]) },
        { data: new Uint8Array([2]) },
      ])
      .ensureAuthorized()
      .asUnsigned()
      .send()
    expect(queried).toHaveLength(2)
    // biome-ignore lint/suspicious/noExplicitAny: test stub
    expect((queried[0] as any).type).toBe("Preimage")
  })

  it("throws INSUFFICIENT_AUTHORIZATION when preimage missing", async () => {
    const { client, submitAndWatch } = await makePreimageClient({ auth: undefined })
    await expect(
      client
        .upload([{ data: new Uint8Array([1]) }])
        .ensureAuthorized()
        .asUnsigned()
        .send(),
    ).rejects.toMatchObject({ code: ErrorCode.INSUFFICIENT_AUTHORIZATION })
    expect(submitAndWatch).not.toHaveBeenCalled()
  })

  it("throws INSUFFICIENT_AUTHORIZATION when expired", async () => {
    const { client, submitAndWatch } = await makePreimageClient({
      auth: { extent: { transactions: 1, bytes: 1024n }, expiration: 50 },
      currentBlock: 100,
    })
    await expect(
      client
        .upload([{ data: new Uint8Array([1]) }])
        .ensureAuthorized()
        .asUnsigned()
        .send(),
    ).rejects.toMatchObject({
      code: ErrorCode.INSUFFICIENT_AUTHORIZATION,
      message: expect.stringMatching(/expired/i),
    })
    expect(submitAndWatch).not.toHaveBeenCalled()
  })

  it("dedupes identical content hashes across items (single RPC)", async () => {
    let queryCount = 0
    const { client } = await makePreimageClient({
      auth: { extent: { transactions: 1, bytes: 1024n }, expiration: 1_000_000 },
      onQuery: () => queryCount++,
    })
    const sameData = new Uint8Array([1, 2, 3])
    await client
      .upload([{ data: sameData }, { data: sameData }, { data: sameData }])
      .ensureAuthorized()
      .asUnsigned()
      .send()
    expect(queryCount).toBe(1)
  })
})

describe("UploadFileBuilder.asUnsigned()", () => {
  it("rejects data > chunkingThreshold with UNSUPPORTED_OPERATION", async () => {
    const { client, submitAndWatch } = await makeClient(FINALIZED_SCRIPT)
    const big = new Uint8Array(3 * 1024 * 1024) // > 2 MiB default
    await expect(
      client.uploadFile(big).asUnsigned().send(),
    ).rejects.toMatchObject({ code: ErrorCode.UNSUPPORTED_OPERATION })
    expect(submitAndWatch).not.toHaveBeenCalled()
  })

  it("rejects when withChunkSize() forces chunking", async () => {
    const { client, submitAndWatch } = await makeClient(FINALIZED_SCRIPT)
    await expect(
      client
        .uploadFile(new Uint8Array(128))
        .withChunkSize(16)
        .asUnsigned()
        .send(),
    ).rejects.toMatchObject({ code: ErrorCode.UNSUPPORTED_OPERATION })
    expect(submitAndWatch).not.toHaveBeenCalled()
  })

  it("returns single cid when data fits in one tx", async () => {
    const { client, submitAndWatch } = await makeClient(FINALIZED_SCRIPT)
    const { cid } = await client
      .uploadFile(new Uint8Array([1, 2, 3]))
      .asUnsigned()
      .send()
    expect(cid).toBeDefined()
    expect(submitAndWatch).toHaveBeenCalledTimes(1)
  })
})
