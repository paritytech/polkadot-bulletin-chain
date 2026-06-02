// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Tests for the `asUnsigned()` builder path. Unsigned uploads now route
 * through the same `pipelineStore` machinery as signed ones (one shared
 * chainHead subscription + TBCH reconciler), with `signer === undefined`
 * selecting the unsigned branch in the pipeline. We mock `pipelineStore`
 * wholesale here and verify the dispatch + ensureAuthorized contract;
 * the actual reconciler semantics are covered by integration tests.
 */

import { beforeEach, describe, expect, it, vi } from "vitest"
import { blobFromBytes, blobFromItems } from "../../src/blob-source"
import * as pipelineModule from "../../src/pipeline"
import { ErrorCode, type UploadItem } from "../../src/types"

// Per-test apiStub is set by the helper below; createClient is mocked at
// file scope (vi.mock hoists). Each test's helper reassigns `currentApi`.
// biome-ignore lint/suspicious/noExplicitAny: dynamic apiStub
let currentApi: any = {}
vi.mock("polkadot-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("polkadot-api")>()
  return {
    ...actual,
    createClient: vi.fn(() => ({
      getTypedApi: () => currentApi,
      submitAndWatch: () => ({ subscribe: () => ({ unsubscribe() {} }) }),
      destroy: () => {},
    })),
  }
})

interface MockClientOpts {
  withSigner?: boolean
  withWsUrls?: boolean
  withQuery?: {
    auth?:
      | { extent: { transactions: number; bytes: bigint }; expiration: number }
      | undefined
    currentBlock?: number
    onQuery?: (scope: unknown) => void
  }
}

async function makeClient(opts: MockClientOpts = {}) {
  const { BulletinClient } = await import("../../src/client")
  const apiStub: Record<string, unknown> = {
    tx: {
      TransactionStorage: {
        store: vi.fn(() => ({
          getBareTx: async () => new Uint8Array([1]),
        })),
        store_with_cid_config: vi.fn(() => ({
          getBareTx: async () => new Uint8Array([1]),
        })),
      },
    },
  }
  if (opts.withQuery) {
    apiStub.query = {
      TransactionStorage: {
        Authorizations: {
          getValue: async (scope: unknown) => {
            opts.withQuery!.onQuery?.(scope)
            return opts.withQuery!.auth
          },
        },
      },
      System: {
        Number: {
          getValue: async () => opts.withQuery!.currentBlock ?? 100,
        },
      },
    }
  }
  const signer =
    opts.withSigner === false
      ? undefined
      : { publicKey: new Uint8Array(32), sign: async () => new Uint8Array(64) }
  // The file-scope vi.mock returns `currentApi` from getTypedApi; set it
  // to this test's stub.
  currentApi = apiStub
  return new BulletinClient({
    descriptor: {},
    providers:
      opts.withWsUrls === false
        ? undefined
        : // biome-ignore lint/suspicious/noExplicitAny: provider stub
          () => [{} as any],
    // biome-ignore lint/suspicious/noExplicitAny: signer stub
    uploadSigner: signer as any,
  } as unknown as ConstructorParameters<typeof BulletinClient>[0])
}

// Route items through the sole submission API. Flags select the builder
// variants (.asUnsigned() / .ensureAuthorized()).
async function submitItems(
  // biome-ignore lint/suspicious/noExplicitAny: BulletinClient under test
  client: any,
  items: UploadItem[],
  opts: { unsigned?: boolean; ensureAuth?: boolean } = {},
) {
  const builder = client.submit(
    await client.estimateUpload(items),
    blobFromItems(items),
  )
  if (opts.ensureAuth) builder.ensureAuthorized()
  if (opts.unsigned) builder.asUnsigned()
  return builder.send()
}

describe("submit().asUnsigned() — pipelineStore dispatch", () => {
  beforeEach(() => vi.restoreAllMocks())

  it("calls pipelineStore with signer=undefined when asUnsigned()", async () => {
    const spy = vi
      .spyOn(pipelineModule, "pipelineStore")
      // biome-ignore lint/suspicious/noExplicitAny: minimal mock surface
      .mockImplementation(async (_api, _signer, items: any) => ({
        cids: items.map(
          (_: unknown, i: number) =>
            ({
              toString: () => `cid:${i}`,
            }) as never,
        ),
      }))

    const client = await makeClient()
    const { cids } = await submitItems(
      client,
      [{ data: new Uint8Array([1]) }],
      { unsigned: true },
    )

    expect(spy).toHaveBeenCalledTimes(1)
    // Second arg = signer; for asUnsigned it must be undefined.
    expect(spy.mock.calls[0]![1]).toBeUndefined()
    expect(cids).toHaveLength(1)
  })

  it("calls pipelineStore with the actual signer when NOT asUnsigned()", async () => {
    const spy = vi
      .spyOn(pipelineModule, "pipelineStore")
      // biome-ignore lint/suspicious/noExplicitAny: minimal mock surface
      .mockImplementation(async (_api, _signer, items: any) => ({
        cids: items.map(() => ({ toString: () => "x" }) as never),
      }))
    const client = await makeClient()
    await submitItems(client, [{ data: new Uint8Array([1]) }])
    expect(spy).toHaveBeenCalled()
    expect(spy.mock.calls[0]![1]).not.toBeUndefined()
  })

  it("submits N items in a single pipelineStore call", async () => {
    const spy = vi
      .spyOn(pipelineModule, "pipelineStore")
      // biome-ignore lint/suspicious/noExplicitAny: minimal mock surface
      .mockImplementation(async (_api, _signer, items: any) => ({
        cids: items.map(
          (_: unknown, i: number) =>
            ({
              toString: () => `cid:${i}`,
            }) as never,
        ),
      }))
    const client = await makeClient()
    const items = Array.from({ length: 5 }, (_, i) => ({
      data: new Uint8Array([i]),
    }))
    const { cids } = await submitItems(client, items, { unsigned: true })

    expect(spy).toHaveBeenCalledTimes(1)
    expect(cids).toHaveLength(5)
    // The pipeline gets the full items array, not N separate calls.
    expect(spy.mock.calls[0]![2]).toHaveLength(5)
  })

  it("rejects empty data with EMPTY_DATA at estimate time (before pipeline)", async () => {
    const spy = vi.spyOn(pipelineModule, "pipelineStore")
    const client = await makeClient()
    await expect(
      client.estimateUpload([{ data: new Uint8Array(0) }]),
    ).rejects.toMatchObject({ code: ErrorCode.EMPTY_DATA })
    expect(spy).not.toHaveBeenCalled()
  })

  it("constructor rejects when `providers` is missing", async () => {
    // The self-contained constructor requires `providers` upfront; this
    // replaces the previous "rejects asUnsigned() without wsUrls" test.
    await expect(makeClient({ withWsUrls: false })).rejects.toMatchObject({
      code: ErrorCode.INVALID_CONFIG,
    })
  })

  it("works on a signer-less client (asUnsigned doesn't need a signer)", async () => {
    const spy = vi
      .spyOn(pipelineModule, "pipelineStore")
      // biome-ignore lint/suspicious/noExplicitAny: minimal mock surface
      .mockImplementation(async (_api, _signer, items: any) => ({
        cids: items.map(() => ({ toString: () => "x" }) as never),
      }))
    const client = await makeClient({ withSigner: false })
    const { cids } = await submitItems(
      client,
      [{ data: new Uint8Array([1]) }],
      { unsigned: true },
    )
    expect(cids).toHaveLength(1)
    expect(spy.mock.calls[0]![1]).toBeUndefined()
  })

  it("signed submit() on a signer-less client throws UNSUPPORTED_OPERATION", async () => {
    const spy = vi.spyOn(pipelineModule, "pipelineStore")
    const client = await makeClient({ withSigner: false })
    await expect(
      submitItems(client, [{ data: new Uint8Array([1]) }]),
    ).rejects.toMatchObject({ code: ErrorCode.UNSUPPORTED_OPERATION })
    expect(spy).not.toHaveBeenCalled()
  })
})

describe("submit().asUnsigned() — blob source", () => {
  beforeEach(() => vi.restoreAllMocks())

  it("stores a single-tx blob unsigned (returns its cid)", async () => {
    vi.spyOn(pipelineModule, "pipelineStore").mockImplementation(
      // biome-ignore lint/suspicious/noExplicitAny: minimal mock surface
      async (_api, _signer, items: any) => ({
        cids: items.map(() => ({ toString: () => "x" }) as never),
      }),
    )
    const client = await makeClient()
    const src = blobFromBytes(new Uint8Array([1, 2, 3]))
    const { cids } = await client
      .submit(await client.estimateUpload(src), src)
      .asUnsigned()
      .send()
    expect(cids[cids.length - 1]).toBeDefined()
  })
})

describe("ensureAuthorized() + asUnsigned()", () => {
  beforeEach(() => vi.restoreAllMocks())

  it("queries Preimage scope (not Account)", async () => {
    vi.spyOn(pipelineModule, "pipelineStore").mockImplementation(
      // biome-ignore lint/suspicious/noExplicitAny: minimal mock surface
      async (_api, _signer, items: any) => ({
        cids: items.map(() => ({ toString: () => "x" }) as never),
      }),
    )
    const queried: unknown[] = []
    const client = await makeClient({
      withQuery: {
        auth: {
          extent: { transactions: 1, bytes: 1024n },
          expiration: 1_000_000,
        },
        onQuery: (s) => queried.push(s),
      },
    })
    await submitItems(
      client,
      [{ data: new Uint8Array([1]) }, { data: new Uint8Array([2]) }],
      { unsigned: true, ensureAuth: true },
    )
    expect(queried).toHaveLength(2)
    // biome-ignore lint/suspicious/noExplicitAny: test stub
    expect((queried[0] as any).type).toBe("Preimage")
  })

  it("throws INSUFFICIENT_AUTHORIZATION when preimage missing", async () => {
    const spy = vi.spyOn(pipelineModule, "pipelineStore")
    const client = await makeClient({ withQuery: { auth: undefined } })
    await expect(
      submitItems(client, [{ data: new Uint8Array([1]) }], {
        unsigned: true,
        ensureAuth: true,
      }),
    ).rejects.toMatchObject({ code: ErrorCode.INSUFFICIENT_AUTHORIZATION })
    expect(spy).not.toHaveBeenCalled()
  })

  it("throws INSUFFICIENT_AUTHORIZATION when expired", async () => {
    const spy = vi.spyOn(pipelineModule, "pipelineStore")
    const client = await makeClient({
      withQuery: {
        auth: { extent: { transactions: 1, bytes: 1024n }, expiration: 50 },
        currentBlock: 100,
      },
    })
    await expect(
      submitItems(client, [{ data: new Uint8Array([1]) }], {
        unsigned: true,
        ensureAuth: true,
      }),
    ).rejects.toMatchObject({
      code: ErrorCode.INSUFFICIENT_AUTHORIZATION,
      message: expect.stringMatching(/expired/i),
    })
    expect(spy).not.toHaveBeenCalled()
  })
})

describe("Duplicate content guard", () => {
  beforeEach(() => vi.restoreAllMocks())

  it("rejects submit() with two items sharing the same content hash", async () => {
    const spy = vi.spyOn(pipelineModule, "pipelineStore")
    const client = await makeClient()
    const sameData = new Uint8Array([1, 2, 3])
    await expect(
      submitItems(client, [{ data: sameData }, { data: sameData }]),
    ).rejects.toMatchObject({ code: ErrorCode.INVALID_CONFIG })
    expect(spy).not.toHaveBeenCalled()
  })

  it("rejects .asUnsigned() with duplicate content too", async () => {
    const spy = vi.spyOn(pipelineModule, "pipelineStore")
    const client = await makeClient()
    const sameData = new Uint8Array([1, 2, 3])
    await expect(
      submitItems(client, [{ data: sameData }, { data: sameData }], {
        unsigned: true,
      }),
    ).rejects.toMatchObject({ code: ErrorCode.INVALID_CONFIG })
    expect(spy).not.toHaveBeenCalled()
  })

  it("allows identical raw bytes when codec/hashAlgo differ (different CID)", async () => {
    const spy = vi
      .spyOn(pipelineModule, "pipelineStore")
      // biome-ignore lint/suspicious/noExplicitAny: minimal mock surface
      .mockImplementation(async (_api, _signer, items: any) => ({
        cids: items.map(
          (_: unknown, i: number) => ({ toString: () => `cid:${i}` }) as never,
        ),
      }))
    const { CidCodec, HashAlgorithm } = await import("../../src/types")
    const client = await makeClient()
    const sameData = new Uint8Array([1, 2, 3])
    const { cids } = await submitItems(client, [
      {
        data: sameData,
        codec: CidCodec.Raw,
        hashAlgo: HashAlgorithm.Blake2b256,
      },
      {
        data: sameData,
        codec: CidCodec.Raw,
        hashAlgo: HashAlgorithm.Sha2_256,
      },
    ])
    expect(cids).toHaveLength(2)
    expect(spy).toHaveBeenCalledTimes(1)
  })
})
